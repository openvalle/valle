#![cfg(feature = "motion")]
//! Sequence provides compile-time scheduling within a Timeline.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, MotionValue, ResolvedSignals, motion_context_at, phase_windows};
use valle_timeline::FrameRate;

fn messages(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid Sequence must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn progress(source: &str, fps: FrameRate, frame: u32, props: BTreeMap<String, MotionValue>) -> f64 {
    let compiled = compile_motion(source).expect("Sequence compiles");
    let artifact = &compiled.artifact;
    let props = valle_motion::resolve_props(&artifact.controls, &props).expect("props");
    let windows = phase_windows(&artifact.controls.phase_spec(), 10_000);
    let ctx = motion_context_at(frame, &windows, fps).expect("frame");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
            unit: None,
            viewport: Some((1920.0, 1080.0)),
        },
    )
    .expect("evaluate");
    let expr = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == "opacity")
        .and_then(|style| match style.value {
            valle_motion::StyleValue::Expr { expr } => Some(expr),
            _ => None,
        })
        .expect("dynamic opacity");
    let MotionValue::Number(value) = values[expr.0 as usize] else {
        panic!("progress is numeric");
    };
    value
}

fn style_number(source: &str, frame: u32, property: &str) -> f64 {
    let compiled = compile_motion(source).expect("Motion source compiles");
    let artifact = &compiled.artifact;
    let props =
        valle_motion::resolve_props(&artifact.controls, &Default::default()).expect("props");
    let windows = phase_windows(&artifact.controls.phase_spec(), 10_000);
    let ctx = motion_context_at(frame, &windows, FrameRate::new(30, 1).unwrap()).expect("frame");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
            unit: None,
            viewport: Some((1920.0, 1080.0)),
        },
    )
    .expect("evaluate");
    let value = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == property)
        .expect("style exists");
    match value.value {
        valle_motion::StyleValue::Static {
            value: MotionValue::Number(value),
        } => value,
        valle_motion::StyleValue::Expr { expr } => {
            let MotionValue::Number(value) = values[expr.0 as usize] else {
                panic!("style is numeric");
            };
            value
        }
        ref other => panic!("numeric style, got {other:?}"),
    }
}

const SECONDS_SEQUENCE: &str = r##"
const intro = defineSequence({
  title: stage({ duration: seconds(0.6) }),
  cards: stage({ after: "title", overlap: seconds(0.2), duration: seconds(1.0) }),
});
export default function P(ctx) {
  return <View key="v" style={{ opacity: stageProgress(ctx.localFrame, ctx.fps, intro.cards) }} />;
}
"##;

#[test]
fn named_stages_compile_to_the_same_artifact_as_manual_arithmetic() {
    let sequence = compile_motion(SECONDS_SEQUENCE).expect("Sequence").artifact;
    let manual = compile_motion(
        r##"
export default function P(ctx) {
  return <View key="v" style={{ opacity: clamp(((ctx.localFrame * ctx.fps.den / ctx.fps.num) - (0.6 - 0.2)) / 1.0, 0, 1) }} />;
}
"##,
    )
    .expect("manual")
    .artifact;
    assert_eq!(
        sequence, manual,
        "Sequence must be syntax sugar over the existing Expr arena"
    );
}

#[test]
fn seconds_stages_are_bit_identical_at_shared_sample_times() {
    for frame in [0, 1, 12, 18, 29, 42, 60] {
        let at_30 = progress(
            SECONDS_SEQUENCE,
            FrameRate::new(30, 1).unwrap(),
            frame,
            BTreeMap::new(),
        );
        let at_60 = progress(
            SECONDS_SEQUENCE,
            FrameRate::new(60, 1).unwrap(),
            frame * 2,
            BTreeMap::new(),
        );
        assert_eq!(at_30.to_bits(), at_60.to_bits(), "shared frame {frame}");
    }
    for frame in [0, 1, 17, 83, 301] {
        let slow = progress(
            SECONDS_SEQUENCE,
            FrameRate::new(30_000, 1_001).unwrap(),
            frame,
            BTreeMap::new(),
        );
        let fast = progress(
            SECONDS_SEQUENCE,
            FrameRate::new(60_000, 1_001).unwrap(),
            frame * 2,
            BTreeMap::new(),
        );
        assert_eq!(
            slow.to_bits(),
            fast.to_bits(),
            "rational shared frame {frame}"
        );
    }
}

