//! Local Studio HTTP host for verified runtime files, project saves, byte ranges, and SSE reloads.
//! Config updates replace `/config.json` and notify clients through `/events`. Each connection
//! handles one request; SSE writes flush immediately. Bind only to 127.0.0.1 and serve only
//! manifest-listed files or sandboxed project assets.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use valle_project::revision::{AuthenticatedContext, ProjectId, ProjectStore};

/// Bound Project authoring session. The token and project id are fixed when
/// the local server starts; every request is checked against both.
pub struct ProjectStudioCtx {
    pub token: String,
    pub project_id: ProjectId,
    pub initial_revision: u64,
    pub store: ProjectStore,
    pub auth: AuthenticatedContext,
}

/// Runtime files, project assets, replaceable configuration, and SSE state.
pub struct StudioHost {
    /// Allowlisted public runtime URLs mapped to verified bytes or explicit local assets.
    pub runtime_files: BTreeMap<String, crate::webruntime::HostedFile>,
    pub assets_dir: Option<PathBuf>,
    /// Contents of `/config.json`, replaced atomically before notifying SSE clients.
    pub config_json: Arc<RwLock<String>>,
    pub sse: SseBroadcaster,
    /// Optional directory for saving browser PNG captures from POST /result.
    pub capture_dir: Option<PathBuf>,
    /// Enable the asset library command endpoint when configured.
    pub library: Option<LibraryCtx>,
    /// Project mode exposes the authoritative snapshot and complete-document
    /// edit endpoint. It never fabricates renderer fulfillment data.
    pub project: Option<ProjectStudioCtx>,
    /// Latest browser report, exposed through GET /smoke-report.
    pub last_report: RwLock<Option<String>>,
}

fn project_snapshot_json(project: &ProjectStudioCtx) -> Result<String> {
    let snapshot = project
        .store
        .get_timeline(&project.project_id, None)
        .context("reading Project Studio HEAD")?;
    // `timeline` is the sole authoring and persistence shape. The expanded
    // document is exposed only under the explicitly internal `render` key so
    // Studio can inspect the compiled projection without mistaking it for the
    // working copy that editTimeline replaces. No renderer fulfillment is
    // implied until a complete fixed package is admitted elsewhere.
    let timeline_json = String::from_utf8(valle_timeline::timeline_bytes(snapshot.timeline())?)
        .context("Timeline was not UTF-8")?;
    let timeline: serde_json::Value = serde_json::from_str(&timeline_json)?;
    let render_timeline_json = String::from_utf8(valle_timeline::internal::canonical_bytes(
        snapshot.canonical(),
    )?)
    .context("compiled Timeline was not UTF-8")?;
    let render_timeline: serde_json::Value = serde_json::from_str(&render_timeline_json)?;
    let revision = snapshot.revision();
    let timeline_revision = serde_json::json!({
        "revision": revision.revision,
        "parentRevision": revision.parent_revision,
        "createdAt": revision.created_at,
        "actor": revision.actor,
        "cause": revision.cause,
        "intent": revision.intent,
    });
    Ok(serde_json::json!({
        "projectId": snapshot.project_id(),
        "timelineRevision": timeline_revision,
        "timelineJson": timeline_json,
        "timeline": timeline,
        "render": {
            "timelineJson": render_timeline_json,
            "timeline": render_timeline,
        },
        "preview": {
            "status": "unavailable",
            "code": "verified_binding_bundle_unavailable"
        }
    })
    .to_string())
}

fn project_boot(project: &ProjectStudioCtx, config_json: &str) -> Result<String> {
    let config = serde_json::from_str::<serde_json::Value>(config_json).unwrap_or_default();
    let runtime_assets = config
        .get("runtimeAssets")
        .unwrap_or(&serde_json::Value::Null);
    let mut asset_urls = serde_json::Map::new();
    for (name, pointer) in [
        ("engineGlue", "/engine/glue"),
        ("engineWasm", "/engine/wasm"),
        ("canvasKitBaseGlue", "/canvasKit/base/glue"),
        ("canvasKitBaseWasm", "/canvasKit/base/wasm"),
        ("canvasKitFullGlue", "/canvasKit/full/glue"),
        ("canvasKitFullWasm", "/canvasKit/full/wasm"),
        ("defaultSansFont", "/fonts/defaultSans"),
        ("productFrameWorker", "/workers/productFrame"),
    ] {
        if let Some(value) = runtime_assets
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
        {
            asset_urls.insert(name.to_owned(), serde_json::Value::String(value.to_owned()));
        }
    }
    Ok(serde_json::json!({
        "protocolVersion": crate::webruntime::PROTOCOL_VERSION,
        "session": {
            "kind": "project",
            "projectId": project.project_id,
            "revision": project.initial_revision,
            "token": project.token,
        },
        "capabilities": {
            "saveTimeline": true,
            "editProject": true,
            "editMotionProps": false,
            "writeMotionSource": false,
        },
        "runtime": {
            "assetBaseUrl": "/assets/",
            "assetUrls": asset_urls,
        },
    })
    .to_string())
}

/// Motion Studio boot protocol.
fn studio_boot_from_config(config_json: &str) -> Result<String> {
    let config: serde_json::Value =
        serde_json::from_str(config_json).context("parse Studio config")?;
    let input = config
        .get("input")
        .and_then(|value| value.as_str())
        .unwrap_or("motion");

    let (session, capabilities, default_asset_base) = if config.get("generation").is_some()
        && config.get("input").is_some()
    {
        (
            serde_json::json!({
                "kind": "motion-file",
                "input": input,
                "generation": config.get("generation").and_then(|value| value.as_u64()).unwrap_or(1),
            }),
            serde_json::json!({
                "saveTimeline": false,
                "editProject": false,
                "editMotionProps": true,
                "writeMotionSource": false,
            }),
            "/motion-assets/".to_owned(),
        )
    } else {
        anyhow::bail!("Studio boot is unavailable for this host");
    };

    let runtime_assets = config
        .get("runtimeAssets")
        .unwrap_or(&serde_json::Value::Null);
    let mut asset_urls = serde_json::Map::new();
    for (name, pointer) in [
        ("engineGlue", "/engine/glue"),
        ("engineWasm", "/engine/wasm"),
        ("canvasKitBaseGlue", "/canvasKit/base/glue"),
        ("canvasKitBaseWasm", "/canvasKit/base/wasm"),
        ("canvasKitFullGlue", "/canvasKit/full/glue"),
        ("canvasKitFullWasm", "/canvasKit/full/wasm"),
        ("defaultSansFont", "/fonts/defaultSans"),
        ("productFrameWorker", "/workers/productFrame"),
    ] {
        if let Some(value) = runtime_assets
            .pointer(pointer)
            .and_then(|value| value.as_str())
        {
            asset_urls.insert(name.to_owned(), serde_json::Value::String(value.to_owned()));
        }
    }
    let asset_base_url = config
        .get("assetBaseUrl")
        .and_then(|value| value.as_str())
        .unwrap_or(&default_asset_base);
    let runtime = serde_json::json!({
        "assetBaseUrl": asset_base_url,
        "assetUrls": asset_urls,
    });

    Ok(serde_json::json!({
        "protocolVersion": crate::webruntime::PROTOCOL_VERSION,
        "session": session,
        "capabilities": capabilities,
        "runtime": runtime,
    })
    .to_string())
}

