// Studio renders through browser Wasm and requires only the motion feature.

//! Motion Studio acceptance with real Chromium and Wasm. Control edits cannot mutate a pinned package; source reload publishes a new package while retaining the last good result.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const SOURCE: &str = r##"export const controls = defineControls({
  props: { size: number({ default: 40, min: 10, max: 200 }) },
  timing: {
    enterFrames: frames({ default: 3, min: 0 }),
    exitFrames: frames({ default: 2, min: 0 }),
  },
});

export default function Knob(ctx, props) {
  // Animate opacity so phase timing changes are visible in engine output bytes.
  return (
    <Scene key="s" className="relative" style={{ width: "240px", height: "140px" }}>
      <View
        key="box"
        className="absolute"
        style={{ left: "10px", top: "10px", width: `${props.size}px`, height: `${props.size}px`, backgroundColor: "#67e8f9", opacity: interpolate(ctx.enter.progress, [0, 1], [0.2, 1]) }}
      />
    </Scene>
  );
}
"##;

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct BrowserGuard {
    child: Child,
    binary: PathBuf,
    page: String,
}

impl BrowserGuard {
    fn new(child: Child, binary: PathBuf, page: impl Into<String>) -> Self {
        Self {
            child,
            binary,
            page: page.into(),
        }
    }
}

impl Drop for BrowserGuard {
    fn drop(&mut self) {
        terminate_browser(&mut self.child);
    }
}

/// Ask Chrome to shut down cleanly before falling back to `SIGKILL`.
///
/// `Child::kill` is an unconditional `SIGKILL` on Unix. Killing a healthy Chrome that way after a
/// successful smoke run makes macOS report that Chrome "quit unexpectedly", which is both noisy
/// and indistinguishable from a real browser crash to a person watching the gate.
fn terminate_browser(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }

    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let _ = child.kill();
    let _ = child.wait();
}

fn assert_chromium_evidence(report: &serde_json::Value, binary: &Path) {
    let user_agent = report["userAgent"]
        .as_str()
        .or_else(|| {
            report
                .pointer("/stats/userAgent")
                .and_then(|value| value.as_str())
        })
        .unwrap_or_else(|| panic!("browser report has no userAgent: {report}"));
    assert!(
        user_agent.contains("Chrome/") || user_agent.contains("Chromium/"),
        "browser report did not come from Chrome/Chromium: {user_agent}"
    );

    let version_output = Command::new(binary)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("read browser version from {}: {error}", binary.display()));
    assert!(
        version_output.status.success(),
        "browser version probe failed for {}: {}",
        binary.display(),
        String::from_utf8_lossy(&version_output.stderr)
    );
    let version = format!(
        "{}{}",
        String::from_utf8_lossy(&version_output.stdout),
        String::from_utf8_lossy(&version_output.stderr)
    )
    .trim()
    .to_owned();
    assert!(
        version.contains("Chrome") || version.contains("Chromium"),
        "browser version probe is not Chrome/Chromium: {version:?}"
    );
    eprintln!(
        "Chrome acceptance evidence: binary={} version={version:?} userAgent={user_agent:?}",
        binary.display()
    );
}

/// Capture Studio and Chromium output for timeout and startup diagnostics.
struct ChildLogs {
    studio_stderr: PathBuf,
    browser_stdout: PathBuf,
    browser_stderr: PathBuf,
}

impl ChildLogs {
    fn new(dir: &Path) -> Self {
        Self {
            studio_stderr: dir.join("studio-stderr.log"),
            browser_stdout: dir.join("browser-stdout.log"),
            browser_stderr: dir.join("browser-stderr.log"),
        }
    }

    fn sink(path: &Path) -> Stdio {
        Stdio::from(std::fs::File::create(path).expect("create child process log"))
    }

