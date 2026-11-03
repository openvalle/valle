//! Files from the web, downloaded into the global cache.
//!
//! A file is kept under `files/` by the SHA-256 of its content, so one file at two URLs is kept
//! once. What a URL served last is recorded under `urls/`, by the SHA-256 of the URL, with the
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
    /// The global cache: `$VALLE_CACHE_DIR`, or `~/.valle/cache`.
    pub fn global() -> Result<Self, String> {
        let root = match std::env::var_os("VALLE_CACHE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::home_dir()
                .ok_or("no home directory to keep the cache in; set VALLE_CACHE_DIR")?
                .join(".valle")
                .join("cache"),
        };
        Ok(Self::new(root))
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
        let urls = self.folder("urls")?;
        let key = hex(&sha256(url.as_bytes()));
        let _lock = lock(&urls.join(format!("{key}.lock")))?;
        let record_path = urls.join(format!("{key}.json"));
        let record = fs::read(&record_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
            .filter(|record| record.url == url && self.file(&record.sha256).is_file());
        match self.download(url, record.as_ref()) {
            Ok(Answer::NotModified) => Ok(self.file(&record.expect("asked about it").sha256)),
            Ok(Answer::File {
                part,
                sha256,
                etag,
                last_modified,
            }) => {
                let path = self.publish(&part, &sha256)?;
                let record = Record {
                    url: url.to_string(),
                    sha256,
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
                Some(record) if failure.passing => Ok(self.file(&record.sha256)),
                _ => Err(format!("{url}: {}", failure.message)),
            },
        }
    }

    /// The file with SHA-256 `sha256` that `url` serves: downloaded once and checked, then never
    /// asked for again.
    pub fn fetch_pinned(&self, url: &str, sha256: &str) -> Result<PathBuf, String> {
        let path = self.file(sha256);
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
            }) if got == sha256 => self.publish(&part, sha256),
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

    /// Where the file with SHA-256 `sha256` is kept.
    fn file(&self, sha256: &str) -> PathBuf {
        self.root.join("files").join(sha256)
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
    fn publish(&self, part: &Path, sha256: &str) -> Result<PathBuf, String> {
        let path = self.file(sha256);
        self.folder("files")?;
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

/// Holds `path` locked until the file is dropped.
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
mod tests;
