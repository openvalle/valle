//! motion-review: review final screen motion and keep slow or motion-blurred controls clean.

use std::{path::Path, process::Command};

use serde_json::Value;

const STROBE: &str =
    include_str!("../../valle-compiler/tests/fixtures/motion/composition/strobe-review.motion.tsx");

fn run(dir: &Path, source: &str, extra: &[&str]) -> (bool, Value) {
    std::fs::write(dir.join("strobe.motion.tsx"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir)
        .args(["--json", "motion", "review", "strobe.motion.tsx"])
        .args(extra)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), value)
}

#[test]
fn review_reports_continuous_strobing_and_accepts_two_controls() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, report) = run(dir.path(), STROBE, &[]);
    assert!(ok, "{report}");
    assert_eq!(report["framesAnalyzed"], 60);
    assert!(report.get("trajectories").is_none());
    let issues = report["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|issue| {
            issue["code"] == "strobe"
                && issue["node"] == "bar"
                && issue["peakDisplacementPxPerFrame"]
                    .as_f64()
                    .is_some_and(|value| (value - 40.0).abs() < 0.5)
                && issue["thresholdPxPerFrame"]
                    .as_f64()
                    .is_some_and(|value| (value - 5.0).abs() < 0.5)
        }),
        "{report}"
    );

    let broad = STROBE.replace("width: 20", "width: 600");
    let (ok, report) = run(dir.path(), &broad, &[]);
    assert!(ok, "{report}");
    assert!(
        !report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "strobe"),
        "{report}"
    );

    let slow = STROBE.replace("* 1200", "* 60");
    let (ok, report) = run(dir.path(), &slow, &[]);
    assert!(ok, "{report}");
    assert_eq!(report["issues"].as_array().unwrap().len(), 0, "{report}");

    let blurred = STROBE.replace(
        "backgroundColor: \"#ffffff\", translate:",
        "backgroundColor: \"#ffffff\", motionBlur: \"auto\", translate:",
    );
    let (ok, report) = run(dir.path(), &blurred, &[]);
    assert!(ok, "{report}");
    assert_eq!(report["issues"].as_array().unwrap().len(), 0, "{report}");

    let hidden = STROBE.replace(
        "backgroundColor: \"#ffffff\", translate:",
        "backgroundColor: \"#ffffff\", opacity: 0, translate:",
    );
    let (ok, report) = run(dir.path(), &hidden, &[]);
    assert!(ok, "{report}");
    assert_eq!(report["issues"].as_array().unwrap().len(), 0, "{report}");
}

#[test]
fn review_flags_sustained_linear_motion_but_not_eased_motion_or_a_sliver() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Motion(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <View key="lead" style={{position:"absolute",left:20,top:100,width:80,height:80,
      backgroundColor:"#ffffff",translate:point(ctx.localFrame < 20 ? ctx.localFrame * 5 : 100,0)}} />
  </Scene>;
}
"##;

    let (ok, report) = run(dir.path(), source, &[]);
    assert!(ok, "{report}");
    assert!(
        report["issues"].as_array().unwrap().iter().any(|issue| {
            issue["code"] == "linear_motion"
                && issue["node"] == "lead"
                && issue["startFrame"] == 0
                && issue["endFrame"] == 20
        }),
        "{report}"
    );

    let eased = source.replace(
        "ctx.localFrame * 5",
        "ctx.localFrame * ctx.localFrame * 0.25",
    );
    let (ok, report) = run(dir.path(), &eased, &[]);
    assert!(ok, "{report}");
    assert!(
        !report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "linear_motion" }),
        "{report}"
    );

    let sliver = source
        .replace("left:20", "left:-995")
        .replace("width:80", "width:1000")
        .replace(
            "point(ctx.localFrame < 20 ? ctx.localFrame * 5 : 100,0)",
            "point(0,ctx.localFrame < 20 ? ctx.localFrame * 5 : 100)",
        );
    let (ok, report) = run(dir.path(), &sliver, &[]);
    assert!(ok, "{report}");
    assert!(
        !report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "linear_motion" }),
        "{report}"
    );
}

