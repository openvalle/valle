//! Mock semantic analysis covers shot transcripts, schema validation, synonyms, audio sentence units, adjacent-hit merging and optional frames.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use valle_project::assets::add::add;
use valle_project::assets::analysis::{AnalyzerOutput, load_slot, write_slot};
use valle_project::assets::analyze::analyze;
use valle_project::assets::search::search;
use valle_project::assets::vlm::{FrameExtractor, VlmAnalyzer};
use valle_project::assets::{AddMode, Ctx, Home};

fn setup(tmp: &Path, name: &str) -> (Ctx, String) {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let c = Ctx::bare(Home::at(tmp.join("home")));
    let src = tmp.join(name);
    std::fs::write(&src, format!("bytes-{name}")).unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (c, hash)
}

/// Mock a two-shot analysis slot.
fn fabricate_shots(c: &Ctx, hash: &str) {
    write_slot(
        c,
        hash,
        "shots",
        1,
        &json!({"threshold": 27.0}),
        &AnalyzerOutput {
            items: vec![
                json!({"start_ms": 0, "end_ms": 3000}),
                json!({"start_ms": 3000, "end_ms": 6000}),
            ],
            cost_ms: 10,
            cost_fen: 0,
        },
    )
    .unwrap();
}

/// Mock an asr@1 slot with three segments.
fn fabricate_asr(c: &Ctx, hash: &str) {
    write_slot(
        c,
        hash,
        "asr",
        1,
        &json!({"gap_s": 0.8}),
        &AnalyzerOutput {
            items: vec![
                json!({"start_ms": 200, "end_ms": 1500, "text": "今天我们去爬山"}),
                json!({"start_ms": 2800, "end_ms": 3600, "text": "山顶风景真好"}),
                json!({"start_ms": 4000, "end_ms": 5500, "text": "记得点赞关注"}),
            ],
            cost_ms: 20,
            cost_fen: 0,
        },
    )
    .unwrap();
}

fn fabricate_shots_asr(c: &Ctx, hash: &str) {
    fabricate_shots(c, hash);
    fabricate_asr(c, hash);
}

#[test]
fn shot_units_transcript_sliced_and_searchable() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "v.mp4");
    fabricate_shots_asr(&c, &hash);

    // The hiking query matches only the first shot window.
    let out = search(&c, "爬山", None, None, None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{out}");
    assert_eq!(results[0]["asset"], json!(hash));
    assert_eq!(results[0]["range"], json!([0.0, 3.0]));
    assert_eq!(results[0]["evidence"][0]["field"], "transcript");

    // A transcript spanning the cut is visible in both windows.
    let out = search(&c, "山顶", None, None, None, None, None).unwrap();
    // Merge adjacent shot hits into one [0, 6] result.
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "merge adjacent hits: {out}");
    assert_eq!(results[0]["range"], json!([0.0, 6.0]));
}

#[test]
fn audio_only_asr_segments_become_units() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "pod.mp3");
    write_slot(
        &c,
        &hash,
        "asr",
        1,
        &json!({}),
        &AnalyzerOutput {
            // Audio-only search units follow punctuation-delimited sentences; include sentence-ending punctuation in ASR items.
            items: vec![
                json!({"start_ms": 0, "end_ms": 4000, "text": "欢迎收听本期节目。"}),
                json!({"start_ms": 5000, "end_ms": 9000, "text": "今天聊聊剪辑工作流。"}),
            ],
            cost_ms: 5,
            cost_fen: 0,
        },
    )
    .unwrap();
    let out = search(&c, "剪辑 工作流", None, None, None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{out}");
    assert_eq!(results[0]["range"], json!([5.0, 9.0]));
    assert_eq!(
        results[0]["frame"],
        serde_json::Value::Null,
        "search must tolerate missing frames"
    );
}

struct FakeExtractor;
impl FrameExtractor for FakeExtractor {
    fn extract(
        &self,
        home: &Home,
        hash: &str,
        _media_path: &Path,
        times_ms: &[i64],
    ) -> valle_project::assets::Result<Vec<Option<String>>> {
        let mut out = Vec::new();
        for ms in times_ms {
            let p = valle_project::assets::cachefs::ensure_cache_path(
                home,
                "frames",
                hash,
                &ms.to_string(),
                "png",
            )?;
            std::fs::write(&p, b"fake-png").unwrap();
            out.push(Some(p.to_string_lossy().into_owned()));
        }
        Ok(out)
    }
}

fn vlm_script(tmp: &Path, name: &str, body: &str) -> Vec<String> {
    let p = tmp.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![p.to_string_lossy().into_owned()]
}

