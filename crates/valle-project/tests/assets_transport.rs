//! External analyzer success, timeout, invalid JSON, batch process reuse and cache collection tests.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use valle_project::assets::add::add;
use valle_project::assets::analyze::analyze;
use valle_project::assets::cachefs::{cache_path, ensure_cache_path};
use valle_project::assets::maintain::gc;
use valle_project::assets::transport::ExternalAnalyzer;
use valle_project::assets::{AddMode, AssetKind, Ctx, ErrorCode, Home};

fn setup(tmp: &Path) -> (Home, String) {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let home = Home::at(tmp.join("home"));
    let c = Ctx::bare(home.clone());
    let src = tmp.join("v.mp4");
    std::fs::write(&src, b"ext-bytes").unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (home, hash)
}

fn script(tmp: &Path, name: &str, body: &str) -> Vec<String> {
    let p = tmp.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![p.to_string_lossy().into_owned()]
}

fn ext(name: &'static str, argv: Vec<String>, timeout_ms: u64) -> ExternalAnalyzer {
    ExternalAnalyzer {
        name,
        version: 1,
        kinds: vec![AssetKind::Video],
        deps: &[],
        argv,
        timeout: Duration::from_millis(timeout_ms),
        params: json!({}),
    }
}

#[test]
fn success_batch_single_process() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    // Count one process invocation, consume the batch from stdin and return two items.
    let counter = tmp.path().join("spawns");
    let argv = script(
        tmp.path(),
        "ok.sh",
        &format!(
            "echo x >> {}\ncat > /dev/null\necho '{{\"items\":[{{\"start_ms\":0,\"end_ms\":500,\"desc\":\"a\"}},{{\"start_ms\":500,\"end_ms\":900,\"desc\":\"b\"}}],\"cost\":{{\"ms\":7,\"fen\":3}}}}'",
            counter.display()
        ),
    );
    let mut c = Ctx::bare(home.clone());
    c.analyzers = vec![Box::new(ext("vlm", argv, 5000))];
    let out = analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["vlm".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 1);
    assert_eq!(out["cost"]["fen"], 3);
    // The full batch uses one process.
    let spawns = std::fs::read_to_string(&counter).unwrap();
    assert_eq!(spawns.lines().count(), 1, "one process handles the batch");
    // The analysis slot contains both items.
    let slot = valle_project::assets::analysis::load_slot(&home, &hash, "vlm", 1)
        .unwrap()
        .unwrap();
    assert_eq!(slot.items.len(), 2);
    assert_eq!(slot.cost.fen, 3);
}

#[test]
fn timeout_kills_and_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    let argv = script(
        tmp.path(),
        "slow.sh",
        "cat > /dev/null\nsleep 5\necho '{\"items\":[]}'",
    );
    let mut c = Ctx::bare(home);
    c.analyzers = vec![Box::new(ext("vlm", argv, 200))];
    let out = analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["vlm".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 0);
    let failed = out["failed"].as_array().unwrap();
    assert!(
        failed[0]["error"].as_str().unwrap().contains("timed out"),
        "{out}"
    );
}

#[test]
fn bad_json_reports_with_stderr_tail() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    let argv = script(
        tmp.path(),
        "bad.sh",
        "cat > /dev/null\necho 'boom log' >&2\necho 'not json at all'",
    );
    let mut c = Ctx::bare(home);
    c.analyzers = vec![Box::new(ext("vlm", argv, 5000))];
    let out = analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["vlm".into()],
        None,
        false,
    )
    .unwrap();
    let failed = out["failed"].as_array().unwrap();
    let msg = failed[0]["error"].as_str().unwrap();
    assert!(msg.contains("JSON"), "{msg}");
    assert!(msg.contains("boom log"), "include stderr summary: {msg}");
}

#[test]
fn direct_transport_error_codes() {
    // Check nonzero exit handling directly through run_external.
    let tmp = tempfile::tempdir().unwrap();
    let argv = script(
        tmp.path(),
        "fail.sh",
        "cat > /dev/null\necho 'oops' >&2\nexit 3",
    );
    let err = valle_project::assets::transport::run_external(
        &argv,
        &json!({"x": 1}),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::AnalyzerFailed);
    assert!(err.message.contains("exited with"));
}

#[test]
fn gc_cleans_cache_orphans_keeps_live() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    let c = Ctx::bare(home.clone());

    // Create cached data for live and dead hashes, plus an invalidly named file.
    let live_frame = ensure_cache_path(&home, "frames", &hash, "1000", "jpg").unwrap();
    std::fs::write(&live_frame, b"jpg").unwrap();
    let dead = "ff".repeat(32);
    let dead_frame = ensure_cache_path(&home, "frames", &dead, "2000", "jpg").unwrap();
    std::fs::write(&dead_frame, b"jpg").unwrap();
    let junk = home.cache_dir().join("frames").join("not-a-hash.tmp");
    std::fs::write(&junk, b"x").unwrap();

    let g = gc(&c).unwrap();
    assert_eq!(
        g.removed_cache, 2,
        "remove dead hashes and invalidly named cache files"
    );
    assert!(live_frame.exists(), "live entries must remain");
    assert!(!dead_frame.exists());
    assert!(!junk.exists());
    // Verify the cache key shape.
    assert!(
        cache_path(&home, "frames", &hash, "1000", "jpg")
            .to_string_lossy()
            .ends_with(&format!("cache/frames/{hash}-1000.jpg"))
    );
}