/// Library session token and per-request kernel context; tests may supply adapters.
pub struct LibraryCtx {
    pub token: String,
    pub make_ctx: Box<dyn Fn() -> valle_project::assets::Ctx + Send + Sync>,
    /// Optional video thumbnail extractor. Receives the library home, full hash, media path, and
    /// duration, then returns a cached PNG path.
    pub thumbnail: Option<ThumbnailFn>,
    /// Allow one analysis run at a time; concurrent requests return `locked`.
    pub analyzing: AtomicBool,
    /// Optional analysis adapter for sidecar lifetime management; otherwise execute directly.
    pub run_analyze: Option<AnalyzeFn>,
}

pub type AnalyzeFn = Box<
    dyn Fn(
            &valle_project::assets::Ctx,
            valle_project::assets::Verb,
        ) -> valle_project::assets::Report
        + Send
        + Sync,
>;

pub type ThumbnailFn = Box<
    dyn Fn(&valle_project::assets::Home, &str, &Path, Option<i64>) -> anyhow::Result<PathBuf>
        + Send
        + Sync,
>;

/// Keep SQL and permanent deletion exclusive to the CLI.
fn library_verb_allowed(verb: &valle_project::assets::Verb) -> bool {
    !matches!(
        verb,
        valle_project::assets::Verb::Sql { .. }
            | valle_project::assets::Verb::Rm { purge: true, .. }
    )
}

fn local_request_authorized(token: &str, expected: &str, origin: Option<&str>) -> bool {
    token_eq(token, expected) && origin.is_some_and(origin_allowed)
}

fn token_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn origin_allowed(value: &str) -> bool {
    let value = value.trim();
    let without_scheme = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(value);
    let host_port = without_scheme.split('/').next().unwrap_or("");
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => host,
            _ => host_port,
        }
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// One channel per SSE client; discard disconnected clients on the next broadcast.
#[derive(Default)]
pub struct SseBroadcaster {
    clients: Mutex<Vec<Sender<String>>>,
}

impl SseBroadcaster {
    pub fn broadcast(&self, event: &str, data: &str) {
        let frame = format!("event: {event}\ndata: {data}\n\n");
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain(|tx| tx.send(frame.clone()).is_ok());
        }
    }

    fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut clients) = self.clients.lock() {
            clients.push(tx);
        }
        rx
    }
}

/// Bind a loopback port and return its actual address; 0 selects an available port.
pub fn bind(port: u16) -> Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("binding 127.0.0.1:{port}"))?;
    let addr = listener.local_addr().context("reading bound address")?;
    Ok((listener, addr))
}

/// Serve each accepted connection on its own thread to support parallel asset requests.
pub fn serve_forever(listener: TcpListener, state: Arc<StudioHost>) -> Result<()> {
    for stream in listener.incoming() {
        let stream = stream.context("accepting connection")?;
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            if let Err(e) = handle_connection(stream, &state) {
                if std::env::var_os("VALLE_WEBHOST_DEBUG").is_some() {
                    eprintln!("webhost conn error: {e:#}");
                }
            }
        });
    }
    anyhow::bail!("Motion Studio server stopped accepting connections");
}

