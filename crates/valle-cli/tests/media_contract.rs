//! Public CLI publication contracts using deterministic graphs and the real ORT/codecs.
#[path = "../../valle-media/test_support/models.rs"]
mod models;

use std::{
    path::Path,
    process::{Command, Output},
};
use valle_media::{
    codec::{FloatWavWriter, TransparentVideoMuxer, write_rgba_png},
    frame::{AudioBuffer, RgbaFrame},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    command
        .env("VALLE_MODEL_CACHE", root.join("models"))
        .env("VALLE_LEGACY_MODEL_CACHE", root.join("legacy"))
        .env("VALLE_HOME", root.join("home"));
    command
}

fn success(output: Output, json: bool) -> Option<serde_json::Value> {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if json {
        Some(serde_json::from_slice(&output.stdout).unwrap())
    } else {
        None
    }
}

fn video(path: &Path) {
    let mut muxer = TransparentVideoMuxer::open(path, 64, 48, 10, 1).unwrap();
    let mut frame = RgbaFrame::new(64, 48);
    for color in [[40, 100, 150, 255], [60, 100, 150, 255]] {
        frame.fill(color);
        muxer.encode_video(&frame).unwrap();
    }
    muxer.finish().unwrap();
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn media_commands_publish_reports_and_support_human_and_json_output() {
    for (tool, id) in [
        ("enhance", "dpdfnet"),
        ("separate", "demucs"),
        ("shots", "omnishotcut"),
        ("matte", "birefnet"),
    ] {
        for json in [true, false] {
            let root = tempfile::tempdir().unwrap();
            models::install(&root.path().join("models"), id);
            let input = root.path().join(if tool == "matte" {
                "source.png"
            } else if tool == "shots" {
                "source.mov"
            } else {
                "source.wav"
            });
            if tool == "shots" {
                video(&input);
            } else if tool == "matte" {
                let mut frame = RgbaFrame::new(2, 2);
                frame.fill([80, 100, 120, 128]);
                write_rgba_png(&input, &frame).unwrap();
            } else {
                let channels = if tool == "separate" { 2 } else { 1 };
                let sample_rate = if tool == "separate" { 44_100 } else { 48_000 };
                let mut writer = FloatWavWriter::create(&input, sample_rate, channels).unwrap();
                writer
                    .write(&AudioBuffer {
                        samples: vec![0.1; 1_000 * channels as usize],
                        sample_rate,
                        channels,
                    })
                    .unwrap();
                writer.finish().unwrap();
            }
            let report = root.path().join("report.json");
            let mut cmd = command(root.path());
            cmd.args(["media", tool])
                .arg(&input)
                .args([
                    "--model",
                    id,
                    "--model-version",
                    models::VERSION,
                    "--backend",
                    "onnx",
                ])
                .arg("--report")
                .arg(&report);
            if tool == "separate" {
                cmd.arg("--output").arg(root.path().join("stems"));
            }
            if json {
                cmd.arg("--json");
            }
            let envelope = success(cmd.output().unwrap(), json);
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
            if let Some(envelope) = envelope {
                assert_eq!(saved, envelope);
            }
            assert_eq!(saved["report"]["operation"], tool);
            assert_eq!(saved["report"]["models"][0]["version"], models::VERSION);
            assert!(saved["result"].is_object());
        }
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn matte_video_honors_ranges_and_rejects_bad_range_syntax() {
    let root = tempfile::tempdir().unwrap();
    models::install(&root.path().join("models"), "birefnet");
    let input = root.path().join("source.mov");
    video(&input);
    for range in ["0,", "0,0.2"] {
        let mut cmd = command(root.path());
        cmd.args(["media", "matte"]).arg(&input).args([
            "--range",
            range,
            "--fps",
            "10",
            "--model-version",
            models::VERSION,
            "--backend",
            "onnx",
            "--overwrite",
        ]);
        success(cmd.output().unwrap(), false);
    }
    for range in ["missing-comma", "bad,1", "0,bad", "1,0", "NaN,2"] {
        let output = command(root.path())
            .args(["media", "matte"])
            .arg(&input)
            .args(["--range", range, "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "invalid_input");
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires pinned Qwen ASR/aligner weights and VALLE_QWEN_FIXTURE_WAV"]
fn real_qwen_cli_publishes_valid_word_and_sentence_transcripts() {
    let source = std::path::PathBuf::from(
        std::env::var_os("VALLE_QWEN_FIXTURE_WAV").expect("real speech WAV"),
    );
    let artifact =
        std::path::PathBuf::from(std::env::var_os("VALLE_QWEN_ASR_DIR").expect("ASR artifact"));
    let models_root = artifact.ancestors().nth(4).unwrap();
    for (level, json) in [("word", true), ("sentence", false)] {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("speech.wav");
        std::fs::copy(&source, &input).unwrap();
        let mut cmd = command(root.path());
        cmd.env("VALLE_MODEL_CACHE", models_root)
            .args(["media", "transcribe"])
            .arg(&input)
            .args(["--lang", "en", "--level", level]);
        if json {
            cmd.arg("--json");
        }
        let result = success(cmd.output().unwrap(), json);
        let output = input.with_extension(if level == "word" {
            "words.json"
        } else {
            "sentences.json"
        });
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        if level == "word" {
            let transcript: valle_media::analysis::Transcript =
                serde_json::from_value(document).unwrap();
            transcript.validate().unwrap();
            assert!(!transcript.words.is_empty());
            assert!(
                !result.unwrap()["result"]["text"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert!(!document["sentences"].as_array().unwrap().is_empty());
        }
    }
}
