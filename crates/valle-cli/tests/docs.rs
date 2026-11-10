//! The guides' examples work as written: every ```tsx scene block and every ```json timeline
//! block passes `check`, and `valle docs` serves the guides from the executable alone.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const DOCS: [&str; 5] = ["start", "motion", "timeline", "project", "cli"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn guide(topic: &str) -> String {
    std::fs::read_to_string(root().join(format!("docs/{topic}.md"))).unwrap()
}

/// The code blocks of a guide whose info string starts with `kind`: their info and text.
fn blocks<'a>(text: &'a str, kind: &str) -> Vec<(&'a str, String)> {
    let mut out = Vec::new();
    let mut open: Option<(&str, String)> = None;
    for line in text.lines() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(info)) => open = Some((info.trim(), String::new())),
            (Some(_), Some(_)) => {
                let (info, body) = open.take().unwrap();
                if info.starts_with(kind) {
                    out.push((info, body));
                }
            }
            (Some((_, body)), None) => {
                body.push_str(line);
                body.push('\n');
            }
            (None, None) => {}
        }
    }
    out
}

fn valle(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|_| panic!("{args:?}: {text}"))
}

/// A WAV of a quiet tone, `seconds` long: sound for the examples' music.
fn wav(path: &Path, seconds: u32) {
    let rate = 48_000u32;
    let samples: Vec<i16> = (0..rate * seconds)
        .map(|i| ((i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 3000.0) as i16)
        .collect();
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | (1 << 16), rate, rate * 2, 2 | (16 << 16)] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// A file for each resource a timeline names that the folder does not have: the test resources
/// by kind, and a short scene for a Motion scene no guide defines.
fn provide(dir: &Path, doc: &Value) {
    let resources = root().join("tests/resources");
    for path in doc["resources"]
        .as_object()
        .into_iter()
        .flat_map(|r| r.values())
    {
        let path = path
            .as_str()
            .unwrap_or_else(|| path["path"].as_str().unwrap());
        let to = dir.join(path);
        if to.exists() {
            continue;
        }
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let from = |name: &str| std::fs::copy(resources.join(name), &to).unwrap();
        match path.rsplit('.').next().unwrap() {
            "mp4" | "mov" => drop(from("counter.mp4")),
            "png" | "jpg" | "webp" => drop(from("photo.png")),
            "ttf" | "otf" => drop(from("NotoSans.ttf")),
            "wav" | "mp3" | "m4a" => wav(&to, 10),
            "tsx" => std::fs::write(
                &to,
                "export const composition = { width: 320, height: 180, fps: 30, duration: 10 };\n\
                 export default function S() { return <View style={{ flex: 1 }} />; }\n",
            )
            .unwrap(),
            other => panic!("{path}: no test file for .{other}"),
        }
    }
}

#[test]
fn every_example_scene_and_timeline_checks() {
    let dir = std::env::temp_dir().join(format!("valle-docs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut checked = 0;
    for topic in DOCS {
        let text = guide(topic);
        // Scenes first: a timeline of the same guide may play them.
        for (info, body) in blocks(&text, "tsx scene") {
            let file = (info.split_whitespace().nth(2))
                .unwrap_or_else(|| panic!("{topic}: ```tsx scene takes a file name"));
            std::fs::write(dir.join(file), &body).unwrap();
            let out = valle(&dir, &["motion", "check", file]);
            assert_eq!(out["status"], "ok", "{topic}: {file}: {out}");
            checked += 1;
        }
        for (n, (_, body)) in blocks(&text, "json timeline").into_iter().enumerate() {
            let doc: Value = serde_json::from_str(&body)
                .unwrap_or_else(|e| panic!("{topic}: timeline {n} is not JSON: {e}"));
            provide(&dir, &doc);
            let file = format!("{topic}-{n}.timeline.json");
            std::fs::write(dir.join(&file), &body).unwrap();
            let out = valle(&dir, &["timeline", "check", &file]);
            assert_eq!(out["status"], "ok", "{topic}: timeline {n}: {out}");
            checked += 1;
        }
    }
    assert!(checked >= 3, "the guides have {checked} checked examples");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_moved_executable_serves_every_guide_as_written() {
    let dir = std::env::temp_dir().join(format!("valle-docs-moved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("valle{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_valle"), &exe).unwrap();
    for topic in DOCS {
        let out = Command::new(&exe)
            .args(["docs", topic])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{topic}: {out:?}");
        let shown = String::from_utf8(out.stdout).unwrap();
        assert_eq!(shown.trim_end(), guide(topic).trim_end(), "{topic}");
    }
    let listed = Command::new(&exe)
        .args(["docs", "--json"])
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    let topics: Vec<&str> = (listed["topics"].as_array().unwrap().iter())
        .map(|t| t["topic"].as_str().unwrap())
        .collect();
    assert_eq!(topics, DOCS);
    let _ = std::fs::remove_dir_all(&dir);
}