/// Handle one request per connection with `Connection: close`; SSE ends at EOF.
fn handle_connection(stream: TcpStream, state: &Arc<StudioHost>) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone().context("cloning stream")?);
    let (method, raw_path, headers) = read_request_head(&mut reader)?;
    let raw_target = raw_path
        .split_once('?')
        .map_or(raw_path.as_str(), |(path, _)| path);
    let path = percent_decode(raw_target);
    if std::env::var_os("VALLE_WEBHOST_DEBUG").is_some() {
        eprintln!("webhost req: {method} {path}");
    }
    let mut out = stream;

    if method == "POST" && path == "/result" {
        let body = read_body(&mut reader, &headers)?;
        if let Ok(mut slot) = state.last_report.write() {
            *slot = Some(body.clone());
        }
        // Respond before processing reports so capture writes cannot delay the browser request.
        let resp = write_simple(&mut out, 204, "No Content", &[], b"");
        handle_browser_report(&body, state);
        return resp;
    }
    if method == "POST" && path == "/library/execute" {
        let body = read_body(&mut reader, &headers)?;
        return serve_library_execute(&mut out, &headers, &body, state);
    }
    if method == "POST" && path == "/timeline/edit" {
        let body = match read_body(&mut reader, &headers) {
            Ok(body) => body,
            Err(error) => {
                return write_simple(
                    &mut out,
                    400,
                    "Bad Request",
                    &[("Content-Type", "application/json")],
                    serde_json::json!({
                        "error": {"code": "invalid_request_body", "message": error.to_string()}
                    })
                    .to_string()
                    .as_bytes(),
                );
            }
        };
        return serve_project_edit(&mut out, &raw_path, &headers, &body, state);
    }
    if method != "GET" {
        return write_simple(&mut out, 405, "Method Not Allowed", &[], b"");
    }

    if path == "/events" {
        return serve_sse(&mut out, state);
    }

    if path == "/smoke-report" {
        let report = state.last_report.read().ok().and_then(|g| g.clone());
        return match report {
            Some(json) => write_simple(
                &mut out,
                200,
                "OK",
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                ],
                json.as_bytes(),
            ),
            None => write_simple(&mut out, 404, "Not Found", &[], b""),
        };
    }

    if path == "/studio/boot.json" {
        let config = state
            .config_json
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let boot = match &state.project {
            Some(project) if request_matches_project(&raw_path, project) => {
                project_boot(project, &config)
            }
            Some(_) => Err(anyhow::anyhow!(
                "Project Studio request does not match the bound project"
            )),
            None => studio_boot_from_config(&config),
        };
        return match boot {
            Ok(boot) => write_simple(
                &mut out,
                200,
                "OK",
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                ],
                boot.as_bytes(),
            ),
            Err(error) => write_simple(
                &mut out,
                404,
                "Not Found",
                &[("Content-Type", "text/plain; charset=utf-8")],
                format!("{error:#}").as_bytes(),
            ),
        };
    }

    if path == "/timeline/get" {
        let Some(project) = &state.project else {
            return write_simple(&mut out, 404, "Not Found", &[], b"");
        };
        if !request_matches_project(&raw_path, project) {
            return write_simple(&mut out, 404, "Not Found", &[], b"");
        }
        return match project_snapshot_json(project) {
            Ok(snapshot) => write_simple(
                &mut out,
                200,
                "OK",
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                ],
                snapshot.as_bytes(),
            ),
            Err(error) => write_simple(
                &mut out,
                500,
                "Internal Server Error",
                &[("Content-Type", "application/json")],
                serde_json::json!({"error":{"code":"project_snapshot_failed","message":format!("{error:#}")}})
                    .to_string()
                    .as_bytes(),
            ),
        };
    }

    if path == "/config.json" {
        // Resolve a project-specific compiled configuration; return a reason when unavailable.
        let config = state
            .config_json
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        return write_simple(
            &mut out,
            200,
            "OK",
            &[
                ("Content-Type", "application/json"),
                // Configuration must be fetched again after each reload.
                ("Cache-Control", "no-store"),
            ],
            config.as_bytes(),
        );
    }

    // Resolve project-local assets within the project directory sandbox.
    if let Some(rel) = path.strip_prefix("/library/blob/") {
        return serve_library_blob(&mut out, &headers, rel, state);
    }
    if let Some(rel) = path.strip_prefix("/library/thumb/") {
        return serve_library_thumb(&mut out, &headers, rel, state);
    }

    let runtime_rel = if path == "/" {
        "preview.html"
    } else if path == "/studio" {
        // Route the Studio alias through the runtime allowlist.
        "studio.html"
    } else {
        path.strip_prefix('/').unwrap_or("")
    };
    if let Some(file) = state.runtime_files.get(runtime_rel) {
        return serve_hosted_file(&mut out, &headers, file, content_type_for(runtime_rel));
    }

    if let Some(rel) = path.strip_prefix("/assets/") {
        let Some(root) = state.assets_dir.as_deref() else {
            return write_simple(&mut out, 404, "Not Found", &[], b"");
        };
        let Some(file) = sandboxed_join(root, rel) else {
            return write_simple(&mut out, 404, "Not Found", &[], b"");
        };
        return serve_file(&mut out, &headers, &file, content_type_for(rel));
    }

    write_simple(&mut out, 404, "Not Found", &[], b"")
}

fn serve_hosted_file(
    out: &mut TcpStream,
    headers: &[(String, String)],
    file: &crate::webruntime::HostedFile,
    content_type: &str,
) -> Result<()> {
    match file {
        crate::webruntime::HostedFile::VerifiedRuntime(bytes) => {
            serve_bytes(out, headers, bytes, content_type)
        }
        crate::webruntime::HostedFile::LocalPath(path) => {
            serve_file(out, headers, path, content_type)
        }
    }
}

fn serve_project_edit(
    out: &mut TcpStream,
    raw_path: &str,
    headers: &[(String, String)],
    body: &str,
    state: &Arc<StudioHost>,
) -> Result<()> {
    let Some(project) = &state.project else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    if !request_matches_project(raw_path, project) {
        return write_simple(out, 404, "Not Found", &[], b"");
    }
    let token = header_value(headers, "x-valle-token").unwrap_or("");
    let origin = header_value(headers, "origin").or_else(|| header_value(headers, "host"));
    if !local_request_authorized(token, &project.token, origin) {
        return write_simple(
            out,
            403,
            "Forbidden",
            &[("Content-Type", "application/json")],
            br#"{"error":{"code":"refused","message":"token/origin check failed"}}"#,
        );
    }
    match project
        .store
        .edit_timeline_json(&project.project_id, body, &project.auth)
    {
        Ok(response) => {
            let json = serde_json::to_vec(&response)?;
            write_simple(
                out,
                200,
                "OK",
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                ],
                &json,
            )
        }
        Err(valle_project::revision::EditTimelineServiceError::Decode(error)) => write_simple(
            out,
            400,
            "Bad Request",
            &[("Content-Type", "application/json")],
            serde_json::json!({"error":{"code":"edit_request_decode","message":error.to_string()}})
                .to_string()
                .as_bytes(),
        ),
        Err(valle_project::revision::EditTimelineServiceError::Store(error)) => write_simple(
            out,
            500,
            "Internal Server Error",
            &[("Content-Type", "application/json")],
            serde_json::json!({"error":{"code":"project_store","message":error.to_string()}})
                .to_string()
                .as_bytes(),
        ),
    }
}

fn request_matches_project(raw_path: &str, project: &ProjectStudioCtx) -> bool {
    query_value(raw_path, "project").as_deref() == Some(project.project_id.as_str())
}

fn query_value(raw_path: &str, name: &str) -> Option<String> {
    raw_path.split_once('?')?.1.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(key) == name).then(|| percent_decode(value))
    })
}

