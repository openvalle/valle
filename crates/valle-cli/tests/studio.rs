//! Studio's local service, started as `valle project studio` starts it and spoken to over HTTP:
//! who it answers, the files it reads and writes, the changes it says, and the commands it runs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

/// The service, stopped when dropped.
struct Studio {
    child: Child,
    port: u16,
}

impl Drop for Studio {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A response: its status, its headers' lines and its body.
struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("JSON")
    }

    fn header(&self, name: &str) -> Option<String> {
        (self.head.lines())
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    }
}

impl Studio {
    fn start(dir: &Path, cache: &Path) -> Studio {
        let mut child = Command::new(VALLE)
            .args(["project", "studio"])
            .arg(dir)
            .args(["--port", "0", "--events"])
            .env("VALLE_CACHE_DIR", cache)
            .stdout(Stdio::piped())
            .spawn()
            .expect("valle runs");
        let mut line = String::new();
        let stdout = child.stdout.take().expect("its output");
        BufReader::new(stdout).read_line(&mut line).expect("a line");
        let ready: Value = serde_json::from_str(&line).expect("ready");
        assert_eq!(ready["event"], "ready", "{line}");
        let url = ready["url"].as_str().expect("its address");
        assert_eq!(url, format!("http://127.0.0.1:{}/", ready["port"]));
        Studio {
            child,
            port: ready["port"].as_u64().expect("its port") as u16,
        }
    }

    /// `method path` with `headers`, as a page of the service sends it.
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut all = vec![("Host".to_string(), format!("127.0.0.1:{}", self.port))];
        for (k, v) in headers {
            all.retain(|(n, _)| !n.eq_ignore_ascii_case(k));
            if !v.is_empty() {
                all.push((k.to_string(), v.to_string()));
            }
        }
        raw(self.port, method, path, &all, body)
    }

    fn get(&self, path: &str) -> Reply {
        self.send("GET", path, &[], &[])
    }
}

fn raw(port: u16, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    text.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream.write_all(text.as_bytes()).expect("sent");
    stream.write_all(body).expect("sent");
    let mut all = Vec::new();
    stream.read_to_end(&mut all).expect("an answer");
    let split = (all.windows(4))
        .position(|w| w == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&all[..split]).to_string();
    let status = head[9..12].parse().expect("a status");
    Reply {
        status,
        head,
        body: all[split + 4..].to_vec(),
    }
}

/// The events of the service's stream, as they come.
fn events(studio: &Studio) -> mpsc::Receiver<Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", studio.port)).expect("connects");
    let request = format!(
        "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        studio.port
    );
    stream.write_all(request.as_bytes()).expect("sent");
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Some(data) = line.strip_prefix("data: ")
                && send
                    .send(serde_json::from_str(data).expect("JSON"))
                    .is_err()
            {
                return;
            }
        }
    });
    receive
}

