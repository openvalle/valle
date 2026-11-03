//! `valle motion|timeline|project studio`: Studio's local service. It listens on 127.0.0.1 only,
//! for requests that carry the token it made when it started and come from its own origin; it
//! serves Studio and the SDK, reads and writes the files of what is open (atomically, refusing a
//! write over a change it did not see), runs `project` commands and renders as the CLI does, and
//! says what changes as server-sent events.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime};

use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use valle_engine::text::BUILT_IN;
use valle_native::urls::UrlCache;
use valle_project::{Project, digest, inside};

use crate::Failure;
use crate::http::{self, Request, Response};

/// How often the files Studio read are looked at for changes, and how long an event stream may
/// stay quiet.
const POLL: Duration = Duration::from_millis(300);
const PING: Duration = Duration::from_secs(15);
/// The projects opened last, newest first.
const RECENT: usize = 20;

/// What is open: a scene, a timeline or a project, the folder its files are read from and
/// written to, and the entry in it; or nothing yet, on the projects' home.
struct Session {
    kind: &'static str,
    root: Option<PathBuf>,
    entry: Option<String>,
}

/// A file Studio read: its path in the folder, and what it was last seen as.
struct Seen {
    path: String,
    len: u64,
    modified: Option<SystemTime>,
    sha256: Option<String>,
}

/// The event streams open, which renders' threads say to as well.
type Clients = Arc<Mutex<Vec<mpsc::Sender<String>>>>;

struct Service {
    token: String,
    port: u16,
    /// `web/` of the source: Studio's and the SDK's builds.
    web: PathBuf,
    exe: PathBuf,
    session: Mutex<Session>,
    watched: Mutex<BTreeMap<PathBuf, Seen>>,
    /// The event streams open.
    clients: Clients,
    /// Held while a file is compared and written, so two writes do not cross.
    writing: Mutex<()>,
    renders: AtomicU64,
}