/// Verify token and Origin, enforce the command allowlist, then return the kernel report.
/// Authorization failures use HTTP 403; command failures use HTTP 200 with an error envelope.
fn serve_library_execute(
    out: &mut TcpStream,
    headers: &[(String, String)],
    body: &str,
    state: &Arc<StudioHost>,
) -> Result<()> {
    let Some(lib) = &state.library else {
        return write_simple(out, 404, "Not Found", &[], b"library mode off");
    };
    let token = header_value(headers, "x-valle-token").unwrap_or("");
    let origin = header_value(headers, "origin").or_else(|| header_value(headers, "host"));
    if !local_request_authorized(token, &lib.token, origin) {
        return write_simple(
            out,
            403,
            "Forbidden",
            &[("Content-Type", "application/json")],
            br#"{"ok":false,"error":{"code":"refused","message":"token/origin check failed"}}"#,
        );
    }

    let report = match serde_json::from_str::<valle_project::assets::Verb>(body) {
        Ok(verb) if !library_verb_allowed(&verb) => valle_project::assets::Report::from_error(
            valle_project::assets::AssetsError::refused(
                "this command is unavailable in the web interface",
            )
            .with_hint("use the CLI for SQL and purging assets"),
        ),
        // Start analysis in the background and report progress and completion over SSE. An atomic
        // guard rejects concurrent runs.
        Ok(verb @ valle_project::assets::Verb::Analyze { .. }) => {
            if lib
                .analyzing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                valle_project::assets::Report::from_error(
                    valle_project::assets::AssetsError::locked("analysis is already running")
                        .with_hint("retry after the library completion event"),
                )
            } else {
                let st = Arc::clone(state);
                std::thread::spawn(move || {
                    let lib = st.library.as_ref().expect("library ctx present");
                    let mut ctx = (lib.make_ctx)();
                    let sse_st = Arc::clone(&st);
                    ctx.progress = Some(Box::new(move |line: &str| {
                        sse_st.sse.broadcast(
                            "library",
                            &serde_json::json!({ "kind": "progress", "line": line }).to_string(),
                        );
                    }));
                    let report = match &lib.run_analyze {
                        Some(f) => f(&ctx, verb),
                        None => valle_project::assets::execute(&ctx, verb),
                    };
                    st.sse.broadcast(
                        "library",
                        &serde_json::json!({ "kind": "done", "ok": report.ok, "report": report })
                            .to_string(),
                    );
                    lib.analyzing.store(false, Ordering::SeqCst);
                });
                valle_project::assets::Report::data(serde_json::json!({ "started": true }))
            }
        }
        Ok(verb) => {
            let ctx = (lib.make_ctx)();
            valle_project::assets::execute(&ctx, verb)
        }
        Err(e) => valle_project::assets::Report::from_error(
            valle_project::assets::AssetsError::bad_query(format!("invalid request body: {e}")),
        ),
    };
    let json = serde_json::to_vec(&report).unwrap_or_default();
    write_simple(
        out,
        200,
        "OK",
        &[
            ("Content-Type", "application/json"),
            ("Cache-Control", "no-store"),
        ],
        &json,
    )
}

/// Resolve a blob by hash prefix, rejecting removed or stale assets. Support byte ranges for media
/// seeking. Reads use the loopback-only host without a token.
fn serve_library_blob(
    out: &mut TcpStream,
    headers: &[(String, String)],
    prefix: &str,
    state: &StudioHost,
) -> Result<()> {
    let Some(lib) = &state.library else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    let ctx = (lib.make_ctx)();
    match valle_project::assets::resolve::resolve(&ctx, prefix) {
        Ok(o) => {
            let path = PathBuf::from(&o.path);
            serve_file(out, headers, &path, content_type_for(&o.path))
        }
        Err(_) => write_simple(out, 404, "Not Found", &[], b""),
    }
}

/// Serve cached thumbnails, redirect image requests to the full blob, or generate video thumbnails
/// when supported. Return 404 when no thumbnail is available.
fn serve_library_thumb(
    out: &mut TcpStream,
    headers: &[(String, String)],
    prefix: &str,
    state: &StudioHost,
) -> Result<()> {
    let Some(lib) = &state.library else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    let ctx = (lib.make_ctx)();
    let Ok(o) = valle_project::assets::resolve::resolve(&ctx, prefix) else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    let hash = o.content_digest.as_hex();
    let cached =
        valle_project::assets::cachefs::cache_path(&ctx.home, "thumbs", &hash, "w320", "png");
    if cached.exists() {
        return serve_file(out, headers, &cached, "image/png");
    }
    let Ok(meta) = valle_project::assets::meta::AssetMeta::load(&ctx.home.meta_path(&hash)) else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    match meta.kind {
        valle_project::assets::AssetKind::Image => {
            let loc = format!("/library/blob/{hash}");
            write_simple(out, 302, "Found", &[("Location", loc.as_str())], b"")
        }
        valle_project::assets::AssetKind::Video => {
            let Some(thumb) = &lib.thumbnail else {
                return write_simple(out, 404, "Not Found", &[], b"");
            };
            let dur_ms = meta.probe.as_ref().and_then(|p| p.duration_ms);
            match thumb(&ctx.home, &hash, Path::new(&o.path), dur_ms) {
                Ok(png) => serve_file(out, headers, &png, "image/png"),
                Err(e) => {
                    eprintln!("library thumbnail generation failed {}: {e}", &hash[..12]);
                    write_simple(out, 404, "Not Found", &[], b"")
                }
            }
        }
        _ => write_simple(out, 404, "Not Found", &[], b""),
    }
}

/// Flush every SSE event immediately. A failed write ends the connection; the next broadcast
/// removes its channel.
fn serve_sse(out: &mut TcpStream, state: &StudioHost) -> Result<()> {
    let rx = state.sse.subscribe();
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nretry: 500\n\n"
    )
    .context("writing sse head")?;
    out.flush().context("flushing sse head")?;
    while let Ok(frame) = rx.recv() {
        out.write_all(frame.as_bytes())
            .context("writing sse frame")?;
        out.flush().context("flushing sse frame")?;
    }
    Ok(())
}

/// Request method, raw path, and headers with lowercase names.
type RequestHead = (String, String, Vec<(String, String)>);

/// Read the request line and headers through the first empty line.
fn read_request_head(reader: &mut BufReader<TcpStream>) -> Result<RequestHead> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("reading request line")?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header).context("reading header")?;
        let header = header.trim_end();
        if n == 0 || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    Ok((method, path, headers))
}

fn read_body(reader: &mut BufReader<TcpStream>, headers: &[(String, String)]) -> Result<String> {
    let len: usize = header_value(headers, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // Bound request bodies while allowing base64 PNG captures.
    anyhow::ensure!(len <= 64 * 1024 * 1024, "request body too large");
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).context("reading body")?;
    String::from_utf8(body).context("request body is not valid UTF-8")
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Persist optional PNG captures and emit the browser report as a structured event.
fn handle_browser_report(body: &str, state: &StudioHost) {
    static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);
    let Ok(mut report) = serde_json::from_str::<serde_json::Value>(body) else {
        eprintln!("Motion Studio browser report (unparsed): {body}");
        return;
    };
    let status = report["status"].as_str().unwrap_or("unknown").to_owned();
    let mut capture_path: Option<String> = None;
    if let Some(png_b64) = report["pngBase64"].as_str() {
        if let Some(dir) = &state.capture_dir {
            use base64::Engine as _;
            match base64::engine::general_purpose::STANDARD.decode(png_b64) {
                Ok(bytes) => {
                    let n = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
                    let path = dir.join(format!("capture-{n:04}-{status}.png"));
                    if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&path, bytes).is_ok()
                    {
                        capture_path = Some(path.display().to_string());
                    }
                }
                Err(err) => eprintln!("Motion Studio capture decode failed: {err}"),
            }
        }
    }
    if let Some(obj) = report.as_object_mut() {
        obj.remove("pngBase64");
    }
    if status != "ok" && report["message"].is_string() {
        eprintln!(
            "Motion Studio browser error: {}",
            report["message"].as_str().unwrap_or_default()
        );
    }
    crate::events::emit(crate::events::EventKind::BrowserReport {
        status,
        message: report
            .get("message")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        generation: report
            .get("generation")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        capture: capture_path
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
        // Include player timing statistics and optional offline audio probe results.
        stats: report
            .get("stats")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        audio: report
            .get("audio")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    });
}

