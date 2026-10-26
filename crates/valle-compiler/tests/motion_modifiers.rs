#![cfg(feature = "motion")]
//! Author modifiers compile into the random-access expression and scene model.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{EvalInputs, Expr, MotionValue, NodeKind, StyleValue, motion_context_at_frame};
use valle_timeline::FrameRate;

fn messages(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid modifier must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn style_number(source: &str, property: &str, frame: u32, fps: FrameRate) -> f64 {
    let artifact = compile_motion(source).expect("modifier compiles").artifact;
    let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let ctx = motion_context_at_frame(frame, 600, fps).expect("context");
    let values = valle_motion::eval_all(
        &artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: Some((1280.0, 720.0)),
        },
    )
    .expect("evaluate");
    let binding = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == property)
        .expect("style binding");
    match &binding.value {
        StyleValue::Static {
            value: MotionValue::Number(value),
        } => *value,
        StyleValue::Expr { expr } => match values[expr.0 as usize] {
            MotionValue::Number(value) => value,
            ref other => panic!("expected number, got {other:?}"),
        },
        other => panic!("expected numeric style, got {other:?}"),
    }
}

#[test]
fn repeater_expands_to_stable_keys_indices_and_progress() {
    let compiled = compile_motion(
        r##"
const copies = defineRepeater({ count: 4, keyPrefix: "echo" });
export default function P() {
  return <View>{copies.map((copy) => (
    <View key={copy.key} style={{ opacity: copy.progress, left: copy.index * 10 }} />
  ))}</View>;
}
"##,
    )
    .expect("repeater compiles");
    let children = compiled
        .artifact
        .nodes
        .iter()
        .filter(|node| node.key.starts_with("echo-"))
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 4);
    for (index, node) in children.iter().enumerate() {
        assert_eq!(node.key, format!("echo-{index}"));
        let opacity = node
            .styles
            .iter()
            .find(|style| style.property == "opacity")
            .expect("opacity");
        assert_eq!(
            opacity.value,
            StyleValue::Static {
                value: MotionValue::Number(index as f64 / 3.0)
            }
        );
    }
    assert!(
        !serde_json::to_string(&compiled.artifact)
            .unwrap()
            .contains("repeater")
    );
}

#[test]
fn wiggle_reuses_noise_and_is_bit_identical_at_shared_times() {
    const SOURCE: &str = r#"
export default function P() { const time = useCurrentTime();
  return <View style={{ left: wiggle(time.seconds, {
    seed: 7, frequency: 1.75, amplitude: 24, phase: 0.25,
  }) }} />;
}
"#;
    let artifact = compile_motion(SOURCE).expect("wiggle compiles").artifact;
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Noise1D { seed: 7, .. }))
    );
    assert!(!serde_json::to_string(&artifact).unwrap().contains("wiggle"));
    for frame in [0, 1, 7, 29, 83] {
        let a = style_number(SOURCE, "left", frame, FrameRate::new(30, 1).unwrap());
        let b = style_number(SOURCE, "left", frame * 2, FrameRate::new(60, 1).unwrap());
        assert_eq!(a.to_bits(), b.to_bits(), "shared frame {frame}");
    }
}

#[test]
fn follow_is_exact_sugar_for_motion_path() {
    let source = |name: &str| {
        format!(
            r#"
const P = path("M 0 0 C 20 0 80 100 100 100");
export default function Card() {{ const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return <View key="dot" style={{{{ motionPath: {name}(P, time.progress, {{anchor:"center",rotate:"auto",angleOffset:12}}) }}}} />;
}}
"#
        )
    };
    let follow = compile_motion(&source("follow")).expect("follow").artifact;
    let original = compile_motion(&source("motionPath"))
        .expect("motionPath")
        .artifact;
    assert_eq!(follow, original);
}

#[test]
fn trail_clamps_or_wraps_without_frame_history() {
    const CLAMPED: &str = r#"
export default function P() { const time = useCurrentTime();
  return <View style={{ opacity: trail(time.progress, 2, { gap: 0.2, mode: "clamp" }) }} />;
}
"#;
    const WRAPPED: &str = r#"
export default function P() { const time = useCurrentTime();
  return <View style={{ opacity: trail(time.progress, 2, { gap: 0.2, mode: "wrap" }) }} />;
}
"#;
    let clamp_artifact = compile_motion(CLAMPED).expect("clamped trail").artifact;
    let wrap_artifact = compile_motion(WRAPPED).expect("wrapped trail").artifact;
    // A value of the clock wraps as `mod` of it does, with the clock tolerance.
    assert!(wrap_artifact.exprs.iter().any(|expr| matches!(
        expr,
        Expr::MathBinary {
            op: valle_motion::MathBinaryOp::ClockMod,
            ..
        }
    )));
    assert!(
        clamp_artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Select { .. }))
    );
    assert!(
        !serde_json::to_string(&wrap_artifact)
            .unwrap()
            .contains("trail")
    );
    let negative = CLAMPED.replace("gap: 0.2", "gap: -0.2");
    let negative_wrap = WRAPPED.replace("gap: 0.2", "gap: -0.2");
    let fps = FrameRate::new(30, 1).unwrap();
    assert!((style_number(CLAMPED, "opacity", 300, fps) - 0.9).abs() < 1e-12);
    assert!((style_number(&negative, "opacity", 300, fps) - 0.1).abs() < 1e-12);
    assert!((style_number(&negative_wrap, "opacity", 0, fps) - 0.6).abs() < 1e-12);
    // Frame 9 is 0.3 s, where `seconds / 0.1` is 2.9999999999999996: the wrap starts over, as
    // `mod` does, rather than showing the end of the trail.
    let at_boundary = |opacity: &str| {
        let source = WRAPPED.replace(
            r#"trail(time.progress, 2, { gap: 0.2, mode: "wrap" })"#,
            opacity,
        );
        assert_ne!(source, WRAPPED);
        style_number(&source, "opacity", 9, fps)
    };
    assert_eq!(at_boundary("mod(time.seconds / 0.1, 1)"), 0.0);
    assert_eq!(
        at_boundary(r#"trail(time.seconds / 0.1, 0, { gap: 0, mode: "wrap" })"#),
        0.0
    );
}