#[test]
fn vlm_recipe_end_to_end_with_synonyms() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "v.mp4");
    fabricate_shots_asr(&c, &hash);

    // Mock schema-valid VLM output for both windows with synonyms; retain the request for assertions.
    let reqdump = tmp.path().join("req.json");
    let argv = vlm_script(
        tmp.path(),
        "vlm.sh",
        &format!(
            "cat > {}\necho '{{\"items\":[{{\"start_ms\":0,\"end_ms\":3000,\"subjects\":[\"一名男子\"],\"event\":\"推着割草机修剪草坪\",\"scene\":\"庭院\",\"shot_scale\":\"全景\",\"empty_broll\":false,\"negative_space\":true,\"dominant_color\":\"绿色\",\"synonyms\":[\"割草机\",\"除草机\",\"打草机\"],\"summary\":\"男子在庭院修剪草坪\"}},{{\"start_ms\":3000,\"end_ms\":6000,\"subjects\":[],\"event\":\"空镜\",\"scene\":\"山顶远景\",\"shot_scale\":\"远景\",\"empty_broll\":true,\"negative_space\":true,\"dominant_color\":\"蓝色\",\"synonyms\":[\"山\",\"山顶\"],\"summary\":\"山顶空镜远景\"}}],\"cost\":{{\"ms\":4000,\"fen\":0}}}}'",
            reqdump.display()
        ),
    );
    let mut c2 = Ctx::bare(c.home.clone());
    c2.analyzers = vec![Box::new(VlmAnalyzer {
        argv,
        timeout: Duration::from_secs(10),
        params: json!({"model": "mock"}),
        extractor: Box::new(FakeExtractor),
    })];
    let out = analyze(
        &c2,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["vlm".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 1, "{out}");

    // The batch request includes both windows, frame paths and transcripts.
    let req: serde_json::Value = serde_json::from_slice(&std::fs::read(&reqdump).unwrap()).unwrap();
    let batch = req["batch"].as_array().unwrap();
    assert_eq!(batch.len(), 2);
    assert!(batch[0]["transcript"].as_str().unwrap().contains("爬山"));
    assert!(
        !batch[0]["frames"].as_array().unwrap().is_empty(),
        "include frame paths"
    );
    // Shots longer than two seconds use first, middle and last frames.
    assert_eq!(batch[0]["frames"].as_array().unwrap().len(), 3);

    // Project VLM descriptions into searchable ai_text.
    let out = search(&c, "庭院", None, None, None, None, None).unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 1);
    assert_eq!(out["results"][0]["evidence"][0]["field"], "ai_text");

    // Index generated synonyms and expand the corresponding query synonym group.
    let out = search(&c, "打草机", None, None, None, None, None).unwrap();
    assert_eq!(
        out["results"].as_array().unwrap().len(),
        1,
        "synonym query must match: {out}"
    );

    // The empty-b-roll query maps to the AI enum filter.
    let out = search(&c, "空镜 山顶", None, None, None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert!(!results.is_empty(), "{out}");
    assert_eq!(results[0]["range"], json!([3.0, 6.0]));

    // Include a representative frame path in each unit.
    assert!(
        out["results"][0]["frame"]
            .as_str()
            .unwrap()
            .contains("cache/frames")
    );
}

#[test]
fn vlm_recipe_runs_without_historical_asr_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "silent.mp4");
    fabricate_shots(&c, &hash);

    let reqdump = tmp.path().join("req-without-asr.json");
    let argv = vlm_script(
        tmp.path(),
        "vlm-without-asr.sh",
        &format!(
            "cat > {}\necho '{{\"items\":[{{\"start_ms\":0,\"end_ms\":3000,\"summary\":\"first\",\"synonyms\":[]}},{{\"start_ms\":3000,\"end_ms\":6000,\"summary\":\"second\",\"synonyms\":[]}}]}}'",
            reqdump.display()
        ),
    );
    let mut c2 = Ctx::bare(c.home.clone());
    c2.analyzers = vec![Box::new(VlmAnalyzer {
        argv,
        timeout: Duration::from_secs(10),
        params: json!({"model": "mock"}),
        extractor: Box::new(FakeExtractor),
    })];

    let out = analyze(
        &c2,
        &[hash[..8].to_owned()],
        false,
        None,
        None,
        &["vlm".into()],
        None,
        false,
    )
    .unwrap();
    assert_eq!(out["done"], 1, "{out}");

    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&reqdump).unwrap()).unwrap();
    for window in request["batch"].as_array().unwrap() {
        assert_eq!(window["transcript"], json!(""));
    }
}

#[test]
fn vlm_invalid_output_retries_then_fails_without_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "v.mp4");
    fabricate_shots_asr(&c, &hash);

    let counter = tmp.path().join("calls");
    // Missing synonyms fail validation on both attempts.
    let argv = vlm_script(
        tmp.path(),
        "bad_vlm.sh",
        &format!(
            "echo x >> {}\ncat > /dev/null\necho '{{\"items\":[{{\"start_ms\":0,\"end_ms\":3000,\"summary\":\"ok\"}},{{\"start_ms\":3000,\"end_ms\":6000,\"summary\":\"ok\"}}]}}'",
            counter.display()
        ),
    );
    let mut c2 = Ctx::bare(c.home.clone());
    c2.analyzers = vec![Box::new(VlmAnalyzer {
        argv,
        timeout: Duration::from_secs(10),
        params: json!({"model": "mock"}),
        extractor: Box::new(FakeExtractor),
    })];
    let out = analyze(
        &c2,
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
    // One retry gives two calls total.
    assert_eq!(
        std::fs::read_to_string(&counter).unwrap().lines().count(),
        2,
        "retry a failure once"
    );
    // Invalid output must not create a VLM slot.
    assert!(load_slot(&c.home, &hash, "vlm", 1).unwrap().is_none());
    let failed = out["failed"].as_array().unwrap();
    assert!(
        failed[0]["error"].as_str().unwrap().contains("schema"),
        "{out}"
    );
}
