//! Native remote-asset cache with streaming downloads, atomic publication, and bounded retries for
//! transient failures. URL keys identify locators; full SHA-256 digests identify downloaded blobs.
//! Revalidate with HTTP validators on every fetch, or redownload when validators are unavailable.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use fs2::FileExt;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use sha2::{Digest, Sha256};
use valle_engine::resource::ContentDigest;

/// Maximum download attempts for transient failures.
const MAX_ATTEMPTS: u32 = 3;
/// Per-request timeout in seconds.
const REQUEST_TIMEOUT_S: u64 = 120;
/// Truncated URL locator-key length; this is not a content identity or integrity check.
const URL_CACHE_KEY_HEX_LEN: usize = 16;
/// Base exponential retry delay in milliseconds.
const RETRY_BASE_MS: u64 = 250;
/// Maximum URL-derived jitter in milliseconds to spread concurrent retries.
const RETRY_JITTER_MS: u64 = 250;
/// Limit exponential backoff growth.
const MAX_BACKOFF_SHIFT: u32 = 4;
/// FNV-1a constants for deterministic jitter without a random-number dependency.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const LOCATORS_DIR: &str = "locators";
const BLOBS_DIR: &str = "blobs";
const PARTIALS_DIR: &str = "partials";

/// Non-security lookup key for one source URL. It must never be exposed as a content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlCacheKey(String);