#[test]
fn auto_rotate_reuses_the_existing_path_angle_expression() {
    let artifact = compile_motion(
        r#"
const P = path("M 0 0 L 100 100");
export default function Card() { const time = useCurrentTime();
  return <View style={{ rotate: autoRotate(P, time.progress) }} />;
}
"#,
    )
    .expect("auto rotate compiles")
    .artifact;
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathAngleAt { .. }))
    );
    assert!(
        !serde_json::to_string(&artifact)
            .unwrap()
            .contains("autoRotate")
    );
}

#[test]
fn modifier_options_and_budgets_fail_closed() {
    let cases = [
        (
            r#"const x=defineRepeater({count:0}); export default function P(){return <View/>;}"#,
            "1..=2048",
        ),
        (
            r#"const x=defineRepeater({count:4,wat:true}); export default function P(){return <View/>;}"#,
            "unknown option",
        ),
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{left:wiggle(time.seconds,60,{seed:1,frequency:2,amplitude:3})}}/>;}"#,
            "the frame rate argument is gone",
        ),
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{left:wiggle(time.seconds,{seed:-1,frequency:2,amplitude:3})}}/>;}"#,
            "seed",
        ),
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{opacity:trail(time.progress,1,{gap:'bad'})}}/>;}"#,
            "gap",
        ),
        (
            r#"export default function P(){ const time = useCurrentTime();return <View style={{opacity:trail(time.progress,1,{gap:.1,mode:"bounce"})}}/>;}"#,
            "clamp",
        ),
        (
            r#"const P=path("M0 0L1 1"); export default function C(){ const time = useCurrentTime();return <View style={{rotate:autoRotate(P,time.progress,{angleOffset:1})}}/>;}"#,
            "exactly two arguments",
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
fn follow_and_auto_rotate_already_share_the_path_sampling_truth() {
    let compiled = compile_motion(
        r##"
const ROUTE = path("M 0 0 C 40 0 60 80 100 80");
export default function P() { const time = useCurrentTime();
  return <View key="dot" style={{
    width: 10, height: 10,
    motionPath: motionPath(ROUTE, time.progress),
  }} />;
}
"##,
    )
    .expect("existing motionPath compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathPointAt { .. }))
    );
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathAngleAt { .. }))
    );
    let styles = &compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "dot")
        .expect("dot")
        .styles;
    assert!(styles.iter().any(|style| style.property == "translate"));
    assert!(styles.iter().any(|style| style.property == "rotate"));
}

#[test]
fn dynamic_noise_is_the_only_wire_needed_by_the_named_wiggle_surface() {
    let compiled = compile_motion(
        r##"
export default function P() { const time = useCurrentTime();
  return <View style={{ opacity: noise1d(7, time.seconds) }} />;
}
"##,
    )
    .expect("dynamic noise compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Noise1D { seed: 7, .. }))
    );

    let wiggle = compile_motion(
        r#"export default function P(){ const time = useCurrentTime();return <View style={{left:wiggle(time.seconds,{seed:7,frequency:2,amplitude:10})}}/>;}"#,
    )
    .expect("named wiggle compiles");
    assert!(
        wiggle
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Noise1D { seed: 7, .. }))
    );
    assert!(
        !serde_json::to_string(&wiggle.artifact)
            .unwrap()
            .contains("wiggle")
    );
}

#[test]
fn prepared_scalar_maps_and_named_repeaters_share_layout_templates() {
    let compiled = compile_motion(
        r##"
const ITEMS = [0, 1, 2, 3];
export default function P() {
  return <View>{ITEMS.map((index) => <View key={`copy-${index}`} style={{opacity:index / 3}} />)}</View>;
}
"##,
    )
    .expect("prepared scalar list compiles");
    let boxes = compiled
        .artifact
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Box))
        .count();
    assert_eq!(boxes, 1, "only the root is outside the shared template");
    assert_eq!(compiled.artifact.instance_groups.len(), 1);
    assert_eq!(compiled.artifact.instance_groups[0].rows(), 4);
    assert_eq!(
        compiled.artifact.instance_groups[0]
            .keys
            .key_at(2)
            .as_deref(),
        Some("copy-2")
    );

    let repeated = compile_motion(
        r#"const I=defineRepeater({count:4}); export default function P(){return <View>{I.map((x)=><View key={x.key}/>)}</View>;}"#,
    )
    .expect("named repeater compiles");
    assert_eq!(repeated.artifact.instance_groups.len(), 1);
    assert_eq!(repeated.artifact.instance_groups[0].rows(), 4);
    assert_eq!(
        repeated.artifact.instance_groups[0].template_node_count(),
        1
    );
    assert_eq!(repeated.artifact.nodes.len(), compiled.artifact.nodes.len());
}