    /// Report empty logs explicitly.
    fn dump(&self) -> String {
        [
            ("studio stderr", &self.studio_stderr),
            ("browser stdout", &self.browser_stdout),
            ("browser stderr", &self.browser_stderr),
        ]
        .iter()
        .map(|(label, path)| {
            let body = match std::fs::read_to_string(path) {
                Ok(text) if text.trim().is_empty() => "(empty)".to_owned(),
                Ok(text) => text,
                Err(_) => "(file missing; process not started)".to_owned(),
            };
            format!("--- {label} ---\n{body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
    }
}

fn temp_dir(prefix: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "{prefix}_{}_{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&base).expect("temp dir");
    base
}

fn find_browser() -> Option<PathBuf> {
    if let Some(overridden) = std::env::var_os("VALLE_WEB_PARITY_BROWSER") {
        let path = PathBuf::from(overridden);
        return path.exists().then_some(path);
    }
    let mac = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if cfg!(target_os = "macos") && mac.exists() {
        return Some(mac);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for name in [
            "chromium",
            "google-chrome",
            "google-chrome-stable",
            "chrome",
        ] {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn require_browser() -> bool {
    std::env::var("VALLE_WEB_PLAYER_REQUIRE_BROWSER").is_ok()
        || std::env::var("VALLE_WEB_PARITY_REQUIRE_BROWSER").is_ok()
}

/// Standalone Motion and Project Studio share one runtime.
fn web_runtime_dir() -> PathBuf {
    std::env::var_os("VALLE_WEB_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"))
}

/// Build the complete runtime, including engine Wasm, before running this test.
fn wasm_ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(wasm_ready_inner)
}

fn wasm_ready_inner() -> bool {
    web_runtime_dir()
        .join(valle_cli::webruntime::BUILD_MANIFEST_FILE)
        .is_file()
}

fn wait_event(
    lines: &Receiver<String>,
    what: &str,
    timeout: Duration,
    logs: &ChildLogs,
    want: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    // Retain all received events to diagnose missing reports and error responses.
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let Ok(line) = lines.recv_timeout(Duration::from_millis(500)) else {
            continue;
        };
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) if want(&value) => return value,
            _ => seen.push(line),
        }
    }
    panic!(
        "timed out waiting for {what} after {}s\n\
         --- Studio NDJSON (received {} lines; none matched)---\n{}\n{}",
        timeout.as_secs(),
        seen.len(),
        if seen.is_empty() {
            "(no lines received)".to_owned()
        } else {
            seen.join("\n")
        },
        logs.dump()
    );
}

fn wait_browser_event(
    lines: &Receiver<String>,
    browser: &mut BrowserGuard,
    what: &str,
    timeout: Duration,
    logs: &ChildLogs,
    want: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        if let Some(status) = browser.child.try_wait().expect("poll browser") {
            panic!(
                "{what}: browser exited before reporting (status {status})\n\
                 binary: {}\npage: {}\n{}",
                browser.binary.display(),
                browser.page,
                logs.dump()
            );
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            panic!(
                "timed out waiting for {what} after {}s while browser remained alive\n\
                 page: {}\n--- Studio NDJSON ({} unmatched) ---\n{}\n{}",
                timeout.as_secs(),
                browser.page,
                seen.len(),
                if seen.is_empty() {
                    "(none)".to_owned()
                } else {
                    seen.join("\n")
                },
                logs.dump()
            );
        };
        match lines.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(line) => match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(value) if want(&value) => return value,
                _ => seen.push(line),
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!(
                    "Studio event stream closed while waiting for {what}\n{}",
                    logs.dump()
                )
            }
        }
    }
}

#[test]
fn motion_studio_controls_are_authoring_state_until_a_new_package_is_published() {
    let Some(browser) = find_browser() else {
        if require_browser() {
            panic!("browser unavailable but require flag is set");
        }
        eprintln!("skip motion_studio_props_are_editable: browser unavailable");
        return;
    };
    if !wasm_ready() {
        if require_browser() {
            panic!("web wasm bundle unavailable but require flag is set");
        }
        eprintln!("skip motion_studio_props_are_editable: web wasm bundle unavailable");
        return;
    }

    let dir = temp_dir("valle_motion_studio");
    let logs = ChildLogs::new(&dir);
    let source = dir.join("knob.motion.tsx");
    std::fs::write(&source, SOURCE).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(["--events", "motion", "studio"])
        .arg(&source)
        .args(["--port", "0"])
        .args(["--size", "240x140"])
        .args(["--web-assets-dir"])
        .arg(web_runtime_dir())
        .stdout(Stdio::piped())
        .stderr(ChildLogs::sink(&logs.studio_stderr))
        .spawn()
        .expect("spawn motion studio");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let _studio = KillOnDrop(child);

    let ready = wait_event(
        &lines,
        "studio ready",
        Duration::from_secs(60),
        &logs,
        |v| v["type"] == "ready",
    );
    let port = ready["data"]["port"].as_u64().expect("port");

    let profile = temp_dir("valle_motion_studio_profile");
    let page = format!("http://127.0.0.1:{port}/studio?smoke=1");
    let chrome = Command::new(&browser)
        .args([
            "--headless=new",
            "--no-sandbox",
            &format!("--user-data-dir={}", profile.display()),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--mute-audio",
            "--hide-scrollbars",
            &page,
        ])
        .stdout(ChildLogs::sink(&logs.browser_stdout))
        .stderr(ChildLogs::sink(&logs.browser_stderr))
        .spawn()
        .expect("launch browser");
    let mut chrome = BrowserGuard::new(chrome, browser, page);

    let report = wait_browser_event(
        &lines,
        &mut chrome,
        "browser report",
        Duration::from_secs(180),
        &logs,
        |v| v["type"] == "browser_report",
    )["data"]
        .clone();
    assert_chromium_evidence(&report, &chrome.binary);
    assert_eq!(report["status"], "ok", "{report}");
    let probe = &report["stats"]["motionStudio"];
    assert_eq!(
        probe["propsEditable"], true,
        "props panel must expose editable controls: {report}"
    );
    assert_eq!(
        probe["fixedPackagePinnedAfterLocalEdit"], true,
        "local controls must not mutate pinned package pixels: {report}"
    );
    assert_eq!(
        probe["playbackVariesAcrossFrames"], true,
        "pinned package phase samples must vary across frames: {report}"
    );

    // Canvas selection resolves source locations using the displayed frame's layout boxes.
    let locate = &probe["locate"];
    assert!(
        locate.is_object(),
        "canvas center must hit a node: {report}"
    );
    assert!(
        locate["location"]
            .as_str()
            .is_some_and(|s| s.contains("knob.motion.tsx:")),
        "selection must resolve source file:line:column : {report}"
    );

    // The server remains available and retains the latest report.
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port as u16)).expect("connect");
    socket
        .write_all(b"GET /config.json HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .expect("request");
    let mut body = String::new();
    use std::io::Read;
    socket.read_to_string(&mut body).expect("read");
    assert!(
        body.contains("\"status\":\"ok\""),
        "Studio configuration must remain available"
    );
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        SOURCE,
        "standalone Studio controls are preview-only and must not rewrite TSX"
    );
}