#[test]
fn review_checks_observed_stagger_for_numbered_moving_groups() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Stagger(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <View key="bar-0" style={{position:"absolute",left:20,top:40,width:80,height:40,
      backgroundColor:"#ffffff",translate:point(clamp(ctx.localFrame - 0,0,100)*5,0)}} />
    <View key="bar-1" style={{position:"absolute",left:20,top:100,width:80,height:40,
      backgroundColor:"#ffffff",translate:point(clamp(ctx.localFrame - 6,0,100)*5,0)}} />
    <View key="bar-2" style={{position:"absolute",left:20,top:160,width:80,height:40,
      backgroundColor:"#ffffff",translate:point(clamp(ctx.localFrame - 12,0,100)*5,0)}} />
  </Scene>;
}
"##;
    let has_stagger = |report: &Value| {
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|issue| issue["code"] == "stagger_timing")
            .cloned()
    };
    let (ok, late) = run(dir.path(), source, &[]);
    assert!(ok, "{late}");
    let issue = has_stagger(&late).unwrap_or_else(|| panic!("{late}"));
    assert_eq!(issue["node"], "bar-0");
    assert_eq!(issue["otherNode"], "bar-2");
    assert_eq!(issue["startFrame"], 1);
    assert_eq!(issue["endFrame"], 13);

    let valid = source
        .replace("ctx.localFrame - 6", "ctx.localFrame - 2")
        .replace("ctx.localFrame - 12", "ctx.localFrame - 4");
    let (ok, report) = run(dir.path(), &valid, &[]);
    assert!(ok, "{report}");
    assert!(has_stagger(&report).is_none(), "{report}");

    let simultaneous = source
        .replace("ctx.localFrame - 6", "ctx.localFrame - 0")
        .replace("ctx.localFrame - 12", "ctx.localFrame - 0");
    let (ok, report) = run(dir.path(), &simultaneous, &[]);
    assert!(ok, "{report}");
    assert!(has_stagger(&report).is_none(), "{report}");

    let fast = source
        .replace("fps: 30, duration: 2", "fps: 60, duration: 1")
        .replace("ctx.localFrame - 6", "ctx.localFrame - 1")
        .replace("ctx.localFrame - 12", "ctx.localFrame - 2");
    let (ok, report) = run(dir.path(), &fast, &[]);
    assert!(ok, "{report}");
    assert!(has_stagger(&report).is_some(), "{report}");

    let reversed = source
        .replace("ctx.localFrame - 0", "ctx.localFrame - 99")
        .replace("ctx.localFrame - 12", "ctx.localFrame - 0")
        .replace("ctx.localFrame - 99", "ctx.localFrame - 12");
    let (ok, report) = run(dir.path(), &reversed, &[]);
    assert!(ok, "{report}");
    assert!(has_stagger(&report).is_some(), "{report}");

    let mixed = source
        .replace("ctx.localFrame - 6", "ctx.localFrame - 99")
        .replace("ctx.localFrame - 12", "ctx.localFrame - 6")
        .replace("ctx.localFrame - 99", "ctx.localFrame - 12");
    let (ok, report) = run(dir.path(), &mixed, &[]);
    assert!(ok, "{report}");
    assert!(has_stagger(&report).is_none(), "{report}");
}

#[test]
fn review_exports_final_screen_trajectory_and_resets_derivatives_after_hidden_gap() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, report) = run(dir.path(), STROBE, &["--trajectories"]);
    assert!(ok, "{report}");
    let points = report["trajectories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["node"] == "bar")
        .unwrap()["points"]
        .as_array()
        .unwrap();
    let at = |frame: u64| points.iter().find(|point| point["frame"] == frame).unwrap();
    assert!(at(0).get("velocityPxPerSecond").is_none());
    assert!((at(1)["velocityPxPerSecond"][0].as_f64().unwrap() - 1200.0).abs() < 1.0e-6);
    assert!(at(2)["accelerationPxPerSecond2"][0].as_f64().unwrap().abs() < 1.0e-6);
    assert!(at(30).get("velocityPxPerSecond").is_none());
}

#[test]
fn review_renders_trajectory_sheet_from_the_compiled_scene() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, report) = run(
        dir.path(),
        STROBE,
        &["--trajectory-sheet", "trajectory.png"],
    );
    assert!(ok, "{report}");
    assert!(report.get("trajectories").is_none());
    let sheet = &report["trajectorySheet"];
    assert_eq!(sheet["frames"][0], 0);
    assert_eq!(sheet["frames"][11], 59);
    assert_eq!(sheet["columns"], 4);
    assert_eq!(sheet["cellSize"], serde_json::json!([320, 180]));
    assert_eq!(sheet["colors"]["bar"], "#00dcff");
    let image = image::open(dir.path().join("trajectory.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (1280, 540));
    // The source fixture is grayscale, so this color can only come from the path overlay.
    assert!(
        image.pixels().any(|pixel| {
            pixel[0] == 0 && pixel[1] == 220 && pixel[2] == 255 && pixel[3] == 255
        })
    );

    let (ok, second) = run(
        dir.path(),
        STROBE,
        &["--trajectory-sheet", "trajectory.png"],
    );
    assert!(!ok);
    assert!(
        second["error"]["message"]
            .as_str()
            .unwrap()
            .contains("refusing to overwrite")
    );
}

#[test]
fn review_rejects_a_frame_budget_that_would_silently_skip_samples() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, report) = run(dir.path(), STROBE, &["--max-frames", "30"]);
    assert!(!ok);
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--max-frames")
    );
}

