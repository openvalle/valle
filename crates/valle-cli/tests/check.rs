//! `valle motion check` warns where the scene's structure follows values that change, so that
//! more than a tenth of its frames trace it again.

use std::process::Command;

use indoc::indoc;
use serde_json::{Value, json};

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

#[test]
fn commands_reject_wrong_entry_suffixes_and_suggest_the_right_group() {
    let dir = std::env::temp_dir().join(format!("valle-entry-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (file, group, suffix, hint) in [
        ("Card.tsx", "motion", ".motion.tsx", None),
        ("data.json", "timeline", ".timeline.json", None),
        (
            "scene.motion.tsx",
            "timeline",
            ".timeline.json",
            Some("motion"),
        ),
        (
            "cut.timeline.json",
            "motion",
            ".motion.tsx",
            Some("timeline"),
        ),
    ] {
        // Existing files make Studio exercise entry validation too, without starting a server.
        std::fs::write(dir.join(file), "not parsed").unwrap();
        let commands: &[&str] = match group {
            "timeline" => &["check", "render", "query", "edit", "studio"],
            _ => &["check", "render", "studio"],
        };
        for command in commands {
            let mut run = Command::new(VALLE);
            run.current_dir(&dir).args([group, command, file, "--json"]);
            if *command == "render" {
                run.args(["-o", "unused.mp4"]);
            } else if *command == "edit" {
                run.args(["--op", "{}"]);
            }
            let out = run.output().unwrap();
            assert!(!out.status.success(), "{group} {command} {file}");
            let result: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(result["code"], "usage", "{result}");
            assert!(
                result["message"].as_str().unwrap().contains(suffix),
                "{result}"
            );
            match hint {
                Some(group) => {
                    let command = if group == "motion" && matches!(*command, "query" | "edit") {
                        "check"
                    } else {
                        command
                    };
                    assert_eq!(result["hint"], format!("valle {group} {command} {file}"))
                }
                None => assert!(result.get("hint").is_none(), "{result}"),
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// `valle motion check` of `scene`, as JSON.
fn check(name: &str, scene: &str) -> Value {
    let dir = std::env::temp_dir().join(format!("valle-check-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("scene.motion.tsx");
    std::fs::write(&file, scene).expect("written");
    let out = Command::new(VALLE)
        .args(["motion", "check"])
        .arg(&file)
        .arg("--json")
        .output()
        .expect("valle runs");
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_slice(&out.stdout).expect("JSON")
}

#[test]
fn a_structure_that_follows_time_is_warned_of() {
    let scene = indoc! {r##"
        export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
        export default function Scene() {
          const { seconds } = useCurrentTime();
          const bars = Array.from({ length: 1 + Math.floor(seconds * 20) }, (_, i) => i);
          return (
            <View className="h-full w-full flex">
              {bars.map((i) => <View key={i} style={{ width: 2, height: 10, backgroundColor: "#fff" }} />)}
            </View>
          );
        }
    "##};
    let out = check("bars", scene);
    assert_eq!(out["status"], "ok", "{out}");
    let message = out["warnings"][0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("trace the scene again"), "{out}");
    assert!(message.contains("scene.motion.tsx:"), "{out}");
}

#[test]
fn a_structure_that_holds_is_not() {
    let scene = indoc! {r#"
        export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
        export default function Scene() {
          const { progress } = useCurrentTime();
          return <View className="h-full w-full" style={{ opacity: progress }} />;
        }
    "#};
    let out = check("still", scene);
    assert_eq!(out["status"], "ok", "{out}");
    // No warnings: the result has none to give, so it has no field for them.
    assert!(out.get("warnings").is_none(), "{out}");
}

#[test]
fn an_error_in_later_frames_is_reported_with_its_first_frame_and_count() {
    let out = check(
        "late-error",
        indoc! {r#"
            export const composition = {width:64,height:36,fps:30,duration:2};
            export default function Scene() {
              const {seconds} = useCurrentTime();
              return <View style={{width:seconds < 1 ? 20 : 'broken'}}/>;
            }
        "#},
    );
    assert_eq!(out["status"], "error", "{out}");
    assert_eq!(out["problems"][0]["first"], 30, "{out}");
    assert_eq!(out["problems"][0]["count"], 30, "{out}");
}

#[test]
fn timeline_check_requires_audio_only_for_enabled_audio_clips_including_nested_edits() {
    let dir = std::env::temp_dir().join(format!("valle-check-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/resources/video/quadrants.mp4");
    std::fs::copy(source, dir.join("silent.mp4")).unwrap();
    let run = |command: &str| {
        let mut cmd = Command::new(VALLE);
        cmd.current_dir(&dir)
            .args(["timeline", command, "edit.timeline.json", "--json"]);
        if command == "render" {
            cmd.args(["-o", "mix.wav"]);
        }
        let out = cmd.output().unwrap();
        let result: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(out.status.success(), result["status"] == "ok", "{result}");
        result
    };
    let mut doc = json!({
        "canvas": {"width":96,"height":64,"fps":4},
        "resources": {"source":"silent.mp4"},
        "tracks": {"visual":[{"clips":[{"kind":"video","src":"source","start":0,"duration":1}]}]}
    });
    let write =
        |value: &Value| std::fs::write(dir.join("edit.timeline.json"), value.to_string()).unwrap();
    write(&doc);
    assert_eq!(run("check")["status"], "ok");
    doc["tracks"]["audio"] = json!([{"clips":[{"src":"source","start":0,"duration":1}]}]);
    write(&doc);
    for command in ["check", "render"] {
        let result = run(command);
        assert_eq!(result["status"], "error", "{command}: {result}");
        assert!(
            result["message"].as_str().unwrap().contains("has no sound"),
            "{result}"
        );
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("tracks.audio[0].clips[0]"),
            "{result}"
        );
    }
    doc["tracks"]["audio"][0]["clips"][0]["enabled"] = false.into();
    write(&doc);
    assert_eq!(run("check")["status"], "ok");
    doc["tracks"]["audio"][0]["clips"][0]["enabled"] = true.into();
    std::fs::write(dir.join("inner.timeline.json"), doc.to_string()).unwrap();
    let parent = json!({
        "canvas":{"width":96,"height":64,"fps":4},
        "resources":{"inner":"inner.timeline.json"},
        "tracks":{"visual":[{"clips":[{"kind":"timeline","src":"inner","start":0,"duration":1}]}]}
    });
    write(&parent);
    let result = run("check");
    assert!(
        result["message"].as_str().unwrap().contains("has no sound"),
        "{result}"
    );
    // A real audio stream, without running the renderer or depending on a CLI FFmpeg binary.
    use valle_native::media::{self, Codec, Encoding, Options};
    let options = Options {
        size: [96, 64],
        fps: [4, 1],
        codec: Codec::H264,
        encoding: Encoding::Software,
        bitrate: None,
        sound: true,
        surfaces: false,
    };
    let mut writer = media::writer(&dir.join("audible.mp4"), &options).unwrap();
    for _ in 0..4 {
        writer.push(&vec![255; 96 * 64 * 4]).unwrap();
    }
    writer.sound(&vec![0.25; 48_000 * 2]).unwrap();
    writer.finish().unwrap();
    drop(writer);
    doc["resources"]["source"] = "audible.mp4".into();
    write(&doc);
    assert_eq!(run("check")["status"], "ok");
    assert_eq!(run("render")["status"], "ok");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn motion_and_nested_timeline_checks_report_hdr_as_a_warning_in_text_and_json() {
    let dir = std::env::temp_dir().join(format!("valle-check-hdr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources/video");
    let scene = indoc! {r#"
        export const composition = {width:96,height:64,fps:4,duration:1};
        export const controls = {resources:{
          a:resource({kind:"video",default:"source.mp4"}),
          b:resource({kind:"video",default:"source.mp4"}),
        }};
        export default function Scene() {
          return <View><Video src="resource://a"/><Video src="resource://b"/></View>;
        }
    "#};
    std::fs::write(dir.join("scene.motion.tsx"), scene).unwrap();
    let inner = json!({
        "canvas":{"width":96,"height":64,"fps":4}, "resources":{"source":"source.mp4"},
        "tracks":{"visual":[{"clips":[{"kind":"video","src":"source","start":0,"duration":1}]}]}
    });
    std::fs::write(dir.join("inner.timeline.json"), inner.to_string()).unwrap();
    let parent = json!({
        "canvas":{"width":96,"height":64,"fps":4},"resources":{"inner":"inner.timeline.json"},
        "tracks":{"visual":[{"clips":[{"kind":"timeline","src":"inner","start":0,"duration":1}]}]}
    });
    std::fs::write(dir.join("edit.timeline.json"), parent.to_string()).unwrap();
    for (file, transfer) in [
        ("pq.mp4", Some("PQ")),
        ("hlg.mp4", Some("HLG")),
        ("quadrants.mp4", None),
    ] {
        std::fs::copy(root.join(file), dir.join("source.mp4")).unwrap();
        for (group, entry) in [
            ("motion", "scene.motion.tsx"),
            ("timeline", "edit.timeline.json"),
        ] {
            for json in [false, true] {
                let mut cmd = Command::new(VALLE);
                cmd.current_dir(&dir).args([group, "check", entry]);
                if json {
                    cmd.arg("--json");
                }
                let out = cmd.output().unwrap();
                assert!(out.status.success(), "{out:?}");
                let text = String::from_utf8(out.stdout).unwrap();
                if json {
                    let result: Value = serde_json::from_str(&text).unwrap();
                    match transfer {
                        Some(t) => {
                            assert_eq!(result["warnings"].as_array().unwrap().len(), 1, "{text}");
                            assert_eq!(result["warnings"][0]["code"], "unsupported-hdr");
                            assert!(
                                result["warnings"][0]["message"]
                                    .as_str()
                                    .unwrap()
                                    .contains(t),
                                "{text}"
                            );
                            assert!(
                                result["warnings"][0]["file"]
                                    .as_str()
                                    .unwrap()
                                    .ends_with("source.mp4")
                            );
                        }
                        None => assert!(result.get("warnings").is_none(), "{text}"),
                    }
                } else if let Some(t) = transfer {
                    assert!(
                        text.contains("warning:") && text.contains(&format!("HDR ({t})")),
                        "{text}"
                    );
                } else {
                    assert!(!text.contains("warning:"), "{text}");
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
