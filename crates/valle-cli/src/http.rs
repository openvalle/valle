//! The little of HTTP/1.1 Studio's local service speaks: one request a connection, read whole,
//! and its response written from bytes, streamed from a file, or sent as server-sent events.

use std::fs::File;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The most a request's headers and its body may take.
const HEAD: usize = 64 << 10;
const BODY: usize = 256 << 20;

pub struct Request {
    pub method: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    query: Vec<(String, String)>,
    /// Header names in lower case.
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The next request on `stream`.
    pub fn read(stream: &mut impl BufRead) -> Result<Request, String> {
        let mut head = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = (stream.read_line(&mut line)).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the connection closed".into());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            head.push(line.trim_end().to_string());
            if head.iter().map(String::len).sum::<usize>() > HEAD {
                return Err("headers too large".into());
            }
        }
        let first = head.first().ok_or("no request line")?;
        let mut parts = first.split(' ');
        let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
            return Err(format!("not a request: {first}"));
        };
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let path = decode(path).ok_or_else(|| format!("not a path: {path}"))?;
        let query = (query.split('&'))
            .filter(|p| !p.is_empty())
            .filter_map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                Some((decode(k)?, decode(&v.replace('+', " "))?))
            })
            .collect();
        let headers: Vec<(String, String)> = (head[1..].iter())
            .filter_map(|h| h.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let mut request = Request {
            method: method.to_string(),
            path,
            query,
            headers,
            body: Vec::new(),
        };
        let length = match request.header("content-length") {
            Some(n) => n.parse::<usize>().map_err(|_| format!("bad length {n}"))?,
            None => 0,
        };
        if length > BODY {
            return Err(format!("a body of {length} bytes is too large"));
        }
        request.body = vec![0; length];
        (stream.read_exact(&mut request.body)).map_err(|e| e.to_string())?;
        Ok(request)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        (self.headers.iter()).find_map(|(k, v)| (k == name).then_some(v.as_str()))
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        (self.query.iter()).find_map(|(k, v)| (k == name).then_some(v.as_str()))
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    file: Option<(File, u64)>,
}

impl Response {
    pub fn new(status: u16, kind: &str, body: impl Into<Vec<u8>>) -> Response {
        Response {
            status,
            headers: vec![("content-type".into(), kind.into())],
            body: body.into(),
            file: None,
        }
    }

    /// Keeps an open file, copied to the connection in bounded chunks when written.
    pub fn file(path: &Path, kind: &str) -> io::Result<Response> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::other("not a file"));
        }
        let mut response = Self::new(200, kind, Vec::new());
        response.file = Some((file, meta.len()));
        Ok(response)
    }

    pub fn json(status: u16, value: &serde_json::Value) -> Response {
        Response::new(status, "application/json", value.to_string())
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The part of a found body that `range` asks for, a `Range` header of one span of bytes,
    /// which a browser needs to seek in a video or a sound; all of it without one.
    pub fn ranged(mut self, range: Option<&str>) -> Response {
        if self.status != 200 {
            return self;
        }
        self = self.header("accept-ranges", "bytes");
        let total = self
            .file
            .as_ref()
            .map_or(self.body.len() as u64, |(_, n)| *n);
        let Some(span) = range.and_then(|r| r.strip_prefix("bytes=")) else {
            return self;
        };
        let bounds = span.split_once('-').and_then(|(from, to)| {
            let last = total.checked_sub(1)?;
            Some(
                match (from.trim().parse::<u64>(), to.trim().parse::<u64>()) {
                    (Ok(from), Ok(to)) => (from, to.min(last)),
                    (Ok(from), Err(_)) if to.trim().is_empty() => (from, last),
                    // The last `to` bytes.
                    (Err(_), Ok(to)) if from.trim().is_empty() => (total.saturating_sub(to), last),
                    _ => return None,
                },
            )
        });
        let Some((from, to)) = bounds.filter(|(from, to)| from <= to) else {
            self.status = 416;
            self.body.clear();
            self.file = None;
            return self.header("content-range", format!("bytes */{total}"));
        };
        self.status = 206;
        if let Some((file, length)) = &mut self.file {
            if let Err(e) = file.seek(SeekFrom::Start(from)) {
                return Self::new(500, "text/plain", e.to_string());
            }
            *length = to - from + 1;
        } else {
            self.body.truncate(to as usize + 1);
            self.body.drain(..from as usize);
        }
        self.header("content-range", format!("bytes {from}-{to}/{total}"))
    }

    /// The status line and headers, and the body unless the response is a stream, whose
    /// length is not known.
    pub fn write(&self, out: &mut impl Write, stream: bool) -> io::Result<()> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, reason(self.status));
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if !stream {
            let length = self
                .file
                .as_ref()
                .map_or(self.body.len() as u64, |(_, n)| *n);
            head.push_str(&format!("content-length: {length}\r\n"));
        }
        head.push_str("connection: close\r\n\r\n");
        out.write_all(head.as_bytes())?;
        if !stream {
            if let Some((file, length)) = &self.file {
                io::copy(&mut file.take(*length), out)?;
            } else {
                out.write_all(&self.body)?;
            }
        }
        out.flush()
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        409 => "Conflict",
        416 => "Range Not Satisfiable",
        428 => "Precondition Required",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "",
    }
}