#[test]
fn review_uses_bound_motion_props_for_the_trajectory() {
    let dir = tempfile::tempdir().unwrap();
    let source = STROBE
        .replace(
            "export default function Strobe(ctx)",
            "export const controls = { props: { speed: number({ default: 1200, min: 0, max: 1200 }) } };\nexport default function Strobe(ctx, props)",
        )
        .replace("* 1200", "* props.speed");
    std::fs::write(dir.path().join("props.json"), r#"{"speed":60}"#).unwrap();
    let (ok, report) = run(dir.path(), &source, &["--props", "props.json"]);
    assert!(ok, "{report}");
    assert!(report["issues"].as_array().unwrap().is_empty(), "{report}");
}

#[test]
fn review_uses_final_layout_motion_when_a_sibling_pushes_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function LayoutPush(ctx) {
  return <Scene style={{width:640,height:360}}>
    <View key="row" style={{display:"flex",width:640,height:120}}>
      <View key="spacer" style={{width:clamp(ctx.seconds,0,0.5)*900,height:20}}/>
      <View key="bar" style={{width:20,height:120,backgroundColor:"#ffffff"}}/>
    </View>
  </Scene>;
}
"##;
    let (ok, report) = run(dir.path(), source, &[]);
    assert!(ok, "{report}");
    assert!(
        report["issues"].as_array().unwrap().iter().any(|issue| {
            issue["code"] == "strobe"
                && issue["node"]
                    .as_str()
                    .is_some_and(|node| node.ends_with("bar"))
                && issue["peakDisplacementPxPerFrame"]
                    .as_f64()
                    .is_some_and(|value| (value - 30.0).abs() < 0.5)
        }),
        "{report}"
    );
}