impl UrlCacheKey {
    fn of_url(url: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let hex = hex::encode(hasher.finalize());
        Self(hex[..URL_CACHE_KEY_HEX_LEN].to_owned())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlCacheLocator {
    url: String,
    content_digest: ContentDigest,
    extension: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// Shared cache under `VALLE_CACHE_DIR/assets` or `~/.cache/valle/assets`. Align with preview and
/// web-runtime caches so projects reuse downloads. Fall back to `.valle/cache` when HOME is
/// unavailable.
pub fn default_cache_dir() -> PathBuf {
    if let Some(root) = std::env::var_os("VALLE_CACHE_DIR") {
        return PathBuf::from(root).join("assets");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home)
            .join(".cache")
            .join("valle")
            .join("assets");
    }
    PathBuf::from(".valle").join("cache")
}

/// Remote-asset cache with revalidated URL locators and content-addressed response blobs.
pub struct AssetCache {
    root: PathBuf,
    client: reqwest::blocking::Client,
}

impl AssetCache {
    /// Create the cache directory and a blocking HTTP client with timeouts.
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join(LOCATORS_DIR))
            .map_err(|e| anyhow!("create cache locators {}: {e}", root.display()))?;
        std::fs::create_dir_all(root.join(BLOBS_DIR))
            .map_err(|e| anyhow!("create cache dir {}: {e}", root.display()))?;
        std::fs::create_dir_all(root.join(PARTIALS_DIR))
            .map_err(|e| anyhow!("create cache partials {}: {e}", root.display()))?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_S))
            .build()?;
        Ok(Self { root, client })
    }

    /// Fetch a URL into a content-addressed blob, revalidating cached locators. The extension hint
    /// affects only the filename, not content identity.
    pub fn fetch(&self, url: &str, ext_hint: &str) -> Result<PathBuf> {
        let extension = cache_extension(ext_hint)?;
        let url_key = UrlCacheKey::of_url(url);
        let locator_path = self
            .root
            .join(LOCATORS_DIR)
            .join(format!("{}.json", url_key.as_str()));
        let locator_lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(
                self.root
                    .join(LOCATORS_DIR)
                    .join(format!("{}.lock", url_key.as_str())),
            )
            .map_err(|error| anyhow!("open URL cache lock: {error}"))?;
        FileExt::lock_exclusive(&locator_lock)
            .map_err(|error| anyhow!("lock URL cache locator: {error}"))?;
        let locator = read_locator(&locator_path, url);
        let mut attempt = 1u32;
        loop {
            match self.try_download(url, &extension, &locator_path, locator.as_ref()) {
                Ok(path) => return Ok(path),
                Err(e) => {
                    if !e.transient || attempt >= MAX_ATTEMPTS {
                        return Err(e
                            .source
                            .context(format!("fetch {url} failed after {attempt} attempts")));
                    }
                    std::thread::sleep(retry_backoff(attempt, url));
                    attempt += 1;
                }
            }
        }
    }

    /// Attempt one download and classify failures as transient or permanent. Atomic publication
    /// prevents partial cache entries.
    fn try_download(
        &self,
        url: &str,
        extension: &str,
        locator_path: &Path,
        cached: Option<&UrlCacheLocator>,
    ) -> std::result::Result<PathBuf, DownloadError> {
        let mut resp = self.send(url, cached)?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            if let Some(cached) = cached {
                let path = locator_blob_path(&self.root, cached);
                if blob_matches_locator(&path, cached) {
                    return Ok(path);
                }
            }
            // A locator without its CAS blob cannot be satisfied by 304. Retry once without
            // validators so the cache repairs itself from real response bytes.
            resp = self.send(url, None)?;
            if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
                return Err(DownloadError {
                    transient: false,
                    source: anyhow!("HTTP {url}: 304 without a usable cached blob"),
                });
            }
        }
        let etag = response_header(&resp, ETAG);
        let last_modified = response_header(&resp, LAST_MODIFIED);
        let mut resp = checked_response(url, resp)?;
        let mut part = new_download_part(&self.root).map_err(|e| DownloadError {
            transient: false, // Local file-creation errors are not retryable.
            source: anyhow!("create unique asset download: {e}"),
        })?;
        let part_path = part.path().to_path_buf();
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = resp.read(&mut buffer).map_err(|e| DownloadError {
                transient: true,
                source: anyhow!("stream {url} → {}: {e}", part_path.display()),
            })?;
            if read == 0 {
                break;
            }
            part.write_all(&buffer[..read]).map_err(|e| DownloadError {
                transient: false,
                source: anyhow!("write {}: {e}", part_path.display()),
            })?;
            hasher.update(&buffer[..read]);
        }
        part.flush().map_err(|e| DownloadError {
            transient: false,
            source: anyhow!("flush {}: {e}", part_path.display()),
        })?;
        let content_digest = ContentDigest::from_bytes(hasher.finalize().into());
        let path = self.root.join(BLOBS_DIR).join(content_digest.as_hex());
        publish_content_blob(part, &path, content_digest)?;
        let locator = UrlCacheLocator {
            url: url.to_owned(),
            content_digest,
            extension: extension.to_owned(),
            etag,
            last_modified,
        };
        write_locator(locator_path, &locator).map_err(|source| DownloadError {
            transient: false,
            source,
        })?;
        Ok(path)
    }

    fn send(
        &self,
        url: &str,
        cached: Option<&UrlCacheLocator>,
    ) -> std::result::Result<reqwest::blocking::Response, DownloadError> {
        let mut request = self.client.get(url);
        if let Some(cached) = cached {
            if let Some(etag) = &cached.etag {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = &cached.last_modified {
                request = request.header(IF_MODIFIED_SINCE, last_modified);
            }
        }
        request.send().map_err(|e| DownloadError {
            transient: !e.is_builder(),
            source: anyhow!("GET {url}: {e}"),
        })
    }
}

/// Download failure and retry classification.
#[derive(Debug)]
struct DownloadError {
    transient: bool,
    source: anyhow::Error,
}

/// Bounded exponential backoff with deterministic URL-derived jitter.
fn retry_backoff(attempt: u32, url: &str) -> Duration {
    let base_ms =
        RETRY_BASE_MS.saturating_mul(1 << attempt.min(MAX_BACKOFF_SHIFT).saturating_sub(1));
    let mut h = FNV_OFFSET;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    Duration::from_millis(base_ms + (h % RETRY_JITTER_MS))
}

