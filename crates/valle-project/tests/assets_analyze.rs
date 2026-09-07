//! Analysis caching, replacement, kind filtering, dependencies, budgets and lock-scope tests.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use valle_project::assets::add::add;
use valle_project::assets::analysis::{Analyzer, AnalyzerInput, AnalyzerOutput, load_slot};
use valle_project::assets::analyze::analyze;
use valle_project::assets::{AddMode, AssetKind, Ctx, Db, ErrorCode, Home};

struct Mock {
    name: &'static str,
    accepts_kind: AssetKind,
    deps: &'static [&'static str],
    params: Value,
    fen: i64,
    fail: bool,
    calls: Arc<AtomicUsize>,
    /// Assert the write lock is available during analyzer execution.
    probe_lock: Option<Home>,
}

impl Analyzer for Mock {
    fn name(&self) -> &'static str {
        self.name
    }
    fn version(&self) -> u32 {
        1
    }
    fn accepts(&self, kind: AssetKind) -> bool {
        kind == self.accepts_kind
    }
    fn dependencies(&self) -> &'static [&'static str] {
        self.deps
    }
    fn default_params(&self) -> Value {
        self.params.clone()
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> valle_project::assets::Result<AnalyzerOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(home) = &self.probe_lock {
            // Long-running analysis must release the write lock.
            let _l = valle_project::assets::lock::acquire(home)
                .expect("write lock must be available during analysis");
        }
        if self.fail {
            return Err(valle_project::assets::AssetsError::analyzer_failed(
                "intentional mock failure",
            ));
        }
        assert!(input.path.exists(), "asset path must be resolved");
        Ok(AnalyzerOutput {
            items: vec![json!({"start_ms": 0, "end_ms": 1000, "note": self.name})],
            cost_ms: 5,
            cost_fen: self.fen,
        })
    }
}

fn mock(name: &'static str, kind: AssetKind, calls: &Arc<AtomicUsize>) -> Mock {
    Mock {
        name,
        accepts_kind: kind,
        deps: &[],
        params: json!({"threshold": 27}),
        fen: 0,
        fail: false,
        calls: calls.clone(),
        probe_lock: None,
    }
}

fn setup(tmp: &Path) -> (Home, String) {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let home = Home::at(tmp.join("home"));
    let c = Ctx::bare(home.clone());
    let src = tmp.join("v.mp4");
    std::fs::write(&src, b"video-bytes").unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (home, hash)
}

#[test]
fn slot_write_cache_hit_force_and_last_params_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    let calls = Arc::new(AtomicUsize::new(0));

    let mut c = Ctx::bare(home.clone());
    let mut m = mock("shots", AssetKind::Video, &calls);
    m.probe_lock = Some(home.clone());
    c.analyzers = vec![Box::new(m)];

    // The first run writes the analysis slot.
    let out = analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["shots".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let slot = load_slot(&home, &hash, "shots", 1).unwrap().unwrap();
    assert_eq!(slot.items.len(), 1);
    let first_params_fingerprint = slot.params_fingerprint;

    // A repeated run reuses cached results without recomputation.
    let out = analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["shots".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 0);
    assert_eq!(out["cached"], 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no recomputation");

    // Force recomputation.
    analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["shots".into()],
        None,
        true,
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // New parameters replace the same slot.
    let mut m2 = mock("shots", AssetKind::Video, &calls);
    m2.params = json!({"threshold": 99});
    c.analyzers = vec![Box::new(m2)];
    analyze(
        &c,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["shots".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "changed parameters must invalidate the cache"
    );
    let slot = load_slot(&home, &hash, "shots", 1).unwrap().unwrap();
    assert_ne!(slot.params_fingerprint, first_params_fingerprint);
    // Only one shots slot file exists.
    let n = std::fs::read_dir(home.analysis_dir(&hash))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("shots@")
        })
        .count();
    assert_eq!(n, 1, "keep only one value per slot");

    // Project shot segments into the index.
    let db = Db::open(&home).unwrap();
    let segs: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM segments WHERE hash=?1",
            [&hash],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(segs, 1);
    let projected_fingerprint: String = db
        .conn
        .query_row(
            "SELECT params_fingerprint FROM analysis WHERE hash=?1 AND analyzer='shots' AND version=1",
            [&hash],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(projected_fingerprint, slot.params_fingerprint.to_storage());
}

#[test]
fn kind_filter_and_selector() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, _hash) = setup(tmp.path());
    // Add an audio asset.
    let c0 = Ctx::bare(home.clone());
    let mp3 = tmp.path().join("a.mp3");
    std::fs::write(&mp3, b"audio-bytes").unwrap();
    add(&c0, &mp3, AddMode::Reflink, None, None, &[]).unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let mut c = Ctx::bare(home);
    c.analyzers = vec![Box::new(mock("shots", AssetKind::Video, &calls))];

    // Kind filtering selects video and excludes audio.
    let out = analyze(&c, &[], true, None, None, &["shots".into()], None, false).unwrap();
    assert_eq!(out["done"], 1);
    assert_eq!(out["skipped_kind"], 1);

    // An unknown analyzer fails with a list of available analyzers.
    let err = analyze(&c, &[], true, None, None, &["vlm".into()], None, false).unwrap_err();
    assert_eq!(err.code, ErrorCode::AnalyzerFailed);
    assert!(err.hint.unwrap().contains("shots"));
}

