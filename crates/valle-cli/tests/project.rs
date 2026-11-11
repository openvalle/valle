//! Agent-facing project results: source facts, check parity and authored error locations.

use std::path::{Path, PathBuf};
use std::process::Command;

use indoc::indoc;
use serde_json::{Value, json};

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

struct Project(PathBuf);

impl Project {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "valle-project-results-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let out = Command::new(VALLE)
            .args(["project", "create"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        Self(root)
    }

    fn run(&self, args: &[&str]) -> Value {
        let out = Command::new(VALLE)
            .current_dir(&self.0)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let result: Value =
            serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{out:?}"));
        assert_eq!(out.status.success(), result["status"] == "ok", "{result}");
        result
    }

    fn document(&self, file: &str, tracks: Value) {
        let doc = json!({"canvas":{"width":64,"height":36,"fps":30},"resources":{"clip":file},"tracks":tracks});
        std::fs::write(self.0.join("main.timeline.json"), doc.to_string()).unwrap();
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/resources/video")
        .join(file)
}

#[test]
fn imports_and_show_expose_oriented_source_metadata_and_probe_failures() {
    let p = Project::new("metadata");
    for (file, linked, size) in [
        ("rotate90.mp4", false, [64, 96]),
        ("quadrants.mp4", true, [96, 64]),
    ] {
        let file = fixture(file);
        let mut args = vec!["project", "add", file.to_str().unwrap()];
        if linked {
            args.push("--link");
        }
        let result = p.run(&args);
        let info = &result["added"][0]["media"];
        assert_eq!(info["duration"], 1.0, "{result}");
        assert_eq!(info["width"], size[0]);
        assert_eq!(info["height"], size[1]);
        assert_eq!(info["fps"], "4");
        assert_eq!(info["audio"], false);
        assert!(result["summary"].as_str().unwrap().contains("no audio"));
        let shown = p.run(&["project", "show"]);
        let entry = shown["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["alias"] == result["added"][0]["alias"])
            .unwrap();
        assert_eq!(&entry["media"], info);
    }
    let image = p.0.join("art.svg");
    std::fs::write(&image, r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><rect width="24" height="16" fill="red"/></svg>"#).unwrap();
    let added = p.run(&["project", "add", image.to_str().unwrap()]);
    assert_eq!(added["added"][0]["media"]["width"], 24.0);
    assert_eq!(added["added"][0]["media"]["height"], 16.0);
    assert!(added["added"][0]["media"]["duration"].is_null());
    let corrupt = p.0.join("broken.mp4");
    std::fs::write(&corrupt, b"not media").unwrap();
    let added = p.run(&["project", "add", corrupt.to_str().unwrap()]);
    assert_eq!(added["status"], "ok");
    assert!(added["added"][0]["media"]["error"].is_string());
    let shown = p.run(&["project", "show"]);
    assert_eq!(shown["status"], "ok");
    assert!(
        shown["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["media"]["error"].is_string())
    );
}

#[test]
fn project_checks_require_audio_and_preserve_hdr_warnings() {
    let p = Project::new("check");
    assert_eq!(p.run(&["project", "check"])["status"], "ok");
    let file = p.0.join("media/source.mp4");
    std::fs::copy(fixture("quadrants.mp4"), &file).unwrap();
    p.document(
        "media/source.mp4",
        json!({"audio":[{"items":[{"id":"sound","src":"clip","duration":1}]}]}),
    );
    for cost in [false, true] {
        let mut args = vec!["project", "check"];
        if cost {
            args.push("--cost");
        }
        let result = p.run(&args);
        assert_eq!(result["status"], "error", "{result}");
        let message = result.to_string();
        assert!(
            message.contains("has no sound") && message.contains("resources.clip"),
            "{message}"
        );
    }
    std::fs::copy(fixture("pq.mp4"), &file).unwrap();
    p.document(
        "media/source.mp4",
        json!({"visual":[{"clips":[{"kind":"video","src":"clip","start":0,"duration":0.25}]}]}),
    );
    for cost in [false, true] {
        let mut args = vec!["project", "check"];
        if cost {
            args.push("--cost");
        }
        let result = p.run(&args);
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["warnings"][0]["code"], "unsupported-hdr", "{result}");
        assert!(result["summary"].as_str().unwrap().contains("HDR (PQ)"));
    }
    // The inner document owns the source path and its warnings, even under another folder.
    let inner = p.0.join("timelines/inner.timeline.json");
    std::fs::rename(p.0.join("main.timeline.json"), &inner).unwrap();
    let mut inner_doc: Value = serde_json::from_slice(&std::fs::read(&inner).unwrap()).unwrap();
    inner_doc["resources"]["clip"] = "../media/source.mp4".into();
    std::fs::write(&inner, inner_doc.to_string()).unwrap();
    p.document(
        "timelines/inner.timeline.json",
        json!({"visual":[{"items":[{"id":"nested","kind":"timeline","src":"clip","duration":0.25}]}]}),
    );
    for args in [
        &["project", "check"][..],
        &["project", "check", "--cost"][..],
    ] {
        let result = p.run(args);
        assert_eq!(result["warnings"][0]["code"], "unsupported-hdr", "{result}");
    }
}

#[test]
fn missing_media_and_unknown_props_name_authored_fields() {
    let p = Project::new("diagnostics");
    for kind in ["video", "image"] {
        for gain in [0, 1] {
            p.document(
                "media/missing.mp4",
                json!({"visual":[{"gain":gain,"items":[{"id":"shot","kind":kind,"src":"clip","duration":1}]}]}),
            );
            for args in [
                &["project", "check"][..],
                &["timeline", "check", "main.timeline.json"][..],
            ] {
                let result = p.run(args);
                assert_eq!(result["status"], "error");
                let message = result.to_string();
                assert!(
                    message.contains("resources.clip") && message.contains("not found"),
                    "{message}"
                );
                assert!(!message.contains("resource r0"), "{message}");
            }
        }
    }
    std::fs::rename(
        p.0.join("main.timeline.json"),
        p.0.join("timelines/inner.timeline.json"),
    )
    .unwrap();
    p.document(
        "timelines/inner.timeline.json",
        json!({"visual":[{"items":[{"id":"nested","kind":"timeline","src":"clip","duration":1}]}]}),
    );
    let result = p.run(&["timeline", "check", "main.timeline.json"]);
    let message = result["message"].as_str().unwrap();
    assert!(
        message.contains("inner.timeline.json: resources.clip") && !message.contains("resource r0"),
        "{message}"
    );
    let source = indoc! {r#"
        export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
        export const controls = { props: { name: string({default: "Ada"}), title: string({default: "Engineer"}) } };
        export default function Scene({name}) { return <Text>{name}</Text>; }
    "#};
    std::fs::write(p.0.join("components/title.motion.tsx"), source).unwrap();
    p.document("components/title.motion.tsx", json!({"visual":[{"items":[{"id":"lower","kind":"motion","component":"clip","duration":1,"props":{"nme":"Ada"}}]}]}));
    for args in [
        &[
            "motion",
            "check",
            "components/title.motion.tsx",
            "--props",
            r#"{"nme":"Ada"}"#,
        ][..],
        &["timeline", "check", "main.timeline.json"][..],
    ] {
        let result = p.run(args);
        assert_eq!(result["status"], "error");
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("available: name, title"),
            "{result}"
        );
    }
}
