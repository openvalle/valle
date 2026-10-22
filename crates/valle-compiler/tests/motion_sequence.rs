#![cfg(feature = "motion")]
//! Sequence provides compile-time scheduling within a Timeline.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, MotionValue, motion_context_at_frame};
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
    let ctx = motion_context_at_frame(frame, 10_000, fps).expect("frame");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
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
    let ctx =
        motion_context_at_frame(frame, 10_000, FrameRate::new(30, 1).unwrap()).expect("frame");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
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
  title: stage({ duration: 0.6 }),
  cards: stage({ after: "title", overlap: 0.2, duration: 1.0 }),
});
export default function P() { const time = useCurrentTime();
  return <View key="v" style={{ opacity: stageProgress(time.seconds, intro.cards) }} />;
}
"##;

#[test]
fn named_stages_compile_to_the_same_artifact_as_manual_arithmetic() {
    let sequence = compile_motion(SECONDS_SEQUENCE).expect("Sequence").artifact;
    let manual = compile_motion(
        r##"
export default function P() { const time = useCurrentTime();
  return <View key="v" style={{ opacity: clamp(((time.seconds) - (0.6 - 0.2)) / 1.0, 0, 1) }} />;
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
fn stages_and_dynamic_stagger_are_in_seconds_at_any_frame_rate() {
    let source = r##"
export const controls = ({ props: { index: number({ default: 2 }) } });
const intro = defineSequence({ cards: stage({ at: 0.25, duration: 0.5 }) });
export default function P(props) { const { seconds } = useCurrentTime();
  const t = staggerProgress(seconds, intro.cards, props.index, 0.125);
  return <View key="v" style={{ opacity: t }} />;
}
"##;
    // The third card starts at 0.25 + 2 × 0.125 seconds; 0.75 seconds is halfway through it.
    assert_eq!(
        progress(source, FrameRate::new(8, 1).unwrap(), 6, BTreeMap::new()),
        0.5
    );
    assert_eq!(
        progress(source, FrameRate::new(16, 1).unwrap(), 12, BTreeMap::new()),
        0.5
    );
}

#[test]
fn malformed_stage_graphs_fail_with_authored_labels() {
    let cases = [
        (
            r#"const s=defineSequence({a:stage({duration:1}),a:stage({at:2,duration:1})}); export default function P(){ return <View/>;}"#,
            "duplicate stage label `a`",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:1}),b:stage({after:"later",duration:1})}); export default function P(){ return <View/>;}"#,
            "later",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:1}),b:stage({after:"a",delay:.1,overlap:.1,duration:1})}); export default function P(){ return <View/>;}"#,
            "both delay and overlap",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:0})}); export default function P(){ return <View/>;}"#,
            "a finite positive number of seconds",
        ),
        (
            r#"const s=defineSequence({a:stage({duration:1}),b:stage({duration:1})}); export default function P(){ return <View/>;}"#,
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
                format!("s{index}:stage({{duration:1 / 30}})")
            } else {
                format!(
                    "s{index}:stage({{after:\"s{}\",duration:1 / 30}})",
                    index - 1
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        "const s=defineSequence({{{stages}}}); export default function P(){{ return <View/>;}}"
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
fn sampler_requires_real_sequence_values_and_the_time_in_seconds() {
    for (source, needle) in [
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{opacity:stageProgress(time.seconds, 60, {start:0,duration:1})}}/>;}"#,
            "the frame rate argument is gone",
        ),
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{opacity:stageProgress(time.seconds, {start:0,duration:1})}}/>;}"#,
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
const s=defineSequence({{pulse:stage({{duration:1}})}});
export default function P(){{ const {{ seconds }} = useCurrentTime();return <View key="v" style={{{{opacity:{sampler}(seconds,s.pulse,2)}}}}/>;}}
"#
        )
    };
    let repeat = source("repeatProgress");
    let yoyo = source("yoyoProgress");
    // At 8 fps, frames 0 to 8 cover the one-second stage.
    for (frame, expected) in [(0, 0.0), (2, 0.5), (4, 0.0), (6, 0.5), (8, 1.0)] {
        assert_eq!(
            progress(
                &repeat,
                FrameRate::new(8, 1).unwrap(),
                frame,
                BTreeMap::new()
            ),
            expected,
            "repeat frame {frame}"
        );
    }
    for (frame, expected) in [(0, 0.0), (2, 0.5), (4, 1.0), (6, 0.5), (8, 0.0)] {
        assert_eq!(
            progress(&yoyo, FrameRate::new(8, 1).unwrap(), frame, BTreeMap::new()),
            expected,
            "yoyo frame {frame}"
        );
    }
    for artifact in [
        compile_motion(&repeat).unwrap().artifact,
        compile_motion(&yoyo).unwrap().artifact,
    ] {
        // A remainder of the clock takes the clock tolerance.
        assert!(artifact.exprs.iter().any(|expr| matches!(
            expr,
            valle_motion::Expr::MathBinary {
                op: valle_motion::MathBinaryOp::ClockMod | valle_motion::MathBinaryOp::PingPong,
                ..
            }
        )));
    }
}