#[test]
fn motion_studio_hot_reload_keeps_last_good_frame_and_recovers() {
    let Some(browser) = find_browser() else {
        if require_browser() {
            panic!("browser unavailable but require flag is set");
        }
        eprintln!("skip motion_studio_hot_reload: browser unavailable");
        return;
    };
    if !wasm_ready() {
        if require_browser() {
            panic!("web wasm bundle unavailable but require flag is set");
        }
        eprintln!("skip motion_studio_hot_reload: web wasm bundle unavailable");
        return;
    }

    let dir = temp_dir("valle_motion_studio_hot_reload");
    let logs = ChildLogs::new(&dir);
    let source = dir.join("knob.motion.tsx");
    std::fs::write(&source, SOURCE).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(["--events", "motion", "studio"])
        .arg(&source)
        .args(["--port", "0", "--size", "240x140"])
        .args(["--web-assets-dir"])
        .arg(web_runtime_dir())
        .stdout(Stdio::piped())
        .stderr(ChildLogs::sink(&logs.studio_stderr))
        .spawn()
        .expect("spawn Motion hot reload Studio");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let _studio = KillOnDrop(child);
    let ready = wait_event(
        &lines,
        "hot reload Studio ready",
        Duration::from_secs(60),
        &logs,
        |value| value["type"] == "ready",
    );
    let port = ready["data"]["port"].as_u64().expect("port");
    let profile = temp_dir("valle_motion_studio_hot_reload_profile");
    let page = format!("http://127.0.0.1:{port}/studio?hot-smoke=1");
    let chrome = Command::new(&browser)
        .args([
            "--headless=new",
            "--no-sandbox",
            &format!("--user-data-dir={}", profile.display()),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--mute-audio",
            "--hide-scrollbars",
            &page,
        ])
        .stdout(ChildLogs::sink(&logs.browser_stdout))
        .stderr(ChildLogs::sink(&logs.browser_stderr))
        .spawn()
        .expect("launch hot reload browser");
    let mut chrome = BrowserGuard::new(chrome, browser, page);

    let initial = wait_browser_event(
        &lines,
        &mut chrome,
        "hot reload browser initial state",
        Duration::from_secs(180),
        &logs,
        |value| value["type"] == "browser_report" && value["data"]["status"] == "hot-ready",
    )["data"]
        .clone();
    assert_eq!(initial["stats"]["motionHot"]["initialGeneration"], 1);

    std::fs::write(&source, "export default function Broken( {").unwrap();
    let failed = wait_browser_event(
        &lines,
        &mut chrome,
        "hot reload diagnostics",
        Duration::from_secs(180),
        &logs,
        |value| {
            value["type"] == "browser_report" && value["data"]["status"] == "hot-error-observed"
        },
    )["data"]
        .clone();
    let error = &failed["stats"]["motionHot"]["error"];
    assert_eq!(error["generation"], 2, "{failed}");
    assert_eq!(error["lastGoodFrameKept"], true, "{failed}");
    assert!(error["diagnostics"].as_u64().unwrap_or(0) > 0, "{failed}");

    // Change a producer-owned control default so the replacement package changes both the
    // canonical Timeline props and rendered geometry. This is a stronger hot-reload probe than a
    // paint-only source edit, which can be visually neutral for an unsupported authoring style.
    let recovered_source = SOURCE.replace("default: 40", "default: 80");
    std::fs::write(&source, &recovered_source).unwrap();
    let recovered = wait_browser_event(
        &lines,
        &mut chrome,
        "hot reload recovery",
        Duration::from_secs(180),
        &logs,
        |value| value["type"] == "browser_report" && value["data"]["status"] == "ok",
    )["data"]
        .clone();
    assert_chromium_evidence(&recovered, &chrome.binary);
    let hot = &recovered["stats"]["motionHot"];
    assert_eq!(hot["recoveredGeneration"], 3, "{recovered}");
    assert_eq!(hot["recoveredPixelsChanged"], true, "{recovered}");
}