pub fn run(group: &str, words: &[String], events: bool) -> Result<Value, Failure> {
    let usage = |m: String| Failure::new("usage", m).hint("valle --help lists the options");
    let (mut port, mut web, mut file) = (0u16, None, None);
    let mut words = words.iter();
    while let Some(w) = words.next() {
        match w.as_str() {
            "--json" | "--events" => {}
            "--port" => {
                let v = words
                    .next()
                    .ok_or_else(|| usage("--port needs a value".into()))?;
                port = v
                    .parse()
                    .map_err(|_| usage(format!("--port {v}: not a port")))?;
            }
            "--web-assets-dir" => {
                let v = words
                    .next()
                    .ok_or_else(|| usage(format!("{w} needs a value")))?;
                web = Some(PathBuf::from(v));
            }
            o if o.starts_with('-') => return Err(usage(format!("no option {o}"))),
            f if file.is_none() => file = Some(PathBuf::from(f)),
            extra => return Err(usage(format!("one file, not also {extra}"))),
        }
    }
    let session = open(group, file.as_deref())?;
    if let (Some(root), "project") = (&session.root, session.kind) {
        remember(root);
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| Failure::new("studio", format!("127.0.0.1:{port}: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Failure::new("studio", e.to_string()))?;
    let mut token = [0u8; 16];
    (SystemRandom::new().fill(&mut token))
        .map_err(|_| Failure::new("studio", "no random numbers for a token"))?;
    let service = Arc::new(Service {
        token: token.iter().map(|b| format!("{b:02x}")).collect(),
        port: port.port(),
        // A build from the source finds its web/; a packaged one is given it.
        web: web.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web")),
        exe: std::env::current_exe().map_err(|e| Failure::new("studio", e.to_string()))?,
        session: Mutex::new(session),
        watched: Mutex::new(BTreeMap::new()),
        clients: Arc::default(),
        writing: Mutex::new(()),
        renders: AtomicU64::new(0),
    });
    let url = format!("http://127.0.0.1:{}/?token={}", service.port, service.token);
    match events {
        true => {
            let ready = json!({ "event": "ready", "url": url, "port": service.port });
            println!("{ready}");
        }
        false => {
            println!("Studio is at {url}\nstop it with Ctrl-C");
            browse(&url);
        }
    }
    let _ = std::io::stdout().flush();
    let watcher = service.clone();
    std::thread::spawn(move || watcher.watch());
    for stream in listener.incoming().flatten() {
        let service = service.clone();
        std::thread::spawn(move || service.serve(stream));
    }
    Ok(json!({ "status": "ok", "summary": "Studio stopped" }))
}

/// What `valle <group> studio <file>` opens.
fn open(group: &str, file: Option<&Path>) -> Result<Session, Failure> {
    let missing = |p: &Path| Failure::new("usage", format!("{}: no such file", p.display()));
    if group == "project" {
        let start = file.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        return Ok(match Project::discover(&start) {
            Ok(project) => Session {
                kind: "project",
                entry: Some(project.timeline.clone()),
                root: Some(project.root),
            },
            // Not in a project: the home, of the projects opened last.
            Err(_) if file.is_none() => Session {
                kind: "home",
                root: None,
                entry: None,
            },
            Err(e) => return Err(Failure::new("project", e)),
        });
    }
    let file = file.ok_or_else(|| Failure::new("usage", "which file?"))?;
    if !file.is_file() {
        return Err(missing(file));
    }
    let timeline = file.to_string_lossy().ends_with(".json");
    if timeline != (group == "timeline") {
        return Err(Failure::new(
            "usage",
            match timeline {
                true => "a .json is a timeline: valle timeline studio",
                false => "valle timeline studio takes a timeline's .json",
            },
        ));
    }
    let file = valle_project::absolute(file);
    // The project it is in, else its folder.
    let root = match Project::discover(file.parent().unwrap_or(Path::new("."))) {
        Ok(project) => project.root,
        Err(_) => file.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    let entry = relative(&root, &file).ok_or_else(|| missing(&file))?;
    Ok(Session {
        kind: if timeline { "timeline" } else { "motion" },
        root: Some(root),
        entry: Some(entry),
    })
}

/// `file`'s path in `root`, `/`-separated.
fn relative(root: &Path, file: &Path) -> Option<String> {
    let parts: Vec<String> = (file.strip_prefix(root).ok()?.components())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

/// Opens `url` in the browser, if the system says how.
fn browse(url: &str) {
    let command = match std::env::consts::OS {
        "macos" => Command::new("open").arg(url).spawn(),
        "windows" => Command::new("cmd").args(["/c", "start", "", url]).spawn(),
        _ => Command::new("xdg-open").arg(url).spawn(),
    };
    if command.is_err() {
        eprintln!("open {url} in a browser");
    }
}

/// Where the projects opened last are kept: beside the global cache.
fn recent_file() -> Option<PathBuf> {
    let cache = match std::env::var_os("VALLE_CACHE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir()?.join(".valle").join("cache"),
    };
    Some(cache.parent()?.join("recent-projects.json"))
}

/// The projects opened last, newest first.
fn recent() -> Vec<Value> {
    let read = recent_file().and_then(|f| std::fs::read(f).ok());
    let list: Option<Vec<Value>> = read.and_then(|b| serde_json::from_slice(&b).ok());
    list.unwrap_or_default()
}

/// Puts `root` first among the projects opened last.
fn remember(root: &Path) {
    let Some(file) = recent_file() else {
        return;
    };
    let path = root.display().to_string();
    let mut list = recent();
    list.retain(|p| p["path"] != path.as_str());
    let opened = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    list.insert(0, json!({ "path": path, "opened": opened }));
    list.truncate(RECENT);
    let _ = valle_project::write(&file, &valle_project::pretty(&Value::from(list)), true);
}

impl Service {
    fn serve(&self, stream: TcpStream) {
        let mut reader = BufReader::new(&stream);
        let request = match Request::read(&mut reader) {
            Ok(r) => r,
            Err(e) => {
                let _ = self
                    .finish(Response::new(400, "text/plain", e))
                    .write(&mut &stream, false);
                return;
            }
        };
        if let Err(response) = self.allowed(&request) {
            let _ = self.finish(response).write(&mut &stream, false);
            return;
        }
        if request.method == "GET" && request.path == "/api/events" {
            self.stream(&stream);
            return;
        }
        let response = self.handle(&request);
        let _ = self.finish(response).write(&mut &stream, false);
    }

    /// Refuses a request not from this service's page: another host or origin, which a page
    /// elsewhere would send, or an API request without the token.
    fn allowed(&self, request: &Request) -> Result<(), Response> {
        let forbidden = |m: &str| Err(Response::new(403, "text/plain", m.to_string()));
        let hosts = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        if !request
            .header("host")
            .is_some_and(|h| hosts.iter().any(|x| x == h))
        {
            return forbidden("not this host");
        }
        if let Some(origin) = request.header("origin")
            && !hosts.iter().any(|h| origin == format!("http://{h}"))
        {
            return forbidden("not this origin");
        }
        let token = request.header("x-valle-token").or(request.query("token"));
        if request.path.starts_with("/api/") && token != Some(self.token.as_str()) {
            return forbidden("no token, or not this service's");
        }
        Ok(())
    }

    /// The headers every response has: the page is isolated across origins, and nothing of the
    /// files is cached by the browser, as they change.
    fn finish(&self, response: Response) -> Response {
        response
            .header("cross-origin-opener-policy", "same-origin")
            .header("cross-origin-embedder-policy", "require-corp")
            .header("cross-origin-resource-policy", "same-origin")
            .header("cache-control", "no-store")
    }

    fn handle(&self, request: &Request) -> Response {
        let path = request.path.as_str();
        let method = request.method.as_str();
        let result = match (method, path) {
            ("GET", "/api/session") => Ok(self.session_json()),
            ("POST", "/api/open") => self.open_project(&request.body),
            ("POST", "/api/project") => self.command(&request.body),
            ("POST", "/api/render") => self.render(&request.body),
            ("GET", "/api/url") => return url(request.query("u")),
            ("GET", p) if p.starts_with("/api/fonts/") => return font(&p["/api/fonts/".len()..]),
            ("GET", p) if p.starts_with("/api/files/") => return self.read(&p[11..]),
            ("PUT", p) if p.starts_with("/api/files/") => return self.write(&p[11..], request),
            ("GET", p) if !p.starts_with("/api/") => return self.asset(p),
            _ => return Response::new(404, "text/plain", "no such request"),
        };
        match result {
            Ok(value) => Response::json(200, &value),
            Err(e) => Response::json(400, &json!({ "status": "error", "message": e })),
        }
    }

    /// Studio's files, and the SDK's under `/valle/`.
    fn asset(&self, path: &str) -> Response {
        let (dir, file) = match path.strip_prefix("/valle/") {
            Some(rest) => ("packages/engine/dist", rest),
            None => ("apps/studio/dist", path.trim_start_matches('/')),
        };
        let file = if file.is_empty() { "index.html" } else { file };
        let Some(file) = inside(file) else {
            return Response::new(403, "text/plain", "not in Studio");
        };
        let full = self.web.join(dir).join(&file);
        match std::fs::read(&full) {
            Ok(bytes) => Response::new(200, http::kind(&file), bytes),
            Err(_) if file == "index.html" => Response::new(
                404,
                "text/plain",
                format!(
                    "Studio is not built in {}: run cargo xtask build-web",
                    self.web.display()
                ),
            ),
            Err(_) => Response::new(404, "text/plain", "not found"),
        }
    }

    fn session_json(&self) -> Value {
        let session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let root = session.root.as_ref();
        let mut out = json!({
            "status": "ok",
            "kind": session.kind,
            "entry": session.entry,
            "root": root.map(|r| r.display().to_string()),
            "name": root.and_then(|r| r.file_name()).map(|n| n.to_string_lossy()),
        });
        if session.kind == "project"
            && let Some(project) = root.and_then(|r| Project::open(r).ok())
        {
            out["timelines"] = project.timelines().into();
        }
        if session.kind == "home" {
            out["recent"] = recent().into();
        }
        out
    }

    /// Opens the project in `{ folder }`, in place of what is open on the home or another
    /// project.
    fn open_project(&self, body: &[u8]) -> Result<Value, String> {
        let body: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
        let folder = body["folder"].as_str().ok_or("which folder?")?;
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        if !["home", "project"].contains(&session.kind) {
            return Err("only a project's Studio opens another project".into());
        }
        let project = Project::open(Path::new(folder))?;
        remember(&project.root);
        *session = Session {
            kind: "project",
            entry: Some(project.timeline.clone()),
            root: Some(project.root),
        };
        drop(session);
        self.watched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(self.session_json())
    }

    fn root(&self) -> Option<PathBuf> {
        let session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        session.root.clone()
    }

    /// The file at `path` in the folder, whose changes are said from now on.
    fn read(&self, path: &str) -> Response {
        let Some((full, path)) = self.place(path) else {
            return Response::new(403, "text/plain", "not in the folder Studio has open");
        };
        let Ok(bytes) = std::fs::read(&full) else {
            return Response::new(404, "text/plain", "not found");
        };
        let sha256 = valle_project::digest_bytes(&bytes);
        let meta = std::fs::metadata(&full).ok();
        let mut watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        watched.insert(
            full,
            Seen {
                path: path.clone(),
                len: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()),
                sha256: Some(sha256.clone()),
            },
        );
        Response::new(200, http::kind(&path), bytes).header("x-valle-sha256", sha256)
    }

    /// Writes the body to the file at `path`, whole or not at all, if the file is still what
    /// `x-valle-base` says the page last saw: its SHA-256, or `none` for no file.
    fn write(&self, path: &str, request: &Request) -> Response {
        let Some((full, path)) = self.place(path) else {
            return Response::new(403, "text/plain", "not in the folder Studio has open");
        };
        let Some(base) = request.header("x-valle-base") else {
            return Response::new(
                428,
                "text/plain",
                "x-valle-base: what the file was, or none",
            );
        };
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let current = full.is_file().then(|| digest(&full).ok()).flatten();
        if current.as_deref() != (base != "none").then_some(base) {
            let body = json!({ "status": "conflict", "sha256": current });
            return Response::json(409, &body);
        }
        if let Err(e) = valle_project::write(&full, &request.body, true) {
            return Response::json(500, &json!({ "status": "error", "message": e }));
        }
        let sha256 = valle_project::digest_bytes(&request.body);
        // Seen as written, so the write is not said as a change.
        let meta = std::fs::metadata(&full).ok();
        let mut watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        watched.insert(
            full,
            Seen {
                path,
                len: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()),
                sha256: Some(sha256.clone()),
            },
        );
        Response::json(200, &json!({ "status": "ok", "sha256": sha256 }))
    }

    /// `path` in the folder open, as a file and as its normalized path.
    fn place(&self, path: &str) -> Option<(PathBuf, String)> {
        let path = inside(path)?;
        Some((self.root()?.join(&path), path))
    }

    /// Runs `valle project <args> --json` on the project open, as the CLI does.
    fn command(&self, body: &[u8]) -> Result<Value, String> {
        let args = arguments(body)?;
        if args.first().map(String::as_str) == Some("studio") {
            return Err("Studio is open already".into());
        }
        let mut command = Command::new(&self.exe);
        command.arg("project").args(&args).arg("--json");
        if let Some(root) = self.root()
            && args.first().map(String::as_str) != Some("create")
        {
            command.arg("-C").arg(root);
        }
        let done = (command.stderr(Stdio::null()).output()).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&done.stdout);
        serde_json::from_str(text.trim()).map_err(|_| format!("valle project: {text}"))
    }

    /// Starts `valle <motion|timeline|project> render <args>` in the folder open, as the CLI
    /// does; what it does is said as `render` events with its id.
    fn render(&self, body: &[u8]) -> Result<Value, String> {
        let args = arguments(body)?;
        match args.as_slice() {
            [group, command, ..]
                if command == "render" && ["motion", "timeline", "project"].contains(&&**group) => {
            }
            _ => return Err("a render is motion, timeline or project render".into()),
        }
        let root = self.root().ok_or("no folder is open")?;
        let id = self.renders.fetch_add(1, Ordering::Relaxed) + 1;
        let mut child = Command::new(&self.exe)
            .args(&args)
            .arg("--events")
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("no output")?;
        let clients = self.clients.clone();
        let clients = move |event: Value| say(&clients, &event);
        std::thread::spawn(move || {
            let mut last = None;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("event").is_some() {
                    let mut event = value;
                    event["render"] = id.into();
                    clients(json!({ "event": "render", "id": id, "progress": event }));
                } else {
                    last = Some(value);
                }
            }
            let _ = child.wait();
            let result = last
                .unwrap_or_else(|| json!({ "status": "error", "message": "the render stopped" }));
            clients(json!({ "event": "render", "id": id, "result": result }));
        });
        Ok(json!({ "status": "ok", "id": id }))
    }

    /// Server-sent events on `stream` until it closes: those `say` says, and a comment now and
    /// then so the connection is kept.
    fn stream(&self, stream: &TcpStream) {
        let (send, events) = mpsc::channel();
        let head = Response::new(200, "text/event-stream", Vec::new());
        if self.finish(head).write(&mut &*stream, true).is_err() {
            return;
        }
        let _ = (&*stream).write_all(b": open\n\n");
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(send);
        loop {
            let text = match events.recv_timeout(PING) {
                Ok(event) => format!("data: {event}\n\n"),
                Err(mpsc::RecvTimeoutError::Timeout) => ": ping\n\n".to_string(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            if (&*stream).write_all(text.as_bytes()).is_err() {
                return;
            }
        }
    }

    /// Looks at the files Studio read, and says each that changed: `change` with its path and
    /// SHA-256, null once it is gone.
    fn watch(&self) {
        loop {
            std::thread::sleep(POLL);
            let mut changed = Vec::new();
            {
                let mut watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
                for (full, seen) in watched.iter_mut() {
                    let meta = std::fs::metadata(full).ok();
                    let len = meta.as_ref().map_or(0, |m| m.len());
                    let modified = meta.as_ref().and_then(|m| m.modified().ok());
                    if meta.is_some() && len == seen.len && modified == seen.modified {
                        continue;
                    }
                    let sha256 = meta.and_then(|_| digest(full).ok());
                    (seen.len, seen.modified) = (len, modified);
                    if sha256 != seen.sha256 {
                        seen.sha256 = sha256.clone();
                        changed.push(
                            json!({ "event": "change", "path": seen.path, "sha256": sha256 }),
                        );
                    }
                }
            }
            for event in changed {
                say(&self.clients, &event);
            }
        }
    }
}

/// Says `event` to every stream open.
fn say(clients: &Clients, event: &Value) {
    let text = event.to_string();
    let mut clients = clients.lock().unwrap_or_else(|e| e.into_inner());
    clients.retain(|c| c.send(text.clone()).is_ok());
}

/// `{ "args": [...] }`'s words.
fn arguments(body: &[u8]) -> Result<Vec<String>, String> {
    let body: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let args = body["args"].as_array().ok_or("{ args: [...] }")?;
    (args.iter())
        .map(|a| {
            a.as_str()
                .map(str::to_string)
                .ok_or("each arg is a string".into())
        })
        .collect()
}

/// A built-in font's file, by its name, from the global cache the CLI fills.
fn font(file: &str) -> Response {
    let Some(font) = BUILT_IN.iter().find(|f| f.file == file) else {
        return Response::new(404, "text/plain", "no built-in font of that name");
    };
    let fetched = UrlCache::global().and_then(|c| c.fetch_pinned(font.url, font.sha256, font.file));
    match fetched.and_then(|p| std::fs::read(&p).map_err(|e| e.to_string())) {
        Ok(bytes) => Response::new(200, http::kind(file), bytes),
        Err(e) => Response::new(502, "text/plain", e),
    }
}

/// The file at a URL, downloaded into the global cache and served from this origin, so the page
/// needs no CORS from its server.
fn url(url: Option<&str>) -> Response {
    let Some(url) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) else {
        return Response::new(400, "text/plain", "u=<an http or https URL>");
    };
    let fetched = UrlCache::global().and_then(|c| c.fetch(url));
    match fetched.and_then(|p| std::fs::read(&p).map_err(|e| e.to_string())) {
        Ok(bytes) => Response::new(200, http::kind(url.split('?').next().unwrap_or(url)), bytes),
        Err(e) => Response::new(502, "text/plain", e),
    }
}
