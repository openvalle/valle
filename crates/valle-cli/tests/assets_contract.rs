use serde_json::{Value, json};
use std::{path::Path, process::Command};
use valle_media::{
    codec::{FloatWavWriter, TransparentVideoMuxer, write_rgba_png},
    frame::{AudioBuffer, RgbaFrame},
};
use valle_project::assets::{
    Ctx, Home,
    analysis::{AnalyzerOutput, write_slot},
};

fn run(root: &Path, args: &[&str], json: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    command.env("VALLE_HOME", root).arg("assets");
    if json {
        command.arg("--json");
    }
    let result = command.args(args).output().unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    if json {
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["ok"], true);
    }
    text
}

#[test]
fn assets_cli_exposes_read_write_analyze_and_maintenance_contracts() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let wav = root.path().join("speech.wav");
    let mut writer = FloatWavWriter::create(&wav, 22_050, 1).unwrap();
    let samples = (0..88_200)
        .map(|i| if i % 11_025 < 80 { 0.5 } else { 0.0 })
        .collect();
    writer
        .write(&AudioBuffer {
            samples,
            sample_rate: 22_050,
            channels: 1,
        })
        .unwrap();
    writer.finish().unwrap();
    let added: Value = serde_json::from_str(&run(
        &home,
        &[
            "add",
            wav.to_str().unwrap(),
            "--mode",
            "copy",
            "--title",
            "Greeting",
            "--tag",
            "speech",
        ],
        true,
    ))
    .unwrap();
    let hash = added["data"]["items"][0]["content_digest"]
        .as_str()
        .unwrap()
        .strip_prefix("sha256:")
        .unwrap()
        .to_owned();
    let context = Ctx::bare(Home::at(&home));
    write_slot(
        &context,
        &hash,
        "asr",
        1,
        &json!({}),
        &AnalyzerOutput {
            items: vec![
                json!({"text":"Hello ","start_ms":100,"end_ms":300}),
                json!({"text":"world.","start_ms":300,"end_ms":800}),
            ],
            cost_ms: 25,
            cost_fen: 3,
        },
    )
    .unwrap();
    let video = root.path().join("sample.mov");
    let mut muxer = TransparentVideoMuxer::open(&video, 32, 32, 10, 1).unwrap();
    let mut frame = RgbaFrame::new(32, 32);
    for color in [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [0, 0, 255, 255],
    ] {
        frame.fill(color);
        muxer.encode_video(&frame).unwrap();
    }
    muxer.finish().unwrap();
    let added: Value = serde_json::from_str(&run(
        &home,
        &["add", video.to_str().unwrap(), "--mode", "reference"],
        true,
    ))
    .unwrap();
    let video_hash = added["data"]["items"][0]["content_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    for json in [false, true] {
        run(&home, &["list", "--kind", "audio", "--tag", "speech"], json);
        run(&home, &["list", "--stale"], json);
        run(&home, &["show", &hash], json);
        run(&home, &["stats"], json);
        run(&home, &["resolve", &hash], json);
        run(&home, &["transcript", &hash, "--level", "word"], json);
        run(&home, &["transcript", &hash], json);
        run(
            &home,
            &["edit", &hash, "--title", "Revised", "--subkind", "sfx"],
            json,
        );
        run(
            &home,
            &["tag", &hash, "--add", "new", "--rm", "unused"],
            json,
        );
        run(
            &home,
            &[
                "annotate", &hash, "--range", "0.1", "0.3", "--text", "Opening", "--tag", "intro",
            ],
            json,
        );
        run(&home, &["search", "Opening", "--limit", "2"], json);
        run(&home, &["search", "absent-query"], json);
        run(
            &home,
            &[
                "entity",
                "add",
                "--name",
                if json { "Second speaker" } else { "Speaker" },
                "--kind",
                "person",
                "--alias",
                if json { "Second narrator" } else { "Narrator" },
            ],
            json,
        );
        run(&home, &["entity", "list"], json);
        run(&home, &["analyze", &hash, "--with", "beats"], json);
        run(&home, &["analyze", &video_hash, "--with", "shots"], json);
        run(&home, &["maintenance", "verify", "--deep"], json);
        run(
            &home,
            &[
                "maintenance",
                "sql",
                "SELECT hash, width, stale, title FROM assets",
            ],
            json,
        );
        run(&home, &["maintenance", "gc"], json);
        run(&home, &["maintenance", "reindex"], json);
    }
    let result = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", &home)
        .args(["assets", "--events", "stats"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
    run(&home, &["remove", &hash], false);
    let removed = run(&home, &["list", "--removed"], false);
    assert!(removed.contains("removed"));
    let revived = run(
        &home,
        &["add", wav.to_str().unwrap(), "--mode", "copy"],
        false,
    );
    assert!(revived.contains("revived"));
    let existing = run(
        &home,
        &["add", wav.to_str().unwrap(), "--mode", "reflink"],
        false,
    );
    assert!(existing.contains("existing"));
    for args in [
        vec!["add", wav.to_str().unwrap(), "--mode", "invalid"],
        vec!["list", "--kind", "invalid"],
        vec!["transcript", &hash, "--level", "invalid"],
        vec!["analyze", "--all", "--with", "asr"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_valle"))
            .env("VALLE_HOME", &home)
            .arg("assets")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn assets_native_probes_admit_fonts_lottie_and_scene3d_and_reject_corrupt_media() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let fixtures = [
        (
            "face.ttf",
            "font",
            &include_bytes!("../../../assets/fonts/noto/NotoSans-Regular.ttf")[..],
        ),
        (
            "scene.glb",
            "model3d",
            &include_bytes!("../../valle-motion/tests/fixtures/scene3d/triangle.glb")[..],
        ),
        (
            "animation.json",
            "lottie",
            &br#"{"fr":30,"ip":0,"op":60,"w":32,"h":32,"layers":[]}"#[..],
        ),
    ];
    for (name, kind, bytes) in fixtures {
        let path = root.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        run(
            &home,
            &["add", path.to_str().unwrap(), "--kind", kind],
            true,
        );
    }
    let image = root.path().join("picture.png");
    let mut frame = RgbaFrame::new(2, 2);
    frame.fill([80, 100, 120, 255]);
    write_rgba_png(&image, &frame).unwrap();
    run(&home, &["add", image.to_str().unwrap()], true);
    let invalid = root.path().join("corrupt.mp4");
    std::fs::write(&invalid, b"corrupt").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_valle"))
        .env("VALLE_HOME", &home)
        .args(["assets", "add"])
        .arg(&image)
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8(result.stdout).unwrap().contains('✗'));
}
