//! `valle motion check` warns where the scene's structure follows values that change, so that
//! more than a tenth of its frames trace it again.

use std::process::Command;

use indoc::indoc;
use serde_json::Value;

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
