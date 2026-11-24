//! Models downloaded from Hugging Face, its mirror hf-mirror.com, or ModelScope, which serve the
//! same files. A broken download is kept as `<file>.part` and picks up where it stopped the next
//! time; nothing gets the file's name before its SHA-256 is checked.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::models::{self, ModelFile, ModelSpec};

/// Tries of a file whose download fails for a reason that may pass: the network, a timeout, a
/// server error. Each picks up where the last stopped.
const ATTEMPTS: u32 = 3;
/// The wait before the second try; it doubles before each next one.
const BACKOFF: Duration = Duration::from_millis(500);

/// Where models are downloaded from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// huggingface.co, or the address `$HF_ENDPOINT` gives, as Hugging Face's own tools read it.
    HuggingFace,
    /// hf-mirror.com, which serves Hugging Face's files at the same paths.
    HfMirror,
    /// modelscope.cn. It has no Hugging Face revisions, so its newest files are asked for: a
    /// change there fails the SHA-256 check rather than installing other files.
    ModelScope,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::HuggingFace, Source::HfMirror, Source::ModelScope];

    /// Its name, as `--source` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Source::HuggingFace => "huggingface",
            Source::HfMirror => "hf-mirror",
            Source::ModelScope => "modelscope",
        }
    }

    pub fn parse(name: &str) -> Result<Source, String> {
        Source::ALL
            .into_iter()
            .find(|s| s.name() == name)
            .ok_or_else(|| {
                let names: Vec<_> = Source::ALL.iter().map(|s| s.name()).collect();
                format!("no source {name}; the sources are {}", names.join(", "))
            })
    }

    /// The address of a model's file.
    pub fn url(self, spec: &ModelSpec, file: &ModelFile) -> String {
        let (repository, path) = (&spec.repository, &file.path);
        match self {
            Source::HuggingFace => {
                let endpoint = std::env::var("HF_ENDPOINT")
                    .ok()
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| "https://huggingface.co".into());
                let endpoint = endpoint.trim_end_matches('/');
                format!("{endpoint}/{repository}/resolve/{}/{path}", spec.revision)
            }
            Source::HfMirror => {
                format!(
                    "https://hf-mirror.com/{repository}/resolve/{}/{path}",
                    spec.revision
                )
            }
            Source::ModelScope => {
                format!("https://modelscope.cn/models/{repository}/resolve/master/{path}")
            }
        }
    }
}

/// How far a download has come: `done` of `file`'s bytes, counting those of a broken download
/// before.
pub type Progress<'a> = &'a mut dyn FnMut(&ModelFile, u64);

/// Downloads the files of a model that are not installed under `root` from `source`, and returns
/// its folder.
pub fn install(
    root: &Path,
    spec: &ModelSpec,
    source: Source,
    progress: Progress,
) -> Result<PathBuf, String> {
    install_with(root, spec, &|file| source.url(spec, file), progress)
}

fn install_with(
    root: &Path,
    spec: &ModelSpec,
    url: &dyn Fn(&ModelFile) -> String,
    progress: Progress,
) -> Result<PathBuf, String> {
    let target = models::dir(root, spec);
    let _lock = models::lock(root, spec)?;
    let agent = agent();
    for file in &spec.files {
        let path = target.join(&file.path);
        if fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == file.bytes) {
            progress(file, file.bytes);
            continue;
        }
        let url = url(file);
        let part = models::part(&path);
        fetch(&agent, &url, &part, file, progress).map_err(|f| format!("{url}: {}", f.message))?;
        models::place(&part, &path, file).map_err(|e| format!("{url}: {e}"))?;
    }
    Ok(target)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(ureq::Proxy::try_from_env())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

/// Why a try failed.
struct Failure {
    /// Whether another try may succeed.
    passing: bool,
    message: String,
}

/// Downloads `file` into `part`, with more tries for failures that may pass.
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    file: &ModelFile,
    progress: Progress,
) -> Result<(), Failure> {
    let mut attempt = 1;
    loop {
        match try_fetch(agent, url, part, file, progress) {
            Err(failure) if failure.passing && attempt < ATTEMPTS => {
                std::thread::sleep(BACKOFF * (1 << (attempt - 1)));
                attempt += 1;
            }
            Err(Failure { passing, message }) if attempt > 1 => {
                let message = format!("{message} (after {attempt} tries)");
                return Err(Failure { passing, message });
            }
            result => return result,
        }
    }
}