/// The next event `pick` takes, within 10 s.
fn next(events: &mpsc::Receiver<Value>, pick: impl Fn(&Value) -> bool) -> Value {
    loop {
        let event = (events.recv_timeout(Duration::from_secs(10))).expect("an event in 10 s");
        if pick(&event) {
            return event;
        }
    }
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("valle-studio-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

#[test]
#[cfg(feature = "embed-web")]
fn packaged_resources_negotiate_brotli_over_http() {
    let dir = folder("embedded");
    let project = dir.join("film");
    let created = Command::new(VALLE)
        .args(["project", "create"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let studio = Studio::start(&project, &dir.join("cache"));
    for (path, kind) in [
        ("/", "text/html; charset=utf-8"),
        ("/main.js", "text/javascript; charset=utf-8"),
        ("/valle/valle_web_bg.wasm", "application/wasm"),
    ] {
        let plain = studio.get(path);
        let br = studio.send("GET", path, &[("Accept-Encoding", "gzip, br")], &[]);
        assert_eq!((plain.status, br.status), (200, 200), "{path}");
        assert_eq!(br.header("content-type").as_deref(), Some(kind));
        assert_eq!(br.header("content-encoding").as_deref(), Some("br"));
        assert_eq!(br.header("vary").as_deref(), Some("Accept-Encoding"));
        assert_eq!(br.header("content-length"), Some(br.body.len().to_string()));
        assert!(plain.header("content-encoding").is_none());
        let mut decoded = Vec::new();
        brotli_decompressor::BrotliDecompress(&mut br.body.as_slice(), &mut decoded).unwrap();
        assert_eq!(decoded, plain.body, "{path}");
    }
    drop(studio);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_service_reads_writes_and_runs_for_its_page_only() {
    let dir = folder("project");
    let project = dir.join("film");
    let created = Command::new(VALLE)
        .args(["project", "create"])
        .arg(&project)
        .arg("--json")
        .output()
        .expect("valle runs");
    assert!(created.status.success(), "{created:?}");
    let studio = Studio::start(&project, &dir.join("cache"));

    // Local requests need no token; unrelated hosts and origins are still refused.
    assert_eq!(studio.get("/api/session").status, 200);
    let evil = [("Host", "evil.example:80")];
    assert_eq!(studio.send("GET", "/api/session", &evil, &[]).status, 403);
    let elsewhere = [("Origin", "https://evil.example")];
    assert_eq!(
        studio.send("GET", "/api/session", &elsewhere, &[]).status,
        403
    );
    let own = format!("http://127.0.0.1:{}", studio.port);
    assert_eq!(
        studio
            .send("GET", "/api/session", &[("Origin", &own)], &[])
            .status,
        200
    );

    let session = studio.get("/api/session");
    assert_eq!(
        session.header("cross-origin-embedder-policy").as_deref(),
        Some("require-corp")
    );
    let session = session.json();
    assert_eq!(session["kind"], "project");
    assert_eq!(session["entry"], "main.timeline.json");
    assert_eq!(session["timelines"][0], "main.timeline.json");

    // Files of the project, and nothing outside it.
    let timeline = studio.get("/api/files/main.timeline.json");
    assert_eq!(timeline.status, 200);
    let sha = timeline.header("x-valle-sha256").expect("its SHA-256");
    assert_eq!(studio.get("/api/files/../outside.txt").status, 403);
    assert_eq!(studio.get("/api/files/missing.txt").status, 404);

    let events = events(&studio);
    // A new file, once; a file written over what was seen, and not over what was not.
    let new = [("X-Valle-Base", "none")];
    assert_eq!(
        studio
            .send("PUT", "/api/files/notes/a.txt", &new, b"one")
            .status,
        200
    );
    assert_eq!(
        studio
            .send("PUT", "/api/files/notes/a.txt", &new, b"two")
            .status,
        409
    );
    assert_eq!(
        std::fs::read(project.join("notes/a.txt")).expect("written"),
        b"one"
    );
    let mut edited = timeline.body.clone();
    edited.extend(b"\n");
    let written = studio.send(
        "PUT",
        "/api/files/main.timeline.json",
        &[("X-Valle-Base", &sha)],
        &edited,
    );
    assert_eq!(written.status, 200);
    let stale = studio.send(
        "PUT",
        "/api/files/main.timeline.json",
        &[("X-Valle-Base", &sha)],
        b"{}",
    );
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["sha256"], written.json()["sha256"]);
    assert_eq!(
        studio
            .send("PUT", "/api/files/main.timeline.json", &[], b"{}")
            .status,
        428
    );

    // A change from outside is said; its own write was not.
    std::fs::write(project.join("main.timeline.json"), b"{\"changed\": true}\n").expect("written");
    let change = next(&events, |e| e["event"] == "change");
    assert_eq!(change["path"], "main.timeline.json");
    assert_ne!(change["sha256"], written.json()["sha256"]);

    // Project commands as the CLI runs them, and renders said as events.
    let body = br#"{ "args": ["history"] }"#;
    let history = studio.send("POST", "/api/project", &[], body);
    assert_eq!(
        history.json()["status"],
        "ok",
        "{}",
        String::from_utf8_lossy(&history.body)
    );
    let body = br#"{ "args": ["motion", "render", "missing.motion.tsx", "-o", "a.png"] }"#;
    let started = studio.send("POST", "/api/render", &[], body).json();
    assert_eq!(started["status"], "ok");
    let done = next(&events, |e| {
        e["event"] == "render" && e.get("result").is_some()
    });
    assert_eq!(done["id"], started["id"]);
    assert_eq!(done["result"]["status"], "error");
    // Timeline edits as `valle timeline edit` does them, and the folder's files.
    let doc = serde_json::json!({
        "canvas": { "width": 64, "height": 36, "fps": 30 },
        "tracks": { "visual": [{ "items": [
            { "id": "a", "kind": "solid", "color": "#000000ff", "duration": 1 }
        ] }] }
    });
    let op = serde_json::json!({ "op": "trim", "clip": "a", "duration": 0.5 });
    let body = serde_json::json!({ "text": doc.to_string(), "op": op }).to_string();
    let edited = studio
        .send("POST", "/api/edit", &[], body.as_bytes())
        .json();
    assert_eq!(
        edited["layout"]["duration"],
        serde_json::json!(0.5),
        "{edited}"
    );
    assert_eq!(edited["changes"][0]["before"]["end"], 1.0);
    assert_eq!(edited["changes"][0]["after"]["end"], 0.5);
    assert!(edited["summary"].as_str().unwrap().contains("updated a"));
    let listed = studio.get("/api/list").json();
    let files = listed["files"].as_array().expect("files");
    assert!(files.iter().any(|f| f["path"] == "notes/a.txt"), "{listed}");
    let body = br#"{ "args": ["studio"] }"#;
    assert_eq!(studio.send("POST", "/api/project", &[], body).status, 400);

    // Save dialogs are local-only and reject invalid names before opening a system panel.
    let body = br#"{ "name": "video.mp4" }"#;
    assert_eq!(
        studio
            .send("POST", "/api/save-file", &elsewhere, body)
            .status,
        403
    );
    for name in ["../video.mp4", "video.png", "folder/video.mp4"] {
        let body = serde_json::json!({ "name": name }).to_string();
        assert_eq!(
            studio
                .send("POST", "/api/save-file", &[], body.as_bytes())
                .status,
            400
        );
    }
    assert_eq!(
        studio.send("POST", "/api/directories", &[], b"{}").status,
        404
    );
    let output = dir.join("导出 videos");
    std::fs::create_dir_all(&output).expect("an output folder");
    std::fs::write(output.join("existing.mp4"), b"keep this video").expect("a video");
    assert_eq!(studio.get("/api/renders/999").status, 404);
    assert_eq!(studio.get("/api/renders/../outside.mp4").status, 404);

    let mut body = serde_json::json!({
        "args": ["motion", "render", "missing.motion.tsx", "-o", output.join("existing.mp4")]
    });
    let refused = studio.send("POST", "/api/render", &[], body.to_string().as_bytes());
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json()["code"], "output_exists");
    assert_eq!(
        std::fs::read(output.join("existing.mp4")).unwrap(),
        b"keep this video"
    );
    body["overwrite"] = true.into();
    let accepted = studio.send("POST", "/api/render", &[], body.to_string().as_bytes());
    assert_eq!(accepted.status, 200);
    let id = accepted.json()["id"].clone();
    let done = next(&events, |e| {
        e["event"] == "render" && e["id"] == id && e.get("result").is_some()
    });
    assert_eq!(done["result"]["status"], "error");
    assert_eq!(studio.get(&format!("/api/renders/{id}")).status, 404);
    body["args"][4] = serde_json::json!(output);
    assert_eq!(
        studio
            .send("POST", "/api/render", &[], body.to_string().as_bytes())
            .status,
        400
    );

    drop(studio);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_studio_out_of_a_project_opens_the_home() {
    let dir = folder("home");
    let project = dir.join("film");
    let created = Command::new(VALLE)
        .args(["project", "create"])
        .arg(&project)
        .output()
        .expect("valle runs");
    assert!(created.status.success(), "{created:?}");
    let mut child = Command::new(VALLE)
        .args(["project", "studio", "--port", "0", "--events"])
        .current_dir(&dir)
        .env("VALLE_CACHE_DIR", dir.join("cache"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("valle runs");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("output"))
        .read_line(&mut line)
        .expect("a line");
    let ready: Value = serde_json::from_str(&line).expect("ready");
    let studio = Studio {
        child,
        port: ready["port"].as_u64().expect("a port") as u16,
    };
    assert_eq!(studio.get("/api/session").json()["kind"], "home");
    assert_eq!(studio.get("/api/files/main.timeline.json").status, 403);
    let body = serde_json::json!({ "folder": project }).to_string();
    let opened = studio
        .send("POST", "/api/open", &[], body.as_bytes())
        .json();
    assert_eq!(opened["kind"], "project");
    assert_eq!(studio.get("/api/files/main.timeline.json").status, 200);
    // It is among the projects opened last.
    let recent = std::fs::read(dir.join("recent-projects.json")).expect("kept");
    let recent: Value = serde_json::from_slice(&recent).expect("JSON");
    assert_eq!(recent[0]["path"], project.display().to_string());
    drop(studio);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn linked_files_are_readable_only_while_the_project_references_them() {
    let dir = folder("linked");
    let root = dir.join("project");
    let project = valle_project::Project::create(&root, 64, 36, "30").unwrap();
    let linked = dir.join("linked audio.bin");
    std::fs::write(&linked, b"linked bytes").unwrap();
    project.add(std::slice::from_ref(&linked), true).unwrap();
    let mut doc: Value =
        serde_json::from_slice(&std::fs::read(root.join("main.timeline.json")).unwrap()).unwrap();
    doc["tracks"] = serde_json::json!({"visual":[{"items":[{"id":"one","kind":"solid","color":"red","duration":1}]}]});
    std::fs::write(root.join("main.timeline.json"), doc.to_string()).unwrap();
    let studio = Studio::start(&root, &dir.join("cache"));
    let url = format!(
        "/api/linked?file={}",
        linked
            .display()
            .to_string()
            .replace('\\', "/")
            .replace(' ', "%20")
    );
    let reply = studio.get(&url);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, b"linked bytes");
    assert_eq!(studio.get("/api/linked?file=relative.txt").status, 400);
    assert_eq!(studio.send("PUT", &url, &[], b"change").status, 404);
    doc["resources"] = serde_json::json!({});
    std::fs::write(root.join("main.timeline.json"), doc.to_string()).unwrap();
    assert_eq!(studio.get(&url).status, 403);
    assert_eq!(std::fs::read(linked).unwrap(), b"linked bytes");
    drop(studio);
    std::fs::remove_dir_all(dir).unwrap();
}