fn checked_response(
    url: &str,
    response: reqwest::blocking::Response,
) -> std::result::Result<reqwest::blocking::Response, DownloadError> {
    let status = response.status();
    if status == reqwest::StatusCode::OK {
        return Ok(response);
    }
    Err(DownloadError {
        transient: status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        source: anyhow!("HTTP {url}: expected 200 OK, received {status}"),
    })
}

fn response_header(
    response: &reqwest::blocking::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn cache_extension(ext_hint: &str) -> Result<String> {
    let extension = ext_hint.trim_start_matches('.');
    if extension.is_empty()
        || extension.len() > 16
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(anyhow!("invalid asset cache extension '{ext_hint}'"));
    }
    Ok(extension.to_ascii_lowercase())
}

fn locator_blob_path(root: &Path, locator: &UrlCacheLocator) -> PathBuf {
    root.join(BLOBS_DIR).join(locator.content_digest.as_hex())
}

fn new_download_part(root: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    tempfile::Builder::new()
        .prefix(".asset-")
        .suffix(".part")
        .tempfile_in(root.join(PARTIALS_DIR))
}

fn publish_content_blob(
    part: tempfile::NamedTempFile,
    path: &Path,
    expected_digest: ContentDigest,
) -> std::result::Result<(), DownloadError> {
    if path.exists() {
        if file_content_digest(path) == Some(expected_digest) {
            return part.close().map_err(|error| DownloadError {
                transient: false,
                source: anyhow!("remove duplicate download: {error}"),
            });
        }
    }

    // `NamedTempFile::persist` atomically replaces the directory entry. The staging filename is
    // unique and never reused, so a crash cannot leave a second writable hard link to this blob.
    part.persist(path)
        .map(|_| ())
        .map_err(|error| DownloadError {
            transient: false,
            source: anyhow!(
                "atomically publish content-addressed blob {}: {error}",
                path.display()
            ),
        })
}

fn blob_matches_locator(path: &Path, locator: &UrlCacheLocator) -> bool {
    file_content_digest(path) == Some(locator.content_digest)
}

fn file_content_digest(path: &Path) -> Option<ContentDigest> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn read_locator(path: &Path, expected_url: &str) -> Option<UrlCacheLocator> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let object = value.as_object()?;
    let url = object.get("url")?.as_str()?;
    let content_digest = ContentDigest::parse(object.get("contentDigest")?.as_str()?).ok()?;
    let extension = object.get("extension")?.as_str()?;
    if url != expected_url || cache_extension(extension).ok()?.as_str() != extension {
        return None;
    }
    let optional_string = |key: &str| match object.get(key) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(value)) => Some(Some(value.clone())),
        _ => None,
    };
    let etag = optional_string("etag")?;
    let last_modified = optional_string("lastModified")?;
    if etag
        .iter()
        .chain(last_modified.iter())
        .any(|value| reqwest::header::HeaderValue::from_bytes(value.as_bytes()).is_err())
    {
        return None;
    }
    Some(UrlCacheLocator {
        url: url.to_owned(),
        content_digest,
        extension: extension.to_owned(),
        etag,
        last_modified,
    })
}

fn write_locator(path: &Path, locator: &UrlCacheLocator) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("URL cache locator has no parent: {}", path.display()))?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "url": locator.url,
        "contentDigest": locator.content_digest,
        "extension": locator.extension,
        "etag": locator.etag,
        "lastModified": locator.last_modified,
    }))?;
    let mut part = tempfile::Builder::new()
        .prefix(".locator-")
        .suffix(".part")
        .tempfile_in(parent)
        .map_err(|error| {
            anyhow!(
                "create unique URL cache locator in {}: {error}",
                parent.display()
            )
        })?;
    part.write_all(&bytes)
        .map_err(|error| anyhow!("write URL cache locator {}: {error}", part.path().display()))?;
    part.as_file()
        .sync_all()
        .map_err(|error| anyhow!("sync URL cache locator {}: {error}", part.path().display()))?;
    part.persist(path)
        .map_err(|error| anyhow!("publish URL cache locator {}: {error}", path.display()))?;
    sync_locator_directory(parent)
}