/// `text` with its `%xx` escapes decoded, if they make UTF-8.
pub fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The content type of a file, by its extension.
pub fn kind(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    match ext.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "tsx" | "ts" | "txt" | "md" | "srt" | "vtt" | "lrc" | "csv" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_whole() {
        let text = "PUT /api/files/a%20b.json?name=x%2By&n=1 HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\
                    Content-Length: 4\r\nX-Valle-Base: none\r\n\r\nbody";
        let r = Request::read(&mut text.as_bytes()).expect("a request");
        assert_eq!(r.method, "PUT");
        assert_eq!(r.path, "/api/files/a b.json");
        assert_eq!(r.query("name"), Some("x+y"));
        assert_eq!(r.header("x-valle-base"), Some("none"));
        assert_eq!(r.body, b"body");
    }

    #[test]
    fn bad_escapes_are_refused() {
        assert_eq!(decode("%zz"), None);
        assert_eq!(decode("%e4%b8%ad"), Some("中".into()));
    }

    #[test]
    fn file_ranges_stream_only_the_requested_bytes() {
        let path = std::env::temp_dir().join(format!("valle-http-ranges-{}", std::process::id()));
        std::fs::write(&path, b"0123456789").unwrap();
        for (range, status, body) in [
            (None, 200, "0123456789"),
            (Some("bytes=2-4"), 206, "234"),
            (Some("bytes=7-"), 206, "789"),
            (Some("bytes=-3"), 206, "789"),
            (Some("bytes=8-99"), 206, "89"),
            (Some("bytes=10-"), 416, ""),
            (Some("bytes=4-2"), 416, ""),
            (Some("bytes=-0"), 416, ""),
        ] {
            let response = Response::file(&path, "video/mp4").unwrap().ranged(range);
            assert_eq!(response.status, status, "{range:?}");
            assert!(
                response.body.is_empty(),
                "files stay out of the in-memory body"
            );
            let mut wire = Vec::new();
            response.write(&mut wire, false).unwrap();
            let split = wire.windows(4).position(|p| p == b"\r\n\r\n").unwrap();
            assert_eq!(&wire[split + 4..], body.as_bytes(), "{range:?}");
        }
        // A large sparse file must remain cheap to seek, regardless of its total length.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(1 << 30)
            .unwrap();
        let response = Response::file(&path, "video/mp4")
            .unwrap()
            .ranged(Some("bytes=2-4"));
        assert!(response.body.is_empty());
        let mut wire = Vec::new();
        response.write(&mut wire, false).unwrap();
        assert!(wire.ends_with(b"\r\n\r\n234"));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            Response::file(&path, "video/mp4")
                .unwrap()
                .ranged(Some("bytes=0-"))
                .status,
            416
        );
        std::fs::remove_file(path).unwrap();
    }
}