/// One try: the rest of `part` after the bytes it holds, or all of it again where the server
/// cannot send a part.
fn try_fetch(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    file: &ModelFile,
    progress: Progress,
) -> Result<(), Failure> {
    let local = |message: String| Failure {
        passing: false,
        message,
    };
    let mut have = fs::metadata(part).map_or(0, |m| m.len());
    if have > file.bytes {
        fs::remove_file(part).map_err(|e| local(format!("{}: {e}", part.display())))?;
        have = 0;
    }
    if have == file.bytes {
        return Ok(());
    }
    // Byte ranges count the bytes as sent: a compressed answer's would not be the file's.
    let mut request = agent.get(url).header("Accept-Encoding", "identity");
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
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
    let start = response
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("bytes ")?
                .split('-')
                .next()?
                .parse::<u64>()
                .ok()
        });
    let mut out = match status {
        // The rest, after what is there.
        206 if have > 0 && start == Some(have) => File::options().append(true).open(part),
        // The whole file, from the start.
        200 => File::create(part),
        // A part the server cannot send, or one that does not start where asked: start over.
        206 | 416 => {
            let _ = fs::remove_file(part);
            return Err(Failure {
                passing: true,
                message: format!("HTTP {status} to a request for bytes {have} on"),
            });
        }
        _ => {
            return Err(Failure {
                passing: status == 429 || status >= 500,
                message: format!("HTTP {status}"),
            });
        }
    }
    .map_err(|e| local(format!("{}: {e}", part.display())))?;
    let mut done = if status == 200 { 0 } else { have };
    progress(file, done);
    let mut body = response.into_body().into_reader();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            // What arrived is kept for the next try.
            Err(e) => {
                return Err(Failure {
                    passing: true,
                    message: e.to_string(),
                });
            }
        };
        done += read as u64;
        if done > file.bytes {
            drop(out);
            let _ = fs::remove_file(part);
            return Err(local(format!("more than the file's {} bytes", file.bytes)));
        }
        out.write_all(&buffer[..read])
            .map_err(|e| local(format!("{}: {e}", part.display())))?;
        progress(file, done);
    }
    if done < file.bytes {
        return Err(Failure {
            passing: true,
            message: format!("the download stopped at {done} of {} bytes", file.bytes),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::models::tests::{Folder, spec};

    /// A server of `a.bin` and `b.bin` that answers byte ranges, and stops the first answer for
    /// `b.bin` after `cut` bytes. It records the `Range` header of every request.
    fn serve(cut: usize, ranges: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut cut = Some(cut);
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                let mut range = None;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_lowercase().strip_prefix("range: bytes=") {
                        range = value.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                }
                log.lock().unwrap().push(format!("{path} {range:?}"));
                let body: &[u8] = if path.ends_with("a.bin") {
                    b"first file"
                } else {
                    b"second"
                };
                let from = if ranges { range.unwrap_or(0) } else { 0 };
                let head = if ranges && range.is_some() {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\n",
                        body.len() - 1,
                        body.len()
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".to_string()
                };
                let rest = &body[from..];
                let sent = match path.ends_with("b.bin") {
                    true => cut.take().map_or(rest.len(), |c| c.min(rest.len())),
                    false => rest.len(),
                };
                let _ = write!(
                    stream,
                    "{head}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    rest.len()
                );
                let _ = stream.write_all(&rest[..sent]);
            }
        });
        (base, seen)
    }

    #[test]
    fn a_broken_download_picks_up_where_it_stopped() {
        let root = Folder::new("resume");
        let spec = spec();
        let (base, seen) = serve(2, true);
        let mut last = Vec::new();
        let url = |file: &ModelFile| format!("{base}/{}", file.path);
        let target = install_with(&root.0, &spec, &url, &mut |f, done| {
            last.push((f.path.clone(), done))
        })
        .unwrap();
        assert!(models::installed(&root.0, &spec));
        assert_eq!(fs::read(target.join("b.bin")).unwrap(), b"second");
        assert_eq!(
            *seen.lock().unwrap(),
            ["/a.bin None", "/b.bin None", "/b.bin Some(2)"]
        );
        assert_eq!(last.last().unwrap(), &("b.bin".to_string(), 6));
        assert!(!models::part(&target.join("b.bin")).exists());
    }

    #[test]
    fn a_server_without_ranges_sends_the_file_again() {
        let root = Folder::new("whole");
        let spec = spec();
        let (base, seen) = serve(3, false);
        let url = |file: &ModelFile| format!("{base}/{}", file.path);
        let target = install_with(&root.0, &spec, &url, &mut |_, _| {}).unwrap();
        assert_eq!(fs::read(target.join("b.bin")).unwrap(), b"second");
        assert_eq!(seen.lock().unwrap()[2], "/b.bin Some(3)");
    }

    #[test]
    fn a_file_with_other_bytes_is_refused() {
        let root = Folder::new("refused");
        let mut spec = spec();
        spec.files[1].sha256 = "0".repeat(64);
        let (base, _) = serve(usize::MAX, true);
        let url = |file: &ModelFile| format!("{base}/{}", file.path);
        let e = install_with(&root.0, &spec, &url, &mut |_, _| {}).unwrap_err();
        assert!(e.contains("/b.bin") && e.contains("SHA-256"), "{e}");
        let b = models::dir(&root.0, &spec).join("b.bin");
        assert!(!b.exists() && !models::part(&b).exists());
    }

    #[test]
    fn sources_address_the_pinned_revision() {
        let spec = spec();
        let file = &spec.files[0];
        assert_eq!(
            Source::HfMirror.url(&spec, file),
            "https://hf-mirror.com/test/model/resolve/0123abcd/a.bin"
        );
        assert_eq!(
            Source::ModelScope.url(&spec, file),
            "https://modelscope.cn/models/test/model/resolve/master/a.bin"
        );
        assert_eq!(Source::parse("modelscope").unwrap(), Source::ModelScope);
        assert!(Source::parse("nope").unwrap_err().contains("hf-mirror"));
    }
}
