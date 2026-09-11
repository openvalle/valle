use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use valle_project::revision::{Actor, AuthenticatedContext, ProjectId, ProjectStore};
use valle_timeline::decode_timeline;

const PROJECT_ID: &str = "edit-entry-consistency";
const INTENT: &str = "multi-entry acceptance";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/timeline")
        .join(name)
}

fn request_json(base_revision: &u64, timeline_path: &Path, intent: &str) -> String {
    let timeline = fs::read_to_string(timeline_path).unwrap();
    format!(
        "{{\"baseRevision\":{},\"timeline\":{},\"intent\":{}}}",
        base_revision,
        timeline,
        serde_json::to_string(intent).unwrap(),
    )
}

fn assert_committed_response(response: &Value) {
    assert_eq!(response["outcome"], "committed", "{response}");
    assert_eq!(response["revision"], 2, "{response}");
    for hidden in [
        "contract",
        "documentHash",
        "resourceManifestHash",
        "diffSummary",
    ] {
        assert!(
            response.get(hidden).is_none(),
            "{hidden} leaked into {response}"
        );
    }
}

fn response_diagnostics(response: &Value) -> &Vec<Value> {
    response["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("response has no diagnostics: {response}"))
}

fn seed_project(home: &Path) -> u64 {
    let project_id = ProjectId::new(PROJECT_ID).unwrap();
    let store = ProjectStore::at(home);
    let auth = AuthenticatedContext::new(Actor::new("agent:seed-acceptance").unwrap());
    let timeline =
        decode_timeline(&fs::read_to_string(fixture("base.timeline.json")).unwrap()).unwrap();
    store
        .create_project(&project_id, &timeline, None, &auth)
        .unwrap()
        .revision()
        .revision
}

fn run_direct(home: &Path, base: &u64, timeline: &Path) -> Value {
    let project_id = ProjectId::new(PROJECT_ID).unwrap();
    let store = ProjectStore::at(home);
    let auth = AuthenticatedContext::new(Actor::new("agent:sdk-acceptance").unwrap());
    serde_json::to_value(
        store
            .edit_timeline_json(&project_id, &request_json(base, timeline, INTENT), &auth)
            .unwrap(),
    )
    .unwrap()
}

fn run_cli(home: &Path, base: &u64, timeline: &Path, expected_success: bool) -> Value {
    let base = base.to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .args([
            "project",
            "--json",
            "apply",
            PROJECT_ID,
            "--base-revision",
            &base,
            "--timeline",
            timeline.to_str().unwrap(),
            "--intent",
            INTENT,
        ])
        .output()
        .expect("run Timeline CLI adapter");
    assert_eq!(
        output.status.success(),
        expected_success,
        "CLI status/output mismatch: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("CLI must emit the typed edit response")
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn make_runtime(root: &Path) -> PathBuf {
    let runtime = root.join("web-runtime");
    let html_path = "runtime/apps/studio.html";
    let html = b"<!doctype html><title>Studio acceptance</title>";
    let js_path = "runtime/apps/studio.js";
    let specs: [(&str, &str, &str, &[u8], Option<&str>); 8] = [
        ("studio-html", "html", html_path, html, None),
        ("studio-js", "app", js_path, b"void 0", None),
        (
            "engine-glue",
            "glue",
            valle_cli::webruntime::ENGINE_GLUE_PATH,
            b"engine glue",
            Some("engine-core"),
        ),
        (
            "engine-wasm",
            "wasm",
            valle_cli::webruntime::ENGINE_WASM_PATH,
            b"engine wasm",
            Some("engine-core"),
        ),
        (
            "canvas-full-glue",
            "glue",
            valle_cli::webruntime::CANVASKIT_FULL_GLUE_PATH,
            b"canvas full glue",
            Some("canvaskit-full"),
        ),
        (
            "canvas-full-wasm",
            "wasm",
            valle_cli::webruntime::CANVASKIT_FULL_WASM_PATH,
            b"canvas full wasm",
            Some("canvaskit-full"),
        ),
        (
            "font",
            "font",
            valle_cli::webruntime::DEFAULT_SANS_FONT_PATH,
            b"font",
            None,
        ),
        (
            "worker",
            "worker",
            valle_cli::webruntime::PRODUCT_FRAME_WORKER_PATH,
            b"worker",
            None,
        ),
    ];
    let mut assets = Vec::new();
    for (id, role, path, bytes, group) in specs {
        let output = runtime.join(path);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(output, bytes).unwrap();
        let mut asset = json!({
            "id": id,
            "owner": "acceptance",
            "role": role,
            "path": path,
            "bytes": bytes.len(),
            "sha256": sha256(bytes),
            "license": "Apache-2.0"
        });
        if let Some(group) = group {
            asset["group"] = json!(group);
        }
        assets.push(asset);
    }
    let package_path = runtime.join("runtime/manifests/acceptance.json");
    fs::create_dir_all(package_path.parent().unwrap()).unwrap();
    fs::write(
        &package_path,
        serde_json::to_vec(&json!({
            "schemaVersion":2,
            "package":"acceptance",
            "assets":assets.clone()
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        runtime.join("runtime/manifest.json"),
        serde_json::to_vec(&json!({
            "schemaVersion":2,
            "runtimeVersion":env!("CARGO_PKG_VERSION"),
            "protocolVersion":valle_cli::webruntime::PROTOCOL_VERSION,
            "packages":[{"id":"acceptance","manifest":"runtime/manifests/acceptance.json"}],
            "assets":assets,
            "assetGroups":[
                {
                    "id":"engine-core",
                    "glue":valle_cli::webruntime::ENGINE_GLUE_PATH,
                    "wasm":[valle_cli::webruntime::ENGINE_WASM_PATH]
                },
                {
                    "id":"canvaskit-full",
                    "glue":valle_cli::webruntime::CANVASKIT_FULL_GLUE_PATH,
                    "wasm":[valle_cli::webruntime::CANVASKIT_FULL_WASM_PATH]
                }
            ],
            "workers":[{
                "id":"product-frame",
                "path":valle_cli::webruntime::PRODUCT_FRAME_WORKER_PATH
            }],
            "apps":[{"id":"studio","html":html_path}],
            "routes":{"studio.html":html_path,"studio.js":js_path},
            "runtimeAssets":valle_cli::webruntime::runtime_assets_json()
        }))
        .unwrap(),
    )
    .unwrap();
    runtime
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_studio(home: &Path, runtime: &Path, base: &u64, timeline: &Path, intent: &str) -> Value {
    let base = base.to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .args([
            "project",
            "--json",
            "studio",
            PROJECT_ID,
            "--web-assets-dir",
            runtime.to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Project Studio host");
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut ready = String::new();
    reader
        .read_line(&mut ready)
        .expect("read Project Studio ready line");
    let ready: Value = serde_json::from_str(&ready).unwrap_or_else(|error| {
        panic!("Project Studio did not emit ready JSON ({error}): {ready:?}")
    });
    let guard = ChildGuard(child);
    let origin = format!("http://127.0.0.1:{}", ready["port"].as_u64().unwrap());
    let output = Command::new("bun")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([
            "web/apps/studio/src/edit-timeline-consistency.runner.ts",
            &origin,
            PROJECT_ID,
            &base,
            timeline.to_str().unwrap(),
            intent,
        ])
        .output()
        .expect("run Studio host/save adapter");
    assert!(
        output.status.success(),
        "Studio adapter failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    drop(guard);
    serde_json::from_slice(&output.stdout).expect("Studio runner must emit JSON")
}

fn browser_authoring_required() -> bool {
    std::env::var_os("VALLE_WEB_PLAYER_REQUIRE_BROWSER").is_some()
}

fn assert_chromium_evidence(report: &Value) {
    let user_agent = report["userAgent"]
        .as_str()
        .unwrap_or_else(|| panic!("Project Studio browser report has no userAgent: {report}"));
    assert!(
        user_agent.contains("Chrome/") || user_agent.contains("Chromium/"),
        "Project Studio authoring did not run in Chrome/Chromium: {user_agent}"
    );

    let Some(binary) = std::env::var_os("VALLE_WEB_PARITY_BROWSER") else {
        eprintln!("Chrome acceptance evidence: binary=<auto-discovered> userAgent={user_agent:?}");
        return;
    };
    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("read explicit browser version from {binary:?}: {error}"));
    assert!(
        output.status.success(),
        "explicit browser version probe failed for {binary:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_owned();
    assert!(
        version.contains("Chrome") || version.contains("Chromium"),
        "explicit browser version is not Chrome/Chromium: {version:?}"
    );
    eprintln!(
        "Chrome acceptance evidence: binary={} version={version:?} userAgent={user_agent:?}",
        Path::new(&binary).display()
    );
}

fn web_runtime_dir() -> PathBuf {
    std::env::var_os("VALLE_WEB_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"))
}

fn load_project_studio_boot(port: u16) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to Project Studio");
    write!(
        stream,
        "GET /studio/boot.json?project={PROJECT_ID} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("Project Studio boot response body");
    serde_json::from_str(body).expect("Project Studio boot JSON")
}

fn run_project_studio_authoring_browser(home: &Path) -> Option<Value> {
    if !browser_authoring_required() {
        eprintln!(
            "skip Project Studio authoring browser gate: VALLE_WEB_PLAYER_REQUIRE_BROWSER is unset"
        );
        return None;
    }
    let runtime = web_runtime_dir();
    if !runtime.join("runtime/manifest.json").is_file() {
        panic!(
            "Project Studio authoring browser gate requires a built Web runtime at {}",
            runtime.display()
        );
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", home)
        .args([
            "project",
            "--json",
            "studio",
            PROJECT_ID,
            "--web-assets-dir",
        ])
        .arg(&runtime)
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real Project Studio host");
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut ready = String::new();
    reader
        .read_line(&mut ready)
        .expect("read real Project Studio ready line");
    let ready: Value = serde_json::from_str(&ready).unwrap_or_else(|error| {
        panic!("Project Studio did not emit ready JSON ({error}): {ready:?}")
    });
    let guard = ChildGuard(child);
    let port = ready["port"].as_u64().unwrap() as u16;
    let origin = format!("http://127.0.0.1:{port}");
    let boot = load_project_studio_boot(port);
    let token = boot["session"]["token"]
        .as_str()
        .expect("Project Studio boot session token");
    assert_eq!(boot["session"]["projectId"], PROJECT_ID);
    for name in [
        "engineGlue",
        "engineWasm",
        "canvasKitFullGlue",
        "canvasKitFullWasm",
        "defaultSansFont",
        "productFrameWorker",
    ] {
        assert!(
            boot["runtime"]["assetUrls"][name]
                .as_str()
                .is_some_and(|url| !url.is_empty()),
            "Project Studio boot is missing runtime asset URL {name}: {boot}"
        );
    }
    let request_dir = tempfile::tempdir().unwrap();
    let request_path = request_dir.path().join("project-studio-authoring.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&json!({
            "protocolVersion": 1,
            "caseId": "project-studio-production-authoring",
            "page": "studio.html",
            "serverUrl": origin,
            "pageQuery": format!("project={PROJECT_ID}&authoringSmoke={token}"),
            "requireBrowser": true,
            "timeoutMs": 60_000,
            "captures": [],
        }))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new("bun")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args(["web/tests/parity/cli.ts", "browser", "--request"])
        .arg(&request_path)
        .output()
        .expect("run real Project Studio browser authoring gate");
    drop(guard);
    assert!(
        output.status.success(),
        "Project Studio browser authoring failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "Project Studio browser output was not JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    if report["status"] == "skipped" {
        panic!("required Project Studio browser gate was skipped: {report}");
    }
    Some(report)
}

#[test]
fn direct_cli_and_studio_share_sparse_full_document_outcomes() {
    let base_path = fixture("base.timeline.json");
    let candidate_path = fixture("candidate.timeline.json");
    let rejected_path = fixture("rejected.timeline.json");

    let direct_success_home = tempfile::tempdir().unwrap();
    let direct_success_base = seed_project(direct_success_home.path());
    let direct_success = run_direct(
        direct_success_home.path(),
        &direct_success_base,
        &candidate_path,
    );
    let direct_rejected_home = tempfile::tempdir().unwrap();
    let direct_rejected_base = seed_project(direct_rejected_home.path());
    let direct_rejected = run_direct(
        direct_rejected_home.path(),
        &direct_rejected_base,
        &rejected_path,
    );
    assert_committed_response(&direct_success);
    assert_eq!(direct_rejected["outcome"], "rejected");
    assert_eq!(
        response_diagnostics(&direct_rejected)
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["invalid_canvas_width", "time_must_be_positive"],
    );
    let expected_diagnostics = response_diagnostics(&direct_rejected).clone();

    let cli_success_home = tempfile::tempdir().unwrap();
    let cli_success_base = seed_project(cli_success_home.path());
    let cli_success = run_cli(
        cli_success_home.path(),
        &cli_success_base,
        &candidate_path,
        true,
    );
    let cli_rejected_home = tempfile::tempdir().unwrap();
    let cli_rejected_base = seed_project(cli_rejected_home.path());
    let cli_rejected = run_cli(
        cli_rejected_home.path(),
        &cli_rejected_base,
        &rejected_path,
        false,
    );
    assert_committed_response(&cli_success);
    assert_eq!(cli_rejected["outcome"], "rejected");
    assert_eq!(response_diagnostics(&cli_rejected), &expected_diagnostics);

    let studio_success_home = tempfile::tempdir().unwrap();
    let studio_success_base = seed_project(studio_success_home.path());
    let studio_success = run_studio(
        studio_success_home.path(),
        &make_runtime(studio_success_home.path()),
        &studio_success_base,
        &candidate_path,
        INTENT,
    );
    assert_eq!(studio_success["boot"]["session"]["projectId"], PROJECT_ID);
    assert_eq!(
        studio_success["boot"]["session"]["revision"],
        studio_success_base
    );
    assert_eq!(
        studio_success["snapshot"]["preview"]["status"],
        "unavailable"
    );
    assert!(
        studio_success["snapshot"]["timeline"]
            .get("version")
            .is_none()
    );
    assert!(studio_success["snapshot"].get("resourceManifest").is_none());
    assert!(
        studio_success["snapshot"]["render"]
            .get("resourceManifest")
            .is_none()
    );
    assert_committed_response(&studio_success["response"]);
    assert_eq!(
        studio_success["reloaded"]["timelineRevision"]["revision"],
        studio_success["response"]["revision"]
    );
    assert_eq!(
        studio_success["reloaded"]["timeline"],
        serde_json::from_slice::<Value>(&fs::read(&candidate_path).unwrap()).unwrap()
    );
    assert!(
        studio_success["reloaded"]["timelineRevision"]
            .get("documentHash")
            .is_none()
    );
    assert!(studio_success["reloaded"].get("resourceManifest").is_none());
    assert!(
        studio_success["reloaded"]["render"]
            .get("resourceManifest")
            .is_none()
    );

    let studio_rejected_home = tempfile::tempdir().unwrap();
    let studio_rejected_base = seed_project(studio_rejected_home.path());
    let studio_rejected = run_studio(
        studio_rejected_home.path(),
        &make_runtime(studio_rejected_home.path()),
        &studio_rejected_base,
        &rejected_path,
        INTENT,
    );
    assert_eq!(studio_rejected["response"]["outcome"], "rejected");
    assert_eq!(
        response_diagnostics(&studio_rejected["response"]),
        &expected_diagnostics
    );
    assert_eq!(
        studio_rejected["reloaded"]["timelineRevision"]["revision"],
        studio_rejected_base
    );

    let studio_stale_home = tempfile::tempdir().unwrap();
    let studio_stale_base = seed_project(studio_stale_home.path());
    let studio_stale_advance = run_direct(
        studio_stale_home.path(),
        &studio_stale_base,
        &candidate_path,
    );
    let studio_stale = run_studio(
        studio_stale_home.path(),
        &make_runtime(studio_stale_home.path()),
        &studio_stale_base,
        &base_path,
        "stale acceptance",
    );
    assert_eq!(studio_stale["response"]["outcome"], "staleBase");
    assert_eq!(
        studio_stale["response"]["revision"],
        studio_stale_advance["revision"]
    );

    let browser_home = tempfile::tempdir().unwrap();
    seed_project(browser_home.path());
    if let Some(browser) = run_project_studio_authoring_browser(browser_home.path()) {
        let stats = &browser["stats"];
        assert_eq!(browser["status"], "ok");
        assert_chromium_evidence(&browser);
        assert_eq!(browser["implementation"], "valle-project-studio-authoring");
        assert_eq!(stats["previewAvailable"], false);
        assert_eq!(stats["marker"], "#14532dff");
        assert_ne!(stats["beforeRevision"], stats["afterRevision"]);
        assert!(stats.get("documentHash").is_none());

        let project_id = ProjectId::new(PROJECT_ID).unwrap();
        let stored = ProjectStore::at(browser_home.path())
            .get_timeline(&project_id, None)
            .expect("reload browser-authored Project snapshot");
        assert_eq!(
            stored.revision().revision,
            stats["afterRevision"].as_u64().unwrap()
        );
        let stored_timeline = serde_json::to_value(stored.timeline().to_wire()).unwrap();
        assert_eq!(
            stored_timeline.pointer("/canvas/background"),
            Some(&json!("#14532dff"))
        );
    }
}
