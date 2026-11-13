//! Files from the web, downloaded into the global cache.
//!
//! A file is kept as `files/<SHA-256 of its content>/<its name>`: the folder finds it, the name
//! tells a person what it is. A URL's file takes its name from the URL's last segment. What a URL
//! served last is recorded under `urls/`, by the SHA-256 of the URL, with the
//! response's validators: each fetch asks the server whether that is still current. A pinned fetch
//! names the content's SHA-256 instead, and never asks again once the file is there.
//!
//! Downloads go to `partial/` first and move into place whole, so a broken one leaves nothing
//! behind; a lock file keeps two processes from downloading the same thing at once.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

/// Tries of a download that fails for a reason that may pass: the network, a timeout, a server
/// error.
const ATTEMPTS: u32 = 3;
/// The wait before the second try; it doubles before each next one.
const BACKOFF: Duration = Duration::from_millis(250);

/// Downloads into a cache directory.
pub struct UrlCache {
    root: PathBuf,
    agent: ureq::Agent,
}

/// What a URL served last.
#[derive(Serialize, Deserialize)]
struct Record {
    url: String,
    sha256: String,
    name: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// One answer from the server.
enum Answer {
    /// The cached copy is still current.
    NotModified,
    /// A downloaded file, still in `partial/`.
    File {
        part: PathBuf,
        sha256: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// Why a try failed.
struct Failure {
    /// Whether another try may succeed.
    passing: bool,
    message: String,
}

impl UrlCache {
    /// The global cache.
    pub fn global() -> Result<Self, String> {
        Ok(Self::new(global_dir()?))
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(ureq::Proxy::try_from_env())
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build();
        Self {
            root: root.into(),
            agent: config.into(),
        }
    }

    /// The file `url` serves now: downloaded unless the cached copy is still current. When the
    /// server cannot be reached, a copy downloaded before is used.
    pub fn fetch(&self, url: &str) -> Result<PathBuf, String> {
        web(url)?;
        let name = name(url);
        let urls = self.folder("urls")?;
        let key = hex(&sha256(url.as_bytes()));
        let _lock = lock(&urls.join(format!("{key}.lock")))?;
        let record_path = urls.join(format!("{key}.json"));
        let record = fs::read(&record_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
            .filter(|record| {
                record.url == url && self.file(&record.sha256, &record.name).is_file()
            });
        // The URL names the content, as a commit or a version hash does: the copy can never be
        // out of date, so it is not asked about again.
        if fixed(url)
            && let Some(record) = &record
        {
            return Ok(self.file(&record.sha256, &record.name));
        }
        match self.download(url, record.as_ref()) {
            Ok(Answer::NotModified) => {
                let record = record.expect("asked about it");
                Ok(self.file(&record.sha256, &record.name))
            }
            Ok(Answer::File {
                part,
                sha256,
                etag,
                last_modified,
            }) => {
                let path = self.publish(&part, &sha256, &name)?;
                let record = Record {
                    url: url.to_string(),
                    sha256,
                    name,
                    etag,
                    last_modified,
                };
                let json = serde_json::to_vec(&record).expect("a record is JSON");
                let part = self.part()?;
                fs::write(&part, json).map_err(|e| format!("{}: {e}", part.display()))?;
                fs::rename(&part, &record_path)
                    .map_err(|e| format!("{}: {e}", record_path.display()))?;
                Ok(path)
            }
            Err(failure) => match record {
                Some(record) if failure.passing => Ok(self.file(&record.sha256, &record.name)),
                _ => Err(format!("{url}: {}", failure.message)),
            },
        }
    }

    /// The file with SHA-256 `sha256` that `url` serves, kept as `name`: downloaded once and
    /// checked, then never asked for again.
    pub fn fetch_pinned(&self, url: &str, sha256: &str, name: &str) -> Result<PathBuf, String> {
        let path = self.file(sha256, name);
        if path.is_file() {
            return Ok(path);
        }
        web(url)?;
        let files = self.folder("files")?;
        let _lock = lock(&files.join(format!("{sha256}.lock")))?;
        if path.is_file() {
            return Ok(path);
        }
        match self.download(url, None) {
            Ok(Answer::File {
                part, sha256: got, ..
            }) if got == sha256 => self.publish(&part, sha256, name),
            Ok(Answer::File {
                part, sha256: got, ..
            }) => {
                let _ = fs::remove_file(part);
                Err(format!("{url}: the file's SHA-256 is {got}, not {sha256}"))
            }
            Ok(Answer::NotModified) => Err(format!("{url}: HTTP 304 to a request without one")),
            Err(failure) => Err(format!("{url}: {}", failure.message)),
        }
    }

    /// Where the file with SHA-256 `sha256` is kept as `name`.
    fn file(&self, sha256: &str, name: &str) -> PathBuf {
        self.root.join("files").join(sha256).join(name)
    }

    /// Downloads `url`, asking whether `cached` is still current, with more tries for failures
    /// that may pass.
    fn download(&self, url: &str, cached: Option<&Record>) -> Result<Answer, Failure> {
        let mut attempt = 1;
        loop {
            match self.try_download(url, cached) {
                Err(failure) if failure.passing && attempt < ATTEMPTS => {
                    std::thread::sleep(BACKOFF * (1 << (attempt - 1)));
                    attempt += 1;
                }
                Err(Failure { passing, message }) if attempt > 1 => {
                    let message = format!("{message} (after {attempt} tries)");
                    return Err(Failure { passing, message });
                }
                answer => return answer,
            }
        }
    }

    fn try_download(&self, url: &str, cached: Option<&Record>) -> Result<Answer, Failure> {
        let mut request = self.agent.get(url);
        if let Some(cached) = cached {
            if let Some(etag) = &cached.etag {
                request = request.header("If-None-Match", etag);
            }
            if let Some(last_modified) = &cached.last_modified {
                request = request.header("If-Modified-Since", last_modified);
            }
        }
        let response = request.call().map_err(|e| Failure {
            passing: matches!(
                e,
                ureq::Error::Io(_)
                    | ureq::Error::Timeout(_)
                    | ureq::Error::HostNotFound
                    | ureq::Error::ConnectionFailed
                    | ureq::Error::Protocol(_)
                    | ureq::Error::BodyStalled
            ),
            message: e.to_string(),
        })?;
        let status = response.status().as_u16();
        if status == 304 && cached.is_some() {
            return Ok(Answer::NotModified);
        }
        if status != 200 {
            return Err(Failure {
                passing: status == 429 || status >= 500,
                message: format!("HTTP {status}"),
            });
        }
        let header = |name: &str| {
            let value = response.headers().get(name)?.to_str().ok()?;
            Some(value.to_string())
        };
        let (etag, last_modified) = (header("ETag"), header("Last-Modified"));
        let local = |message: String| Failure {
            passing: false,
            message,
        };
        let part = self.part().map_err(local)?;
        let mut file =
            File::create(&part).map_err(|e| local(format!("{}: {e}", part.display())))?;
        let mut body = response.into_body().into_reader();
        let mut digest = Context::new(&SHA256);
        let mut buffer = vec![0; 1 << 16];
        loop {
            let read = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    return Err(Failure {
                        passing: true,
                        message: e.to_string(),
                    });
                }
            };
            digest.update(&buffer[..read]);
            if let Err(e) = file.write_all(&buffer[..read]) {
                let _ = fs::remove_file(&part);
                return Err(local(format!("{}: {e}", part.display())));
            }
        }
        Ok(Answer::File {
            part,
            sha256: hex(digest.finish().as_ref()),
            etag,
            last_modified,
        })
    }

    /// Moves a downloaded file into place.
    fn publish(&self, part: &Path, sha256: &str, name: &str) -> Result<PathBuf, String> {
        let path = self.file(sha256, name);
        self.folder(&format!("files/{sha256}"))?;
        fs::rename(part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// A new file name in `partial/`, unique across threads and processes.
    fn part(&self) -> Result<PathBuf, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("{}-{next}", std::process::id());
        Ok(self.folder("partial")?.join(name))
    }

    fn folder(&self, name: &str) -> Result<PathBuf, String> {
        let folder = self.root.join(name);
        fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        Ok(folder)
    }
}

/// Checks that `url` is an HTTP(S) address.
fn web(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(())
    } else {
        Err(format!("{url}: not an HTTP or HTTPS address"))
    }
}

/// Whether `url` names the content it serves: a segment of it is a git commit or another content
/// hash, 40 or 64 hex digits, so what it serves cannot change under the same URL. A URL such as
/// jsDelivr's `gh/google/fonts@<commit>/…` is one.
fn fixed(url: &str) -> bool {
    let content = url.split(['?', '#']).next().unwrap_or_default();
    content.split(['/', '@']).any(|segment| {
        matches!(segment.len(), 40 | 64) && segment.chars().all(|c| c.is_ascii_hexdigit())
    })
}

/// The name a file from `url` is kept as: the URL's last segment, or `file` when that is empty or
/// has a character other than a letter, a digit, `.`, `-` or `_`.
fn name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    // After the host.
    let path = path
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('/'));
    let last = path.map_or("", |(_, path)| path.rsplit('/').next().unwrap_or_default());
    let plain = last
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if plain && !last.trim_matches('.').is_empty() {
        last.to_string()
    } else {
        "file".to_string()
    }
}