#[test]
fn dependency_chain_and_failure_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, hash) = setup(tmp.path());
    let shots_calls = Arc::new(AtomicUsize::new(0));
    let asr_calls = Arc::new(AtomicUsize::new(0));
    let vlm_calls = Arc::new(AtomicUsize::new(0));

    // Run missing shots and ASR dependencies before VLM analysis.
    let mut c = Ctx::bare(home.clone());
    let mut vlm = mock("vlm", AssetKind::Video, &vlm_calls);
    vlm.deps = &["shots", "asr"];
    let asr = Mock {
        accepts_kind: AssetKind::Video,
        ..mock("asr", AssetKind::Video, &asr_calls)
    };
    c.analyzers = vec![
        Box::new(vlm),
        Box::new(mock("shots", AssetKind::Video, &shots_calls)),
        Box::new(asr),
    ];
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
    assert_eq!(out["done"], 3, "{out}");
    assert_eq!(shots_calls.load(Ordering::SeqCst), 1);
    assert_eq!(asr_calls.load(Ordering::SeqCst), 1);
    assert_eq!(vlm_calls.load(Ordering::SeqCst), 1);

    // Skip downstream VLM analysis when ASR fails.
    let tmp2 = tempfile::tempdir().unwrap();
    let (home2, hash2) = setup(tmp2.path());
    let mut c2 = Ctx::bare(home2);
    let mut vlm2 = mock("vlm", AssetKind::Video, &vlm_calls);
    vlm2.deps = &["asr"];
    let mut asr2 = mock("asr", AssetKind::Video, &asr_calls);
    asr2.fail = true;
    c2.analyzers = vec![Box::new(vlm2), Box::new(asr2)];
    let out = analyze(
        &c2,
        &[hash2[..8].to_owned()],
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
    assert_eq!(failed.len(), 2, "{out}");
    assert!(
        failed
            .iter()
            .any(|f| f["error"].as_str().unwrap().contains("dependency failure"))
    );
}

#[test]
fn budget_stops_but_keeps_done() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, _h1) = setup(tmp.path());
    let c0 = Ctx::bare(home.clone());
    // Three video assets.
    let mut hashes = Vec::new();
    for i in 0..3 {
        let p = tmp.path().join(format!("v{i}.mp4"));
        std::fs::write(&p, format!("bytes-{i}")).unwrap();
        hashes.push(
            add(&c0, &p, AddMode::Reflink, None, None, &[])
                .unwrap()
                .content_digest
                .as_hex(),
        );
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut c = Ctx::bare(home.clone());
    let mut vlm = mock("vlm", AssetKind::Video, &calls);
    vlm.fen = 80; // 0.8 currency units per asset.
    c.analyzers = vec![Box::new(vlm)];

    // Select these three assets explicitly, excluding the setup asset.
    let sel: Vec<String> = hashes.iter().map(|h| h[..12].to_owned()).collect();
    // A 1.5 budget stops after the second result reaches 1.6; the third asset is not processed.
    let err = analyze(
        &c,
        &sel,
        false,
        None,
        None,
        &["vlm".into()],
        Some(1.5),
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::BudgetExceeded);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the third asset must not run"
    );
    // Retain completed analysis slots.
    let done_slots = hashes
        .iter()
        .filter(|h| load_slot(&home, h, "vlm", 1).unwrap().is_some())
        .count();
    assert_eq!(done_slots, 2, "retain completed results");
    // A larger budget resumes only unfinished work, using cached completed results.
    let out = analyze(
        &c,
        &sel,
        false,
        None,
        None,
        &["vlm".into()],
        Some(5.0),
        false,
    )
    .unwrap();
    assert_eq!(out["cached"], 2);
    assert_eq!(out["done"], 1);
}