/// Exercise four scenes at 1920x1080 in Studio, checking control edits, pinned packages and canvas-to-source selection.
#[test]
fn module_scenes_are_editable_and_source_locatable_in_studio() {
    let Some(browser) = find_browser() else {
        if require_browser() {
            panic!("browser unavailable but require flag is set");
        }
        eprintln!("skip Studio scene acceptance: browser unavailable");
        return;
    };
    if !wasm_ready() {
        if require_browser() {
            panic!("web wasm bundle unavailable but require flag is set");
        }
        eprintln!("skip Studio scene acceptance: web wasm bundle unavailable");
        return;
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/valle-compiler/tests/fixtures/motion/modules");
    for (name, source, data, asset) in [
        (
            "dashboard",
            "components/dashboard.tsx",
            Some("data-dashboard/dashboard.data.json"),
            None,
        ),
        ("route-network", "components/route-network.tsx", None, None),
        (
            "code-build",
            "components/code-build.tsx",
            None,
            Some("dot=assets/dot.png"),
        ),
        ("brand-poster", "components/brand-poster.tsx", None, None),
    ] {
        let dir = temp_dir(&format!("valle_scene_studio_{name}"));
        let logs = ChildLogs::new(&dir);
        let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
        command
            .args(["--events", "motion", "studio"])
            .arg(root.join(source))
            .args([
                "--port",
                "0",
                "--duration",
                "6",
                "--fps",
                "30",
                "--size",
                "1920x1080",
                "--web-assets-dir",
            ])
            .arg(web_runtime_dir());
        if let Some(relative) = data {
            command.args(["--data"]).arg(root.join(relative));
        }
        if let Some(binding) = asset {
            let (control, relative) = binding.split_once('=').expect("asset binding");
            command
                .args(["--asset"])
                .arg(format!("{control}={}", root.join(relative).display()));
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(ChildLogs::sink(&logs.studio_stderr))
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {name} Studio: {error}"));
        let stdout = child.stdout.take().expect("Studio stdout");
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let studio = KillOnDrop(child);
        let ready = wait_event(
            &lines,
            &format!("{name} Studio ready"),
            Duration::from_secs(60),
            &logs,
            |value| value["type"] == "ready",
        );
        let port = ready["data"]["port"].as_u64().expect("Studio port");
        let profile = temp_dir(&format!("valle_scene_studio_{name}_profile"));
        let page = format!("http://127.0.0.1:{port}/studio?smoke=1");
        let chrome = Command::new(&browser)
            .args([
                "--headless=new",
                "--disable-dev-shm-usage",
                "--no-sandbox",
                &format!("--user-data-dir={}", profile.display()),
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--mute-audio",
                "--hide-scrollbars",
                "--window-size=1920,1080",
                &page,
            ])
            .stdout(ChildLogs::sink(&logs.browser_stdout))
            .stderr(ChildLogs::sink(&logs.browser_stderr))
            .spawn()
            .unwrap_or_else(|error| panic!("launch {name} Studio browser: {error}"));
        let mut chrome = BrowserGuard::new(chrome, browser.clone(), page);
        let report = wait_browser_event(
            &lines,
            &mut chrome,
            &format!("{name} Studio browser report"),
            Duration::from_secs(240),
            &logs,
            |value| value["type"] == "browser_report",
        )["data"]
            .clone();
        assert_chromium_evidence(&report, &chrome.binary);
        assert_eq!(report["status"], "ok", "{name}: {report}");
        let probe = &report["stats"]["motionStudio"];
        assert_eq!(probe["prop"], "accentStrength", "{name}: {report}");
        assert_eq!(probe["propsEditable"], true, "{name}: {report}");
        assert_eq!(
            probe["fixedPackagePinnedAfterLocalEdit"], true,
            "{name}: {report}"
        );
        assert_eq!(
            probe["playbackVariesAcrossFrames"], true,
            "{name}: {report}"
        );
        let location = probe["locate"]["location"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no source location: {report}"));
        assert!(
            location.contains(".tsx:"),
            "{name} locate did not return an authored TSX span: {location}"
        );
        drop(chrome);
        drop(studio);
    }
}
