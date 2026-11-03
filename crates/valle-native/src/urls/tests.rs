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