#[test]
fn review_reports_short_text_dwell_and_accepts_long_hidden_or_offscreen_controls() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Readability(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <Text key="caption" style={{position:"absolute",left:100,top:100,fontSize:32,
      color:"#ffffff",opacity:ctx.localFrame >= 10 && ctx.localFrame < 15 ? 1 : 0}}>
      Read these three words
    </Text>
  </Scene>;
}
"##;
    let (ok, report) = run(dir.path(), source, &[]);
    assert!(ok, "{report}");
    assert!(
        report["issues"].as_array().unwrap().iter().any(|issue| {
            issue["code"] == "text_readability"
                && issue["node"] == "caption"
                && issue["startFrame"] == 10
                && issue["endFrame"] == 14
                && issue["wordCount"] == 4
                && issue["visibleSeconds"]
                    .as_f64()
                    .is_some_and(|seconds| (seconds - 5.0 / 30.0).abs() < 1.0e-9)
                && issue["recommendedSeconds"] == 1.5
        }),
        "{report}"
    );

    let (ok, partial) = run(dir.path(), &source.replace("left:100", "left:-260"), &[]);
    assert!(ok, "{partial}");
    assert!(
        partial["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "text_readability" && issue["node"] == "caption"),
        "{partial}"
    );

    let changing = source.replace(
        "Read these three words",
        "{ctx.localFrame < 13 ? 'One two' : 'Three four five six'}",
    );
    let (ok, changed) = run(dir.path(), &changing, &[]);
    assert!(ok, "{changed}");
    let text_issues = changed["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue["code"] == "text_readability")
        .collect::<Vec<_>>();
    assert_eq!(text_issues.len(), 2, "{changed}");
    assert_eq!(
        (
            text_issues[0]["startFrame"].as_u64(),
            text_issues[0]["endFrame"].as_u64()
        ),
        (Some(10), Some(12))
    );
    assert_eq!(
        (
            text_issues[1]["startFrame"].as_u64(),
            text_issues[1]["endFrame"].as_u64()
        ),
        (Some(13), Some(14))
    );

    for control in [
        source.replace("ctx.localFrame < 15", "ctx.localFrame < 56"),
        source.replace("ctx.localFrame >= 10 && ctx.localFrame < 15 ? 1 : 0", "0"),
    ] {
        let (ok, report) = run(dir.path(), &control, &[]);
        assert!(ok, "{report}");
        assert!(report["issues"].as_array().unwrap().is_empty(), "{report}");
    }
    let (ok, offscreen) = run(dir.path(), &source.replace("left:100", "left:1000"), &[]);
    assert!(ok, "{offscreen}");
    assert!(
        offscreen["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "text_out_of_frame" && issue["node"] == "caption" }),
        "{offscreen}"
    );
    assert!(
        !offscreen["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "text_readability" }),
        "{offscreen}"
    );
}

#[test]
fn review_coalesces_text_overlap_and_clears_it_when_layout_separates_the_boxes() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Overlap() {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <Text key="first" style={{position:"absolute",left:100,top:100,fontSize:32,color:"#ffffff"}}>Alpha words</Text>
    <Text key="second" style={{position:"absolute",left:110,top:105,fontSize:32,color:"#ffffff"}}>Beta words</Text>
  </Scene>;
}
"##;
    let (ok, report) = run(dir.path(), source, &[]);
    assert!(ok, "{report}");
    assert!(
        report["issues"].as_array().unwrap().iter().any(|issue| {
            issue["code"] == "text_overlap"
                && issue["node"] == "first"
                && issue["otherNode"] == "second"
                && issue["startFrame"] == 0
                && issue["endFrame"] == 59
        }),
        "{report}"
    );

    let (ok, separated) = run(dir.path(), &source.replace("left:110", "left:400"), &[]);
    assert!(ok, "{separated}");
    assert!(
        separated["issues"].as_array().unwrap().is_empty(),
        "{separated}"
    );

    let timed = source
        .replace("function Overlap()", "function Overlap(ctx)")
        .replace("left:110", "left:ctx.localFrame < 30 ? 110 : 400");
    let (ok, timed_report) = run(dir.path(), &timed, &[]);
    assert!(ok, "{timed_report}");
    let overlaps = timed_report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue["code"] == "text_overlap")
        .collect::<Vec<_>>();
    assert_eq!(overlaps.len(), 1, "{timed_report}");
    assert_eq!(overlaps[0]["startFrame"], 0);
    assert_eq!(overlaps[0]["endFrame"], 29);
}

#[test]
fn review_hints_when_prominent_actions_compete_with_reading_or_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 1 };
export default function CompetingActions(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <Text key="caption" style={{position:"absolute",left:60,top:30,fontSize:28,color:"#ffffff"}}>Read this title</Text>
    <View key="left" style={{position:"absolute",left:20 + ctx.seconds * 360,top:160,width:80,height:80,backgroundColor:"#ff0000"}}/>
    <View key="right" style={{position:"absolute",left:350 + ctx.seconds * 360,top:160,width:80,height:80,backgroundColor:"#0000ff"}}/>
  </Scene>;
}
"##;
    let (ok, report) = run(dir.path(), source, &[]);
    assert!(ok, "{report}");
    let issues = report["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|issue| {
            issue["code"] == "motion_while_reading"
                && issue["node"] == "caption"
                && issue["otherNode"] == "left"
                && issue["startFrame"] == 1
        }),
        "{report}"
    );
    assert!(
        issues.iter().any(|issue| {
            issue["code"] == "simultaneous_main_actions"
                && issue["node"] == "left"
                && issue["otherNode"] == "right"
                && issue["startFrame"] == 1
        }),
        "{report}"
    );

    let (ok, no_caption) = run(
        dir.path(),
        &source.replace(
            "fontSize:28,color:\"#ffffff\"",
            "fontSize:28,color:\"#ffffff\",opacity:0",
        ),
        &[],
    );
    assert!(ok, "{no_caption}");
    assert!(
        !no_caption["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "motion_while_reading" }),
        "{no_caption}"
    );

    let (ok, one_action) = run(
        dir.path(),
        &source.replace("left:350 + ctx.seconds * 360", "left:350"),
        &[],
    );
    assert!(ok, "{one_action}");
    assert!(
        !one_action["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["code"] == "simultaneous_main_actions" }),
        "{one_action}"
    );

    let (ok, slow) = run(
        dir.path(),
        &source.replace("ctx.seconds * 360", "ctx.seconds * 30"),
        &[],
    );
    assert!(ok, "{slow}");
    assert!(
        !slow["issues"].as_array().unwrap().iter().any(|issue| {
            issue["code"] == "motion_while_reading" || issue["code"] == "simultaneous_main_actions"
        }),
        "{slow}"
    );

    let sliver = source.replace(
        "left:20 + ctx.seconds * 360,top:160,width:80",
        "left:-995,top:160 + ctx.seconds * 360,width:1000",
    );
    let (ok, sliver) = run(dir.path(), &sliver, &[]);
    assert!(ok, "{sliver}");
    assert!(
        !sliver["issues"].as_array().unwrap().iter().any(|issue| {
            (issue["code"] == "motion_while_reading" && issue["otherNode"] == "left")
                || issue["code"] == "simultaneous_main_actions"
        }),
        "{sliver}"
    );
}