/// Holds `path` locked until the file is dropped.
/// The global cache's folder: `$VALLE_CACHE_DIR`, or `~/.valle/cache`.
pub fn global_dir() -> Result<PathBuf, String> {
    Ok(match std::env::var_os("VALLE_CACHE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir()
            .ok_or("no home directory to keep the cache in; set VALLE_CACHE_DIR")?
            .join(".valle")
            .join("cache"),
    })
}

fn lock(path: &Path) -> Result<File, String> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .and_then(|file| file.lock().map(|()| file));
    file.map_err(|e| format!("{}: {e}", path.display()))
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    ring::digest::digest(&SHA256, bytes).as_ref().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    use super::*;

    /// A server on 127.0.0.1 that answers each connection with the next reply, then stops listening.
    struct Server {
        url: String,
        /// The head of each request, lowercased.
        requests: Arc<Mutex<Vec<String>>>,
        thread: Option<JoinHandle<()>>,
    }

    impl Server {
        fn new(replies: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/font.ttf", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let thread = std::thread::spawn(move || {
                for reply in replies {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut head = String::new();
                    while reader.read_line(&mut head).unwrap() > 2 {}
                    seen.lock().unwrap().push(head.to_lowercase());
                    stream.write_all(&reply).unwrap();
                }
            });
            Self {
                url,
                requests,
                thread: Some(thread),
            }
        }

        /// The requests, once every reply is sent.
        fn finish(mut self) -> Vec<String> {
            self.thread.take().unwrap().join().unwrap();
            self.requests.lock().unwrap().clone()
        }
    }

    fn reply(status: u16, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
        let mut reply = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
        for (name, value) in headers {
            reply += &format!("{name}: {value}\r\n");
        }
        reply += &format!("Content-Length: {}\r\n\r\n{body}", body.len());
        reply.into_bytes()
    }

    /// An empty cache directory of its own for `test`.
    fn cache(test: &str) -> (UrlCache, PathBuf) {
        let root = std::env::temp_dir().join(format!("valle-urls-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (UrlCache::new(&root), root)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn files_are_downloaded_once_and_revalidated() {
        let (cache, root) = cache("revalidated");
        let server = Server::new(vec![
            reply(200, &[("ETag", "\"a\"")], "one"),
            reply(304, &[], ""),
        ]);
        let first = cache.fetch(&server.url).unwrap();
        assert_eq!(read(&first), "one");
        let one = hex(&sha256(b"one"));
        assert_eq!(first, root.join("files").join(one).join("font.ttf"));
        assert_eq!(cache.fetch(&server.url).unwrap(), first);
        let url = server.url.clone();
        let requests = server.finish();
        assert!(!requests[0].contains("if-none-match"), "{}", requests[0]);
        assert!(
            requests[1].contains("if-none-match: \"a\""),
            "{}",
            requests[1]
        );
        // The server is gone: the copy from before.
        assert_eq!(cache.fetch(&url).unwrap(), first);
        assert_eq!(fs::read_dir(root.join("partial")).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_changed_file_is_kept_apart() {
        let (cache, root) = cache("changed");
        let server = Server::new(vec![
            reply(
                200,
                &[("Last-Modified", "Tue, 03 Nov 2026 08:00:00 GMT")],
                "one",
            ),
            reply(200, &[], "two"),
        ]);
        let first = cache.fetch(&server.url).unwrap();
        let second = cache.fetch(&server.url).unwrap();
        assert_eq!([read(&first), read(&second)], ["one", "two"]);
        let requests = server.finish();
        assert!(requests[1].contains("if-modified-since: tue, 03 nov 2026 08:00:00 gmt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn server_errors_are_tried_again_but_missing_files_are_not() {
        let (cache, root) = cache("errors");
        let server = Server::new(vec![reply(503, &[], ""), reply(200, &[], "ok")]);
        assert_eq!(read(&cache.fetch(&server.url).unwrap()), "ok");
        assert_eq!(server.finish().len(), 2);

        let server = Server::new(vec![reply(404, &[], "")]);
        let e = cache.fetch(&server.url).unwrap_err();
        assert_eq!(e, format!("{}: HTTP 404", server.url));
        assert_eq!(server.finish().len(), 1);

        let server = Server::new(vec![reply(500, &[], ""); 3]);
        let e = cache.fetch(&server.url).unwrap_err();
        assert_eq!(e, format!("{}: HTTP 500 (after 3 tries)", server.url));
        server.finish();

        let e = cache.fetch("file:///etc/hosts").unwrap_err();
        assert_eq!(e, "file:///etc/hosts: not an HTTP or HTTPS address");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pinned_files_are_checked_and_never_asked_for_again() {
        let (cache, root) = cache("pinned");
        let font = hex(&sha256(b"font"));
        let server = Server::new(vec![reply(200, &[], "other"), reply(200, &[], "font")]);
        let e = cache
            .fetch_pinned(&server.url, &font, "Font.ttf")
            .unwrap_err();
        let other = hex(&sha256(b"other"));
        assert_eq!(
            e,
            format!("{}: the file's SHA-256 is {other}, not {font}", server.url)
        );
        assert!(!root.join("files").join(&other).exists());
        let path = cache.fetch_pinned(&server.url, &font, "Font.ttf").unwrap();
        assert_eq!(read(&path), "font");
        assert_eq!(path, root.join("files").join(&font).join("Font.ttf"));
        let url = server.url.clone();
        server.finish();
        // The server is gone, and is not needed.
        assert_eq!(cache.fetch_pinned(&url, &font, "Font.ttf").unwrap(), path);
        assert_eq!(fs::read_dir(root.join("partial")).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_url_that_names_its_content_is_not_asked_about_again() {
        let (cache, root) = cache("fixed");
        let commit = "5bb28733cec313b63fbd415b574b5ae35c8aa8b8";
        let server = Server::new(vec![reply(200, &[("ETag", "\"a\"")], "font")]);
        let url = server
            .url
            .replace("/font.ttf", &format!("/@{commit}/font.ttf"));
        let first = cache.fetch(&url).unwrap();
        assert_eq!(read(&first), "font");
        // A second fetch asks nothing: the server has no reply to give and the thread would
        // panic if it were asked.
        assert_eq!(cache.fetch(&url).unwrap(), first);
        assert_eq!(server.finish().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_a_whole_content_hash_fixes_a_url() {
        let commit = "5bb28733cec313b63fbd415b574b5ae35c8aa8b8";
        let sha = "498efd461f6ddfcb7a111bf9a565709d2085d48201d501ead960d93e84ffbb88";
        assert!(fixed(&format!("https://x/@{commit}/a.ttf")));
        assert!(fixed(&format!("https://x/{sha}/a.ttf")));
        assert!(fixed(&format!("https://x/{sha}/a.ttf?v=1#top")));
        assert!(!fixed("https://x/@main/a.ttf"));
        assert!(!fixed(&format!("https://x/{}/a.ttf", &commit[..39])));
        assert!(!fixed(&format!("https://x/{}/a.ttf", "g".repeat(40))));
        assert!(!fixed("https://x/a.ttf"));
    }

    #[test]
    fn files_are_named_by_the_last_segment_of_their_url() {
        assert_eq!(
            name("https://example.com/fonts/Inter.woff2?v=4#x"),
            "Inter.woff2"
        );
        assert_eq!(name("https://example.com/a/b-c_d.1.ttf"), "b-c_d.1.ttf");
        assert_eq!(name("https://example.com/"), "file");
        assert_eq!(name("https://example.com"), "file");
        assert_eq!(name("https://example.com/.."), "file");
        assert_eq!(name("https://example.com/f%C3%BCr.ttf"), "file");
    }
}