#[test]
fn repeat_cycle_count_is_static_and_bounded() {
    for (cycles, needle) in [("0", "1..=1024"), ("1025", "1..=1024"), ("1.5", "integer")] {
        let source = format!(
            r#"const s=defineSequence({{a:stage({{duration:1}})}}); export default function P(){{ const time = useCurrentTime();return <View style={{{{opacity:repeatProgress(time.seconds,s.a,{cycles})}}}}/>;}}"#
        );
        let diagnostics = messages(&source);
        assert!(
            diagnostics.iter().any(|message| message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn freeze_time_holds_one_local_clock_without_new_ir() {
    let sugar = r#"
export default function P() { const { seconds } = useCurrentTime();
  return <View key="v" style={{ width: freezeTime(seconds, { from: 0.1, to: 0.25 }) }} />;
}
"#;
    let manual = r#"
export default function P() { const { seconds } = useCurrentTime();
  return <View key="v" style={{ width: seconds < 0.1 ? seconds : seconds < 0.25 ? 0.1 : seconds - 0.15 }} />;
}
"#;
    // At 30 fps the time holds at 0.1 seconds from frame 3 through frame 7.
    for (frame, expected) in [
        (0, 0.0),
        (2, 2.0 / 30.0),
        (3, 0.1),
        (6, 0.1),
        (7, 0.1),
        (8, 8.0 / 30.0 - 0.15),
        (12, 0.4 - 0.15),
    ] {
        let width = style_number(sugar, frame, "width");
        assert!((width - expected).abs() < 1e-12, "frame {frame}: {width}");
        assert_eq!(
            width,
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
const SAMPLE = freezeTime(0.3, { from: 0.1, to: 0.25 });
export default function P() { return <View key="v" style={{ width: SAMPLE }} />; }
"#;
    assert!((style_number(static_source, 0, "width") - 0.15).abs() < 1e-12);
}

#[test]
fn freeze_time_interval_is_static_and_fail_closed() {
    for (source, needle) in [
        (
            r#"export default function P(){ const { seconds } = useCurrentTime();return <View style={{width:freezeTime(seconds,{from:0.1})}}/>;}"#,
            "requires both",
        ),
        (
            r#"export default function P(){ const { seconds } = useCurrentTime();return <View style={{width:freezeTime(seconds,{from:0.1,to:0.25,easing:"linear"})}}/>;}"#,
            "unknown freezeTime option",
        ),
        (
            r#"export default function P(){ const { seconds } = useCurrentTime();return <View style={{width:freezeTime(seconds,{from:0.25,to:0.1})}}/>;}"#,
            "from < to",
        ),
        (
            r#"export const controls=({props:{from:number({default:0.1})}});export default function P(props){ const { seconds } = useCurrentTime();return <View style={{width:freezeTime(seconds,{from:props.from,to:0.25})}}/>;}"#,
            "known at compile time",
        ),
        (
            r#"export default function P(){ const { seconds } = useCurrentTime();return <View style={{width:freezeFrame(seconds,{from:3,to:7})}}/>;}"#,
            "`freezeFrame` is not available",
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
  title: stage({ at: 0, duration: 0.6 }),
});
export default function Card() { const time = useCurrentTime();
  const t = stageProgress(time.seconds, intro.title);
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