#[cfg(unix)]
fn sync_locator_directory(path: &Path) -> Result<()> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            anyhow!(
                "sync URL cache locator directory {}: {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
fn sync_locator_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn url_cache_key_is_stable_but_is_not_a_content_digest() {
        let a = UrlCacheKey::of_url("https://e.com/a.mp4");
        assert_eq!(a, UrlCacheKey::of_url("https://e.com/a.mp4"));
        assert_ne!(a, UrlCacheKey::of_url("https://e.com/b.mp4"));
        assert_eq!(a.as_str().len(), URL_CACHE_KEY_HEX_LEN);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(cache_extension(".MP4").unwrap(), "mp4");
    }

    #[test]
    fn same_url_with_new_bytes_publishes_a_new_content_digest_blob() {
        let (url, server) = serve_bodies([b"first bytes".as_slice(), b"second bytes".as_slice()]);
        let root = test_root("stale-url");
        let cache = AssetCache::new(root.clone()).unwrap();
        let first = cache.fetch(&url, "mp4").unwrap();
        let second = cache.fetch(&url, "mp4").unwrap();

        assert_ne!(
            first, second,
            "response bytes, not URL, must address the blob"
        );
        assert_eq!(std::fs::read(first).unwrap(), b"first bytes");
        assert_eq!(std::fs::read(&second).unwrap(), b"second bytes");
        let digest = second.file_name().unwrap().to_str().unwrap();
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains('.'));

        server.join().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn matching_http_validator_reuses_the_verified_content_blob() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_request(&mut first);
            assert!(!request.contains("If-None-Match"));
            first
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nstable",
                )
                .unwrap();

            let (mut second, _) = listener.accept().unwrap();
            let request = read_request(&mut second);
            assert!(
                request.contains("if-none-match: \"v1\"")
                    || request.contains("If-None-Match: \"v1\"")
            );
            second
                .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let root = test_root("validator");
        let cache = AssetCache::new(root.clone()).unwrap();
        let url = format!("http://{address}/asset");

        let first = cache.fetch(&url, "bin").unwrap();
        let second = cache.fetch(&url, "bin").unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(second).unwrap(), b"stable");

        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_bytes_have_one_blob_across_extension_hints() {
        let (url, server) = serve_bodies([b"same bytes".as_slice(), b"same bytes".as_slice()]);
        let root = test_root("extension-dedup");
        let cache = AssetCache::new(root.clone()).unwrap();

        let video = cache.fetch(&url, "mp4").unwrap();
        let generic = cache.fetch(&url, "bin").unwrap();
        assert_eq!(video, generic);
        assert_eq!(std::fs::read_dir(root.join(BLOBS_DIR)).unwrap().count(), 1);

        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_fetches_of_one_url_do_not_share_partial_files() {
        let (url, server) = serve_bodies([b"stable".as_slice(), b"stable".as_slice()]);
        let root = test_root("concurrent-url");
        let cache = Arc::new(AssetCache::new(root.clone()).unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                let url = url.clone();
                thread::spawn(move || {
                    barrier.wait();
                    cache.fetch(&url, "bin").unwrap()
                })
            })
            .collect::<Vec<_>>();
        let paths = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(paths[0], paths[1]);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"stable");
        assert_eq!(std::fs::read_dir(root.join(BLOBS_DIR)).unwrap().count(), 1);

        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crash_orphan_is_never_reopened_or_linked_to_a_published_blob() {
        let root = test_root("unique-partials");
        std::fs::create_dir_all(root.join(PARTIALS_DIR)).unwrap();
        std::fs::create_dir_all(root.join(BLOBS_DIR)).unwrap();

        let mut orphan = new_download_part(&root).unwrap();
        orphan.write_all(b"crash orphan").unwrap();
        let (_, orphan_path) = orphan.keep().unwrap();

        let mut current = new_download_part(&root).unwrap();
        current.write_all(b"published bytes").unwrap();
        assert_ne!(current.path(), orphan_path);
        let digest = ContentDigest::of_bytes(b"published bytes");
        let blob = root.join(BLOBS_DIR).join(digest.as_hex());
        publish_content_blob(current, &blob, digest).unwrap();

        assert_eq!(std::fs::read(&orphan_path).unwrap(), b"crash orphan");
        assert_eq!(std::fs::read(&blob).unwrap(), b"published bytes");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn locator_publish_never_follows_preexisting_or_destination_symlinks() {
        use std::os::unix::fs::symlink;

        let root = test_root("locator-symlink");
        let locators = root.join(LOCATORS_DIR);
        std::fs::create_dir_all(&locators).unwrap();
        let outside = root.join("outside-target");
        std::fs::write(&outside, b"must survive").unwrap();
        let locator_path = locators.join("0123456789abcdef.json");
        symlink(&outside, &locator_path).unwrap();
        symlink(&outside, locator_path.with_extension("json.part")).unwrap();

        write_locator(
            &locator_path,
            &UrlCacheLocator {
                url: "https://example.invalid/asset".into(),
                content_digest: ContentDigest::from_hex(&"a".repeat(64)).unwrap(),
                extension: "bin".into(),
                etag: None,
                last_modified: None,
            },
        )
        .unwrap();

        assert_eq!(std::fs::read(&outside).unwrap(), b"must survive");
        assert!(
            !std::fs::symlink_metadata(&locator_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            read_locator(&locator_path, "https://example.invalid/asset")
                .unwrap()
                .content_digest,
            ContentDigest::from_hex(&"a".repeat(64)).unwrap()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_blob_is_atomically_replaced_after_validator_refetch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            assert!(!read_request(&mut first).contains("If-None-Match"));
            first
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nstable",
                )
                .unwrap();

            let (mut second, _) = listener.accept().unwrap();
            let request = read_request(&mut second);
            assert!(
                request.contains("if-none-match: \"v1\"")
                    || request.contains("If-None-Match: \"v1\"")
            );
            second
                .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                .unwrap();

            let (mut repair, _) = listener.accept().unwrap();
            assert!(!read_request(&mut repair).contains("If-None-Match"));
            repair
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nstable",
                )
                .unwrap();
        });
        let root = test_root("repair-corrupt");
        let cache = AssetCache::new(root.clone()).unwrap();
        let url = format!("http://{address}/asset");

        let blob = cache.fetch(&url, "bin").unwrap();
        std::fs::write(&blob, b"broken").unwrap();
        let repaired = cache.fetch(&url, "bin").unwrap();
        assert_eq!(repaired, blob);
        assert_eq!(std::fs::read(repaired).unwrap(), b"stable");

        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_empty_and_unresolved_redirect_responses_never_enter_cas() {
        for status in ["204 No Content", "206 Partial Content", "302 Found"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let status = status.to_owned();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                read_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            });
            let root = test_root("reject-non-200");
            let cache = AssetCache::new(root.clone()).unwrap();
            let error = cache
                .fetch(&format!("http://{address}/asset"), "bin")
                .expect_err("only a complete 200 response may enter CAS");
            assert!(
                format!("{error:#}").contains("expected 200 OK"),
                "{error:#}"
            );
            assert_eq!(std::fs::read_dir(root.join(BLOBS_DIR)).unwrap().count(), 0);

            server.join().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    fn serve_bodies<const N: usize>(
        bodies: [&'static [u8]; N],
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                read_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        (format!("http://{address}/asset"), server)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    fn test_root(case: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "valle-render-assets-{case}-{}-{nonce}",
            std::process::id()
        ))
    }
}
