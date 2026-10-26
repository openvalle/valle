//! Authoring errors retain their source module and output clock through every delivery entry point.

use std::{fs, path::Path, process::Command};

use serde_json::{Value, json};

fn run(root: &Path, args: &[&str], succeeds: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(root)
        .env("VALLE_HOME", root.join("home"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        succeeds,
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn preflight_and_delivery_locate_late_opacity_errors_in_imported_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("card.tsx"),
        r##"export function Card() {
  const time = useCurrentTime();
  return <View key="card" style={{ width: 32, height: 32,
    backgroundColor: "#ffffff", opacity: time.seconds < 1 ? 0.5 : -0.125 }} />;
}
"##,
    )
    .unwrap();
    let source = r#"import { Card } from "./card.tsx";
export const composition = { width: 64, height: 36, fps: 4, duration: 2 };
export default function Main() { return <Scene><Card /></Scene>; }
"#;
    fs::write(root.join("main.motion.tsx"), source).unwrap();
    fs::write(root.join("safe.motion.tsx"), r##"export const composition = { width: 64, height: 36, fps: 4, duration: 2 };
export default function Safe() { return <Scene><View key="card" style={{width:32,height:32,backgroundColor:"#ffffff"}} /></Scene>; }
"##).unwrap();
    fs::write(
        root.join("timeline.json"),
        json!({"canvas":{"width":64,"height":36,"fps":4},
        "resources":{"safe":"safe.motion.tsx","main":"main.motion.tsx"},
        "tracks":{"visual":[{"clips":[
            {"kind":"motion","component":"safe","start":0,"duration":2},
            {"kind":"motion","component":"main","start":2,"duration":2}]}]}})
        .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("project.json"),
        r#"{"name":"diagnostics","timeline":"timeline.json"}"#,
    )
    .unwrap();

    run(root, &["project", "check"], true);
    let standalone = run(
        root,
        &["motion", "check", "main.motion.tsx", "--cost"],
        false,
    );
    let project = run(root, &["project", "check", "--cost"], false);
    for (report, frame) in [(&standalone, 4), (&project, 12)] {
        let failures = report["cost"]["failures"].as_array().unwrap();
        assert_eq!(failures.len(), 4, "{report}");
        assert_eq!(failures[0]["frame"], frame, "{report}");
        assert_eq!(failures[0]["seconds"], json!(f64::from(frame) / 4.0));
        let source = &failures[0]["source"];
        assert!(
            source["path"].as_str().unwrap().ends_with("card.tsx"),
            "{report}"
        );
        assert_eq!(source["line"], 4, "{report}");
        assert!(
            !source["expansionStack"].as_array().unwrap().is_empty(),
            "{report}"
        );
        assert!(
            failures[0]["error"]
                .as_str()
                .unwrap()
                .contains("opacity: -0.125"),
            "{report}"
        );
    }
    for (args, frame, source) in [
        (
            vec!["motion", "render", "main.motion.tsx", "--frame", "4"],
            4,
            &standalone["cost"]["failures"][0]["source"],
        ),
        (
            vec!["timeline", "render", "timeline.json", "--frame", "12"],
            12,
            &project["cost"]["failures"][0]["source"],
        ),
        (
            vec!["project", "render", "--frame", "12"],
            12,
            &project["cost"]["failures"][0]["source"],
        ),
    ] {
        let report = run(
            root,
            &[args.as_slice(), &["--backend", "raster", "-o", "bad.png"]].concat(),
            false,
        );
        assert_eq!(report["error"]["frame"], frame, "{report}");
        assert_eq!(&report["error"]["source"], source, "{report}");
        let message = report["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("opacity: -0.125") && message.contains("card.tsx:4:"),
            "{report}"
        );
        assert!(!root.join("bad.png").exists());
    }
}

#[test]
fn instance_opacity_failure_locates_the_template_expression_and_actual_value() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("instances.motion.tsx"),
        r##"export const composition = {width:64,height:36,fps:4,duration:2};
const dots = Array.from({length:72}, (_, i) => ({id:`dot-${i}`, x:i%8}));
export default function Dots() {
  const time = useCurrentTime();
  return <Scene>{dots.map(dot => <View key={dot.id} className="absolute"
    style={{left:dot.x,top:0,width:2,height:2,backgroundColor:"#ffffff",
      opacity:time.seconds < 1 ? 0.5 : -0.125}} />)}</Scene>;
}
"##,
    )
    .unwrap();
    let report = run(
        root,
        &["motion", "check", "instances.motion.tsx", "--cost"],
        false,
    );
    let failure = &report["cost"]["failures"][0];
    assert_eq!(failure["frame"], 4, "{report}");
    assert_eq!(failure["source"]["line"], 7, "{report}");
    let message = failure["error"].as_str().unwrap();
    assert!(
        message.contains("instance group 0") && message.contains("opacity: -0.125"),
        "{report}"
    );
}

#[test]
fn children_and_duplicate_keys_keep_compile_locations_and_actionable_messages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (source, expected) in [
        (
            r#"export default function Title({ children }) { return <Text>{children}</Text>; }"#,
            "unsupported text children",
        ),
        (
            r#"export default function Title() { return <Scene><View key="same"/><View key="same"/></Scene>; }"#,
            "duplicate node key",
        ),
    ] {
        fs::write(root.join("bad.motion.tsx"), source).unwrap();
        let report = run(root, &["motion", "check", "bad.motion.tsx"], false);
        let diagnostics = report["error"]["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic["message"]
                .as_str()
                .unwrap()
                .contains(expected)
                && diagnostic["span"]["line"] == 1),
            "{report}"
        );
    }
}