/// Serve files with optional single byte ranges; valid ranges return 206, invalid ranges 416.
fn serve_file(
    out: &mut TcpStream,
    headers: &[(String, String)],
    path: &Path,
    content_type: &str,
) -> Result<()> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return write_simple(out, 404, "Not Found", &[], b"");
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    match header_value(headers, "range").map(|r| parse_byte_range(r, total)) {
        None => {
            write_head(
                out,
                200,
                "OK",
                &[
                    ("Content-Type", content_type),
                    ("Accept-Ranges", "bytes"),
                    ("Content-Length", &total.to_string()),
                ],
            )?;
            std::io::copy(&mut file, out).context("writing file body")?;
            out.flush().context("flushing file body")
        }
        Some(Some((start, end))) => {
            let len = end - start + 1;
            file.seek(SeekFrom::Start(start))
                .with_context(|| format!("seeking {}", path.display()))?;
            write_head(
                out,
                206,
                "Partial Content",
                &[
                    ("Content-Type", content_type),
                    ("Accept-Ranges", "bytes"),
                    ("Content-Range", &format!("bytes {start}-{end}/{total}")),
                    ("Content-Length", &len.to_string()),
                ],
            )?;
            std::io::copy(&mut file.take(len), out).context("writing range body")?;
            out.flush().context("flushing range body")
        }
        Some(None) => write_simple(
            out,
            416,
            "Range Not Satisfiable",
            &[("Content-Range", &format!("bytes */{total}"))],
            b"",
        ),
    }
}

fn serve_bytes(
    out: &mut TcpStream,
    headers: &[(String, String)],
    bytes: &[u8],
    content_type: &str,
) -> Result<()> {
    let total = u64::try_from(bytes.len()).context("runtime asset is too large")?;
    match header_value(headers, "range").map(|range| parse_byte_range(range, total)) {
        None => {
            write_head(
                out,
                200,
                "OK",
                &[
                    ("Content-Type", content_type),
                    ("Accept-Ranges", "bytes"),
                    ("Content-Length", &total.to_string()),
                ],
            )?;
            out.write_all(bytes)
                .context("writing verified runtime body")?;
            out.flush().context("flushing verified runtime body")
        }
        Some(Some((start, end))) => {
            let len = end - start + 1;
            let start = usize::try_from(start).context("runtime range start is too large")?;
            let end = usize::try_from(end).context("runtime range end is too large")?;
            write_head(
                out,
                206,
                "Partial Content",
                &[
                    ("Content-Type", content_type),
                    ("Accept-Ranges", "bytes"),
                    ("Content-Range", &format!("bytes {start}-{end}/{total}")),
                    ("Content-Length", &len.to_string()),
                ],
            )?;
            out.write_all(&bytes[start..=end])
                .context("writing verified runtime range")?;
            out.flush().context("flushing verified runtime range")
        }
        Some(None) => write_simple(
            out,
            416,
            "Range Not Satisfiable",
            &[("Content-Range", &format!("bytes */{total}"))],
            b"",
        ),
    }
}

fn write_head(
    out: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
) -> Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes()).context("writing head")
}

fn write_simple(
    out: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<()> {
    let len = body.len().to_string();
    let mut all: Vec<(&str, &str)> = headers.to_vec();
    all.push(("Content-Length", &len));
    write_head(out, status, reason, &all)?;
    out.write_all(body).context("writing body")?;
    out.flush().context("flushing body")
}

/// Parse one bounded, open-ended, or suffix byte range. Reject malformed or multiple ranges and
/// out-of-bounds starts.
fn parse_byte_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') || total == 0 {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        // Suffix range: the last n bytes.
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return None;
        }
        let start = total.saturating_sub(n);
        return Some((start, total - 1));
    }
    let start: u64 = start_s.parse().ok()?;
    let end: u64 = if end_s.is_empty() {
        total - 1
    } else {
        end_s.parse().ok()?
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end.min(total - 1)))
}

/// Accept only normal relative path components within the asset root.
fn sandboxed_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