#[test]
fn frames_stages_and_dynamic_stagger_use_the_authored_unit() {
    let source = r##"
export const controls = defineControls({ props: { index: number({ default: 2 }) } });
const intro = defineSequence({ cards: stage({ at: frames(4), duration: frames(10) }) });
export default function P(ctx, props) {
  const t = staggerProgress(ctx.localFrame, ctx.fps, intro.cards, props.index, frames(3));
  return <View key="v" style={{ opacity: t }} />;
}
"##;
    assert_eq!(
        progress(source, FrameRate::new(30, 1).unwrap(), 15, BTreeMap::new()),
        0.5
    );
    assert_eq!(
        progress(source, FrameRate::new(60, 1).unwrap(), 15, BTreeMap::new()),
        0.5,
        "frames() schedules are deliberately frame-locked"
    );
}

#[test]
fn malformed_stage_graphs_fail_with_authored_labels() {
    let cases = [
        (
            r#"const s=defineSequence({a:stage({duration:seconds(1)}),a:stage({at:seconds(2),duration:seconds(1)})}); export default function P(ctx){return <View/>;}"#,
            "duplicate stage label `a`",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:seconds(1)}),b:stage({after:"later",duration:seconds(1)})}); export default function P(ctx){return <View/>;}"#,
            "later",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:seconds(1)}),b:stage({after:"a",delay:seconds(.1),overlap:seconds(.1),duration:seconds(1)})}); export default function P(ctx){return <View/>;}"#,
            "both delay and overlap",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:seconds(1)}),b:stage({after:"a",duration:frames(3)})}); export default function P(ctx){return <View/>;}"#,
            "cannot mix",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:seconds(0)})}); export default function P(ctx){return <View/>;}"#,
            "positive duration",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:seconds(1)}),b:stage({duration:seconds(1)})}); export default function P(ctx){return <View/>;}"#,
            "explicit at or after",
        ),
    ];
    for (source, needle) in cases {
        let diagnostics = messages(source);
        assert!(
            diagnostics.iter().any(|message| message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn sequence_stage_count_has_a_hard_prepare_budget() {
    let stages = (0..257)
        .map(|index| {
            if index == 0 {
                format!("s{index}:stage({{duration:frames(1)}})")
            } else {
                format!(
                    "s{index}:stage({{after:\"s{}\",duration:frames(1)}})",
                    index - 1
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        "const s=defineSequence({{{stages}}}); export default function P(ctx){{return <View/>;}}"
    );
    let diagnostics = messages(&source);
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("256-stage budget")),
        "got {diagnostics:#?}"
    );
}

#[test]
fn sampler_requires_real_sequence_values_and_the_context_fps() {
    for (source, needle) in [
        (
            r#"export default function P(ctx){return <View style={{opacity:stageProgress(ctx.localFrame, 60, {start:0,duration:1})}}/>;}"#,
            "exactly `ctx.fps`",
        ),
        (
            r#"export default function P(ctx){return <View style={{opacity:stageProgress(ctx.localFrame, ctx.fps, {start:0,duration:1})}}/>;}"#,
            "named stage",
        ),
    ] {
        let diagnostics = messages(source);
        assert!(
            diagnostics.iter().any(|message| message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn repeat_and_yoyo_reuse_the_public_modulo_math() {
    let source = |sampler: &str| {
        format!(
            r#"
const s=defineSequence({{pulse:stage({{duration:frames(8)}})}});
export default function P(ctx){{return <View key="v" style={{{{opacity:{sampler}(ctx.localFrame,ctx.fps,s.pulse,2)}}}}/>;}}
"#
        )
    };
    let repeat = source("repeatProgress");
    let yoyo = source("yoyoProgress");
    for (frame, expected) in [(0, 0.0), (2, 0.5), (4, 0.0), (6, 0.5), (8, 1.0)] {
        assert_eq!(
            progress(
                &repeat,
                FrameRate::new(30, 1).unwrap(),
                frame,
                BTreeMap::new()
            ),
            expected,
            "repeat frame {frame}"
        );
    }
    for (frame, expected) in [(0, 0.0), (2, 0.5), (4, 1.0), (6, 0.5), (8, 0.0)] {
        assert_eq!(
            progress(
                &yoyo,
                FrameRate::new(30, 1).unwrap(),
                frame,
                BTreeMap::new()
            ),
            expected,
            "yoyo frame {frame}"
        );
    }
    for artifact in [
        compile_motion(&repeat).unwrap().artifact,
        compile_motion(&yoyo).unwrap().artifact,
    ] {
        assert!(artifact.exprs.iter().any(|expr| matches!(
            expr,
            valle_motion::Expr::MathBinary {
                op: valle_motion::MathBinaryOp::Mod | valle_motion::MathBinaryOp::PingPong,
                ..
            }
        )));
    }
}

#[test]
fn repeat_cycle_count_is_static_and_bounded() {
    for (cycles, needle) in [("0", "1..=1024"), ("1025", "1..=1024"), ("1.5", "integer")] {
        let source = format!(
            r#"const s=defineSequence({{a:stage({{duration:frames(8)}})}}); export default function P(ctx){{return <View style={{{{opacity:repeatProgress(ctx.localFrame,ctx.fps,s.a,{cycles})}}}}/>;}}"#
        );
        let diagnostics = messages(&source);
        assert!(
            diagnostics.iter().any(|message| message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn freeze_frame_holds_one_local_clock_without_new_ir() {
    let sugar = r#"
export default function P(ctx) {
  return <View key="v" style={{ width: freezeFrame(ctx.hold.frame, { from: 3, to: 7 }) }} />;
}
"#;
    let manual = r#"
export default function P(ctx) {
  const frame = ctx.hold.frame;
  return <View key="v" style={{ width: frame < 3 ? frame : frame < 7 ? 3 : frame - 4 }} />;
}
"#;
    for (frame, expected) in [
        (0, 0.0),
        (2, 2.0),
        (3, 3.0),
        (6, 3.0),
        (7, 3.0),
        (8, 4.0),
        (12, 8.0),
    ] {
        assert_eq!(
            style_number(sugar, frame, "width"),
            expected,
            "frame {frame}"
        );
        assert_eq!(
            style_number(sugar, frame, "width"),
            style_number(manual, frame, "width"),
            "manual arithmetic at frame {frame}"
        );
    }
    let artifact = compile_motion(sugar).unwrap().artifact;
    assert!(artifact.exprs.iter().all(|expr| matches!(
        expr,
        valle_motion::Expr::Context { .. }
            | valle_motion::Expr::Const { .. }
            | valle_motion::Expr::Compare { .. }
            | valle_motion::Expr::Sub { .. }
            | valle_motion::Expr::Select { .. }
    )));

    let static_source = r#"
const SAMPLE = freezeFrame(9, { from: 3, to: 7 });
export default function P() { return <View key="v" style={{ width: SAMPLE }} />; }
"#;
    assert_eq!(style_number(static_source, 0, "width"), 5.0);
}

#[test]
fn freeze_frame_interval_is_static_integer_and_fail_closed() {
    for (source, needle) in [
        (
            r#"export default function P(ctx){return <View style={{width:freezeFrame(ctx.hold.frame,{from:3})}}/>;}"#,
            "requires both",
        ),
        (
            r#"export default function P(ctx){return <View style={{width:freezeFrame(ctx.hold.frame,{from:3,to:7,easing:"linear"})}}/>;}"#,
            "unknown freezeFrame option",
        ),
        (
            r#"export default function P(ctx){return <View style={{width:freezeFrame(ctx.hold.frame,{from:3.5,to:7})}}/>;}"#,
            "integer frames",
        ),
        (
            r#"export default function P(ctx){return <View style={{width:freezeFrame(ctx.hold.frame,{from:7,to:3})}}/>;}"#,
            "0 <= from < to",
        ),
        (
            r#"export const controls=defineControls({props:{from:number({default:3})}});export default function P(ctx,props){return <View style={{width:freezeFrame(ctx.hold.frame,{from:props.from,to:7})}}/>;}"#,
            "known at compile time",
        ),
    ] {
        let diagnostics = messages(source);
        assert!(
            diagnostics.iter().any(|message| message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn first_class_sequence_surface_lowers_without_a_sequence_wire_type() {
    let compiled = compile_motion(
        r##"
const intro = defineSequence({
  title: stage({ at: seconds(0), duration: seconds(0.6) }),
});
export default function Card(ctx) {
  const t = stageProgress(ctx.enter.frame, ctx.fps, intro.title);
  return <View style={{ opacity: t }} />;
}
"##,
    )
    .expect("Sequence author surface compiles");
    assert!(
        !serde_json::to_string(&compiled.artifact)
            .expect("artifact json")
            .contains("sequence"),
        "Sequence must compile away into the existing expression IR"
    );
}