pub(crate) fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or_default();
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        // Streaming WebAssembly compilation requires the application/wasm MIME type.
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_fixture(background: &str) -> valle_timeline::Timeline {
        valle_timeline::decode_timeline(&format!(
            r##"{{
              "canvas": {{"width": 640, "height": 360, "fps": 30, "background": "{background}"}},
              "tracks": {{"visual": [{{
                "clips": [{{"start": 0, "duration": 2, "kind": "solid", "color": "#000000ff"}}]
              }}]}}
            }}"##
        ))
        .unwrap()
    }

    fn timeline_json(background: &str) -> String {
        String::from_utf8(valle_timeline::timeline_bytes(&timeline_fixture(background)).unwrap())
            .unwrap()
    }

    fn project_edit_round_trip_with_headers(
        state: &Arc<StudioHost>,
        body: &[u8],
        headers: &str,
    ) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = Arc::clone(state);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &server_state).unwrap();
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let head = format!(
            "POST /timeline/edit?project=http-contract HTTP/1.1\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    fn project_edit_round_trip(state: &Arc<StudioHost>, body: &[u8]) -> Vec<u8> {
        project_edit_round_trip_with_headers(
            state,
            body,
            "Host: localhost\r\nOrigin: http://localhost\r\nX-Valle-Token: studio-token\r\n",
        )
    }

    #[test]
    fn project_edit_rejects_invalid_utf8_and_never_echoes_requester_sse() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ProjectStore::at(temporary.path());
        let project_id = ProjectId::new("http-contract").unwrap();
        let auth = AuthenticatedContext::new(
            valle_project::revision::Actor::new("studio:http-contract").unwrap(),
        );
        let base_json = timeline_json("#000000ff");
        let candidate_json = timeline_json("#14532dff");
        let rejected_json = r##"{
          "canvas": {"width": 640, "height": 360, "fps": 30},
          "tracks": {"visual":[{"clips":[{"start":0,"duration":2,"kind":"video","src":"missing"}]}]}
        }"##;
        let base = timeline_fixture("#000000ff");
        let genesis = store
            .create_project(&project_id, &base, None, &auth)
            .unwrap();
        let base_revision = genesis.revision().revision;
        let state = Arc::new(StudioHost {
            runtime_files: BTreeMap::new(),
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: None,
            project: Some(ProjectStudioCtx {
                token: "studio-token".to_owned(),
                project_id,
                initial_revision: base_revision,
                store,
                auth,
            }),
            last_report: RwLock::new(None),
        });
        let events = state.sse.subscribe();

        let snapshot: serde_json::Value =
            serde_json::from_str(&project_snapshot_json(state.project.as_ref().unwrap()).unwrap())
                .unwrap();
        assert!(snapshot["timeline"].get("version").is_none());
        assert!(snapshot["render"]["timeline"]["document"].is_object());
        assert!(snapshot.get("resourceManifest").is_none());
        assert!(snapshot["render"].get("resourceManifest").is_none());
        assert!(snapshot["timelineRevision"].get("documentHash").is_none());
        assert!(
            snapshot["timelineRevision"]
                .get("resourceManifestHash")
                .is_none()
        );

        let invalid = project_edit_round_trip(&state, b"{\"timeline\":\xff}");
        let invalid = String::from_utf8(invalid).unwrap();
        assert!(invalid.starts_with("HTTP/1.1 400 Bad Request"), "{invalid}");
        assert!(invalid.contains("invalid_request_body"), "{invalid}");

        let request = |timeline: &str| {
            format!("{{\"baseRevision\":{base_revision},\"timeline\":{timeline}}}",)
        };
        for (timeline, expected) in [
            (candidate_json.as_str(), "\"outcome\":\"committed\""),
            (rejected_json, "\"outcome\":\"rejected\""),
            (base_json.as_str(), "\"outcome\":\"staleBase\""),
        ] {
            let response = project_edit_round_trip(&state, request(timeline).as_bytes());
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
            assert!(response.contains(expected), "{response}");
            assert_eq!(
                events.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty),
                "the requester's HTTP outcome must not be reflected as an external update"
            );
        }
    }

    #[test]
    fn project_edit_requires_the_bound_token_and_local_origin() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ProjectStore::at(temporary.path());
        let project_id = ProjectId::new("http-contract").unwrap();
        let auth = AuthenticatedContext::new(
            valle_project::revision::Actor::new("studio:http-contract").unwrap(),
        );
        let timeline = timeline_fixture("#000000ff");
        let genesis = store
            .create_project(&project_id, &timeline, None, &auth)
            .unwrap();
        let state = Arc::new(StudioHost {
            runtime_files: BTreeMap::new(),
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: None,
            project: Some(ProjectStudioCtx {
                token: "studio-token".to_owned(),
                project_id,
                initial_revision: genesis.revision().revision,
                store,
                auth,
            }),
            last_report: RwLock::new(None),
        });

        for (label, headers) in [
            (
                "wrong token",
                "Host: localhost\r\nOrigin: http://localhost\r\nX-Valle-Token: wrong-token\r\n",
            ),
            (
                "non-local Origin",
                "Host: localhost\r\nOrigin: https://attacker.example\r\nX-Valle-Token: studio-token\r\n",
            ),
            (
                "missing token",
                "Host: localhost\r\nOrigin: http://localhost\r\n",
            ),
            ("missing Origin and Host", "X-Valle-Token: studio-token\r\n"),
        ] {
            let response = project_edit_round_trip_with_headers(&state, b"{}", headers);
            let response = String::from_utf8(response).unwrap();
            assert!(
                response.starts_with("HTTP/1.1 403 Forbidden"),
                "{label} must be forbidden: {response}"
            );
            assert!(
                response.contains("token/origin check failed"),
                "{label}: {response}"
            );
        }
    }

    #[test]
    fn studio_boot_marks_motion_file_without_frontend_guessing() {
        let boot = studio_boot_from_config(
            r#"{
              "generation": 4,
              "input": "card.motion.tsx",
              "runtimeAssets": {"engine": {"wasm": "engine.wasm"}}
            }"#,
        )
        .unwrap();
        let boot: serde_json::Value = serde_json::from_str(&boot).unwrap();
        assert_eq!(boot["session"]["kind"], "motion-file");
        assert_eq!(boot["session"]["generation"], 4);
        assert_eq!(boot["capabilities"]["editMotionProps"], true);
        assert_eq!(boot["capabilities"]["writeMotionSource"], false);
        assert_eq!(boot["runtime"]["assetUrls"]["engineWasm"], "engine.wasm");
    }

    /// Verify library authentication and the command allowlist.
    #[test]
    fn library_execute_enforces_token_and_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let assets_home = valle_project::assets::Home::at(tmp.path());
        let token = "tok-lib".to_owned();
        let factory_home = assets_home.clone();
        let state = Arc::new(StudioHost {
            runtime_files: BTreeMap::new(),
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: Some(LibraryCtx {
                token: token.clone(),
                make_ctx: Box::new(move || {
                    valle_project::assets::Ctx::new(
                        factory_home.clone(),
                        Box::new(valle_project::assets::NoProber),
                    )
                }),
                thumbnail: None,
                analyzing: AtomicBool::new(false),
                run_analyze: None,
            }),
            project: None,
            last_report: RwLock::new(None),
        });
        let (listener, addr) = bind(0).unwrap();
        std::thread::spawn(move || {
            let _ = serve_forever(listener, state);
        });
        let post = |headers: &str, body: &str| -> (u16, String) {
            let mut s = TcpStream::connect(addr).unwrap();
            let req = format!(
                "POST /library/execute HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            s.read_to_string(&mut resp).unwrap();
            let status: u16 = resp
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            (
                status,
                resp.split("\r\n\r\n").nth(1).unwrap_or("").to_owned(),
            )
        };
        let auth = format!("Origin: http://localhost\r\nX-Valle-Token: {token}\r\n");

        // Missing token returns 403.
        let (st, _) = post("Origin: http://localhost\r\n", r#"{"verb":"describe"}"#);
        assert_eq!(st, 403);

        // SQL and permanent deletion return a refusal envelope.
        let (st, body) = post(&auth, r#"{"verb":"sql","query":"select 1"}"#);
        assert_eq!(st, 200);
        assert!(
            body.contains("refused"),
            "SQL must be rejected by the allowlist: {body}"
        );
        let (st, body) = post(&auth, r#"{"verb":"rm","hash":"abc123","purge":true}"#);
        assert_eq!(st, 200);
        assert!(
            body.contains("refused"),
            "purge must be rejected by the allowlist: {body}"
        );

        // Allowed commands return successful reports.
        let (st, body) = post(&auth, r#"{"verb":"describe"}"#);
        assert_eq!(st, 200, "body={body}");
        assert!(
            body.contains("\"ok\":true"),
            "describe must succeed: {body}"
        );

        // Non-purge removal reaches the kernel and reports a missing hash.
        let (st, body) = post(&auth, r#"{"verb":"rm","hash":"deadbeef00"}"#);
        assert_eq!(st, 200);
        assert!(
            body.contains("\"ok\":false") && body.contains("not_found"),
            "removal without purge must reach the core: {body}"
        );
    }

    /// Verify blob ranges and thumbnail cache, redirect, and unavailable paths.
    #[test]
    fn library_blob_and_thumb_routes() {
        let tmp = tempfile::tempdir().unwrap();
        let assets_home = valle_project::assets::Home::at(tmp.path());
        // Register synthetic image and video files with a stub prober.
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let png_path = src_dir.join("pic.png");
        std::fs::write(&png_path, b"fake-png-bytes-0123456789").unwrap();
        let mp4_path = src_dir.join("clip.mp4");
        std::fs::write(&mp4_path, b"fake-mp4").unwrap();
        let ctx = valle_project::assets::Ctx::new(
            assets_home.clone(),
            Box::new(valle_project::assets::NoProber),
        );
        let seed = |p: &Path| -> String {
            let r = valle_project::assets::execute(
                &ctx,
                valle_project::assets::Verb::Add {
                    path: p.to_string_lossy().into_owned(),
                    mode: valle_project::assets::AddMode::Copy,
                    kind: None,
                    title: None,
                    tags: vec![],
                },
            );
            assert!(r.ok, "seed add: {:?}", r.error);
            valle_project::ContentDigest::parse(r.data.unwrap()["content_digest"].as_str().unwrap())
                .unwrap()
                .as_hex()
        };
        let png_hash = seed(&png_path);
        let mp4_hash = seed(&mp4_path);

        let factory_home = assets_home.clone();
        let state = Arc::new(StudioHost {
            runtime_files: BTreeMap::new(),
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: Some(LibraryCtx {
                token: "t".to_owned(),
                make_ctx: Box::new(move || {
                    valle_project::assets::Ctx::new(
                        factory_home.clone(),
                        Box::new(valle_project::assets::NoProber),
                    )
                }),
                thumbnail: None,
                analyzing: AtomicBool::new(false),
                run_analyze: None,
            }),
            project: None,
            last_report: RwLock::new(None),
        });
        let (listener, addr) = bind(0).unwrap();
        std::thread::spawn(move || {
            let _ = serve_forever(listener, state);
        });
        let get = |path: &str, extra: &str| -> (u16, String, String) {
            let mut s = TcpStream::connect(addr).unwrap();
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n"
            );
            s.write_all(req.as_bytes()).unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).unwrap();
            let resp = String::from_utf8_lossy(&resp).into_owned();
            let (head, body) = resp.split_once("\r\n\r\n").unwrap_or((resp.as_str(), ""));
            let status: u16 = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            (status, head.to_owned(), body.to_owned())
        };

        // Resolve a hash prefix for full and partial reads.
        let (st, _, body) = get(&format!("/library/blob/{}", &png_hash[..8]), "");
        assert_eq!(st, 200);
        assert_eq!(body.as_bytes(), b"fake-png-bytes-0123456789");
        let (st, head, body) = get(
            &format!("/library/blob/{}", &png_hash[..8]),
            "Range: bytes=0-3\r\n",
        );
        assert_eq!(st, 206, "head={head}");
        assert_eq!(body.as_bytes(), b"fake");

        // Unknown hash returns 404.
        let (st, _, _) = get("/library/blob/ffffffffff", "");
        assert_eq!(st, 404);

        // Image thumbnails redirect to the blob's full hash.
        let (st, head, _) = get(&format!("/library/thumb/{}", &png_hash[..8]), "");
        assert_eq!(st, 302, "head={head}");
        assert!(
            head.contains(&format!("/library/blob/{png_hash}")),
            "Location must point to the full-hash blob: {head}"
        );

        // Video thumbnails require an extractor or a cached result.
        let (st, _, _) = get(&format!("/library/thumb/{}", &mp4_hash[..8]), "");
        assert_eq!(st, 404);
        let cache = valle_project::assets::cachefs::ensure_cache_path(
            &assets_home,
            "thumbs",
            &mp4_hash,
            "w320",
            "png",
        )
        .unwrap();
        std::fs::write(&cache, b"png-bytes").unwrap();
        let (st, _, body) = get(&format!("/library/thumb/{}", &mp4_hash[..8]), "");
        assert_eq!(st, 200);
        assert_eq!(body.as_bytes(), b"png-bytes");
    }

    /// Verify immediate analysis acknowledgement, SSE completion, and concurrent-run rejection.
    #[test]
    fn library_analyze_runs_background_with_sse_and_single_flight() {
        /// Delay the mock analyzer to expose the concurrent-run guard.
        struct SlowMock;
        impl valle_project::assets::analysis::Analyzer for SlowMock {
            fn name(&self) -> &'static str {
                "mock"
            }
            fn version(&self) -> u32 {
                1
            }
            fn accepts(&self, _kind: valle_project::assets::AssetKind) -> bool {
                true
            }
            fn analyze(
                &self,
                _input: &valle_project::assets::analysis::AnalyzerInput<'_>,
            ) -> valle_project::assets::Result<valle_project::assets::analysis::AnalyzerOutput>
            {
                std::thread::sleep(std::time::Duration::from_millis(400));
                Ok(valle_project::assets::analysis::AnalyzerOutput {
                    items: vec![serde_json::json!({"start_ms": 0, "end_ms": 1000, "note": "m"})],
                    ..Default::default()
                })
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let assets_home = valle_project::assets::Home::at(tmp.path());
        let seed_ctx = valle_project::assets::Ctx::new(
            assets_home.clone(),
            Box::new(valle_project::assets::NoProber),
        );
        let f = tmp.path().join("a.mp4");
        std::fs::write(&f, b"bytes").unwrap();
        let r = valle_project::assets::execute(
            &seed_ctx,
            valle_project::assets::Verb::Add {
                path: f.to_string_lossy().into_owned(),
                mode: valle_project::assets::AddMode::Copy,
                kind: None,
                title: None,
                tags: vec![],
            },
        );
        let hash = valle_project::ContentDigest::parse(
            r.data.unwrap()["content_digest"].as_str().unwrap(),
        )
        .unwrap()
        .as_hex();

        let factory_home = assets_home.clone();
        let state = Arc::new(StudioHost {
            runtime_files: BTreeMap::new(),
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: Some(LibraryCtx {
                token: "t".to_owned(),
                make_ctx: Box::new(move || {
                    let mut ctx = valle_project::assets::Ctx::new(
                        factory_home.clone(),
                        Box::new(valle_project::assets::NoProber),
                    );
                    ctx.analyzers = vec![Box::new(SlowMock)];
                    ctx
                }),
                thumbnail: None,
                analyzing: AtomicBool::new(false),
                run_analyze: None,
            }),
            project: None,
            last_report: RwLock::new(None),
        });
        let (listener, addr) = bind(0).unwrap();
        std::thread::spawn(move || {
            let _ = serve_forever(listener, state);
        });

        // Subscribe before triggering analysis.
        let mut sse = TcpStream::connect(addr).unwrap();
        sse.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        sse.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();

        let post = |body: &str| -> String {
            let mut s = TcpStream::connect(addr).unwrap();
            let req = format!(
                "POST /library/execute HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nX-Valle-Token: t\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            s.read_to_string(&mut resp).unwrap();
            resp.split("\r\n\r\n").nth(1).unwrap_or("").to_owned()
        };

        // Starting analysis returns immediately.
        let verb = format!(r#"{{"verb":"analyze","hashes":["{hash}"],"with":["mock"]}}"#);
        let body = post(&verb);
        assert!(
            body.contains("\"started\":true"),
            "must immediately return started: {body}"
        );

        // A second run is rejected while the first is active.
        let body = post(&verb);
        assert!(
            body.contains("locked"),
            "a concurrent second request must be locked: {body}"
        );

        // Expect progress and successful completion events.
        let mut sse_text = String::new();
        let mut buf = [0u8; 4096];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match sse.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    sse_text.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if sse_text.contains("\"kind\":\"done\"") {
                        break;
                    }
                }
                Err(_) => break,
            }
            if std::time::Instant::now() > deadline {
                break;
            }
        }
        assert!(
            sse_text.contains("event: library") && sse_text.contains("\"kind\":\"progress\""),
            "must receive a progress event: {sse_text}"
        );
        assert!(
            sse_text.contains("\"kind\":\"done\"") && sse_text.contains("\"ok\":true"),
            "must receive successful completion: {sse_text}"
        );

        // The guard is released after analysis completes.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let body = post(&verb);
        assert!(
            body.contains("\"started\":true"),
            "must accept another request after completion: {body}"
        );

        // Verify the completed analysis is persisted and reusable.
        let show = post(&format!(r#"{{"verb":"show","hash":"{hash}"}}"#));
        assert!(
            show.contains("\"analyzer\":\"mock\""),
            "show must include the analysis slot: {show}"
        );
    }

    #[test]
    fn verified_runtime_bytes_are_served_with_range_without_reopening_a_path() {
        let mut runtime_files = BTreeMap::new();
        runtime_files.insert(
            "preview.html".to_owned(),
            crate::webruntime::HostedFile::VerifiedRuntime(Arc::from(&b"verified-runtime"[..])),
        );
        let state = Arc::new(StudioHost {
            runtime_files,
            assets_dir: None,
            config_json: Arc::new(RwLock::new("{}".to_owned())),
            sse: SseBroadcaster::default(),
            capture_dir: None,
            library: None,
            project: None,
            last_report: RwLock::new(None),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = Arc::clone(&state);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &server_state).unwrap();
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .write_all(
                b"GET /preview.html HTTP/1.1\r\nRange: bytes=9-15\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 206 Partial Content"));
        assert!(response.contains("Content-Range: bytes 9-15/16"));
        assert!(response.ends_with("runtime"));
    }

    #[test]
    fn byte_range_parsing_covers_forms_and_rejects_invalid() {
        assert_eq!(parse_byte_range("bytes=0-3", 10), Some((0, 3)));
        assert_eq!(parse_byte_range("bytes=4-", 10), Some((4, 9)));
        assert_eq!(parse_byte_range("bytes=-3", 10), Some((7, 9)));
        // Clamp the end offset to EOF.
        assert_eq!(parse_byte_range("bytes=8-99", 10), Some((8, 9)));
        // Reject invalid starts, reversed or multiple ranges, and empty files.
        assert_eq!(parse_byte_range("bytes=10-", 10), None);
        assert_eq!(parse_byte_range("bytes=5-2", 10), None);
        assert_eq!(parse_byte_range("bytes=0-1,3-4", 10), None);
        assert_eq!(parse_byte_range("bytes=0-", 0), None);
    }

    #[test]
    fn asset_sandbox_blocks_escape() {
        let root = Path::new("/srv/assets");
        assert_eq!(
            sandboxed_join(root, "a/b.png"),
            Some(PathBuf::from("/srv/assets/a/b.png"))
        );
        assert_eq!(sandboxed_join(root, "../etc/passwd"), None);
        assert_eq!(sandboxed_join(root, "/etc/passwd"), None);
        assert_eq!(sandboxed_join(root, "a/../../x"), None);
    }

    #[test]
    fn percent_decode_handles_encoded_and_passthrough() {
        assert_eq!(percent_decode("/assets/a%20b.png"), "/assets/a b.png");
        assert_eq!(percent_decode("/assets/plain.png"), "/assets/plain.png");
        assert_eq!(percent_decode("/a%zz"), "/a%zz");
    }

    #[test]
    fn sse_broadcast_drops_disconnected_clients() {
        let sse = SseBroadcaster::default();
        let rx = sse.subscribe();
        sse.broadcast("reload", "{\"generation\":2}");
        assert_eq!(
            rx.recv().unwrap(),
            "event: reload\ndata: {\"generation\":2}\n\n"
        );
        drop(rx);
        // Remove disconnected clients without panicking.
        sse.broadcast("reload", "{}");
        assert!(sse.clients.lock().unwrap().is_empty());
    }
}
