#![cfg(feature = "motion")]
//! Deterministic frame-time math, dynamic text and author expression helpers.
use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, NodeKind, ResolvedSignals, StyleValue, TextValue,
    motion_context_at, phase_windows, resolve_props,
};
use valle_timeline::FrameRate;

fn evaluate(source: &str) -> (valle_motion::SceneArtifact, Vec<MotionValue>) {
    let artifact = compile_motion(source)
        .expect("frame expression source compiles")
        .artifact;
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let windows = phase_windows(&artifact.controls.phase_spec(), 60);
    let ctx = motion_context_at(10, &windows, FrameRate::new(30, 1).unwrap()).expect("context");
    let values = valle_motion::eval_all(
        &artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &ResolvedSignals::default(),
            unit: None,
            viewport: Some((1920.0, 1080.0)),
        },
    )
    .expect("evaluate");
    (artifact, values)
}

fn style_value<'a>(
    artifact: &'a valle_motion::SceneArtifact,
    values: &'a [MotionValue],
    key: &str,
    property: &str,
) -> &'a MotionValue {
    let binding = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .and_then(|node| node.styles.iter().find(|style| style.property == property))
        .unwrap_or_else(|| panic!("{key}.{property} exists"));
    match &binding.value {
        StyleValue::Static { value } => value,
        StyleValue::Expr { expr } => &values[expr.0 as usize],
    }
}

#[test]
fn math_aliases_and_javascript_remainder_share_deterministic_frame_semantics() {
    let source = r##"
export const controls = defineControls({ props: {
  angle: number({ default: 0.25 }), value: number({ default: -5.5 }), divisor: number({ default: 2 }),
  huge: number({ default: 1e308 }), tiny: number({ default: 1e-308 }),
}});
export default function P(ctx, props) { return <Scene>
  <View key="sin" style={{ opacity: Math.sin(props.angle) }} />
  <View key="sqrt" style={{ opacity: Math.sqrt(props.angle) }} />
  <View key="exp" style={{ opacity: Math.exp(-props.angle) }} />
  <View key="cos" style={{ opacity: Math.cos(props.angle) }} />
  <View key="tan" style={{ opacity: Math.tan(props.angle) }} />
  <View key="atan2" style={{ opacity: Math.atan2(props.angle, 2) }} />
  <View key="floor" style={{ opacity: Math.floor(props.value) }} />
  <View key="ceil" style={{ opacity: Math.ceil(props.value) }} />
  <View key="round" style={{ opacity: Math.round(props.value) }} />
  <View key="trunc" style={{ opacity: Math.trunc(props.value) }} />
  <View key="remainder" style={{ left: props.value % props.divisor }} />
  <View key="extreme-remainder" style={{ left: props.huge % props.tiny }} />
</Scene>; }
"##;
    let (artifact, values) = evaluate(source);
    let number = |key: &str, property: &str| match style_value(&artifact, &values, key, property) {
        MotionValue::Number(value) => *value,
        other => panic!("expected number, got {other:?}"),
    };
    assert_eq!(
        number("sqrt", "opacity").to_bits(),
        valle_draw::math::sqrt(0.25).to_bits()
    );
    assert_eq!(
        number("exp", "opacity").to_bits(),
        valle_draw::math::exp(-0.25).to_bits()
    );
    assert_eq!(
        number("sin", "opacity").to_bits(),
        valle_draw::math::sin(0.25).to_bits()
    );
    assert_eq!(
        number("cos", "opacity").to_bits(),
        valle_draw::math::cos(0.25).to_bits()
    );
    assert_eq!(
        number("tan", "opacity").to_bits(),
        valle_draw::math::tan(0.25).to_bits()
    );
    assert_eq!(
        number("atan2", "opacity").to_bits(),
        valle_draw::math::atan2(0.25, 2.0).to_bits()
    );
    assert_eq!(number("floor", "opacity"), -6.0);
    assert_eq!(number("ceil", "opacity"), -5.0);
    assert_eq!(number("round", "opacity"), -5.0);
    assert_eq!(number("trunc", "opacity"), -5.0);
    assert_eq!(number("remainder", "left"), -1.5);
    assert_eq!(
        number("extreme-remainder", "left").to_bits(),
        (1e308_f64 % 1e-308_f64).to_bits()
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::MathUnary { .. }))
    );
    assert!(artifact.exprs.iter().any(|expr| matches!(
        expr,
        Expr::MathBinary {
            op: valle_motion::MathBinaryOp::Remainder,
            ..
        }
    )));
    assert!(artifact.exprs.iter().all(|expr| !matches!(
        expr,
        Expr::MathBinary {
            op: valle_motion::MathBinaryOp::Mod,
            ..
        }
    )));
}

#[test]
fn dynamic_number_text_angle_helpers_transform_axes_and_wiggle2d_are_author_sugar() {
    let source = r##"
export const controls = defineControls({ props: {
  angle: number({ default: 90 }), value: number({ default: -5.5 }), x: number({ default: 24 }), y: number({ default: 12 }),
}});
export default function P(ctx, props) { return <Scene>
  <Text key="value">{props.value}</Text>
  <Path key="arc" d={arc(point(100, 100), 40, 0, deg(props.angle))} fill="none" stroke="#fff" />
  <View key="transform" style={{ transform: `translateX(${props.x}px) translateY(${props.y}px) rotate(${props.angle}deg) scale(1)` }} />
  <View key="wiggle" style={{ translate: wiggle2D(ctx.localFrame, ctx.fps, { seed: 7, frequency: 2, amplitude: [8, 4] }) }} />
  <View key="tau" style={{ opacity: (ctx.hold.progress * TAU) / TAU }} />
</Scene>; }
"##;
    let (artifact, values) = evaluate(source);
    let text = artifact
        .nodes
        .iter()
        .find(|node| node.key == "value")
        .and_then(|node| match &node.kind {
            NodeKind::Text { text, .. } => Some(text),
            _ => None,
        })
        .expect("text");
    let TextValue::Expr { expr } = text else {
        panic!("dynamic text")
    };
    assert_eq!(values[expr.0 as usize], MotionValue::Str("-5.5".into()));

    let MotionValue::Length2(translation) =
        style_value(&artifact, &values, "transform", "translate")
    else {
        panic!("axis transform lowers to Length2")
    };
    assert_eq!(translation.x.value, 24.0);
    assert_eq!(translation.y.value, 12.0);
    assert!(matches!(
        style_value(&artifact, &values, "wiggle", "translate"),
        MotionValue::Length2(_)
    ));
    assert!(matches!(
        artifact
            .nodes
            .iter()
            .find(|node| node.key == "arc")
            .map(|node| &node.kind),
        Some(NodeKind::Path { .. })
    ));
}

#[test]
fn unsupported_math_zero_remainder_and_bad_arc_keep_pointed_diagnostics() {
    let cases = [
        (
            r#"export default function P(ctx){return <View style={{opacity:Math.log(ctx.hold.progress)}}/>;}"#,
            "Math.log",
        ),
        (
            r#"export default function P(){return <View style={{opacity:sqrt(0.25)}}/>;}"#,
            "helper `sqrt`",
        ),
        (
            r#"export default function P(ctx){return <View style={{left:ctx.localFrame % 0}}/>;}"#,
            "divisor",
        ),
        (
            r#"export default function P(){return <Path d={arc(point(0,0),0,0,TAU)} />;}"#,
            "radius",
        ),
        (
            r#"export default function P(){return <Path d={arc(point(0,0),10,0,TAU + 1)} />;}"#,
            "sweep",
        ),
        (
            r#"export default function P(ctx){return <View style={{translate:wiggle2D(ctx.localFrame,ctx.fps,{seed:1,frequency:2,amplitude:5})}}/>;}"#,
            "[x, y]",
        ),
    ];
    for (source, needle) in cases {
        let diagnostics =
            compile_motion(source).expect_err("invalid frame expression fails closed");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn existing_noise_name_accepts_dynamic_coordinates() {
    let compiled = compile_motion(
        r##"
export default function Card(ctx) {
  const n = noise1d(7, ctx.hold.progress * 10);
  return <View style={{ opacity: n }} />;
}
"##,
    )
    .expect("dynamic noise compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, valle_motion::Expr::Noise1D { seed: 7, .. })),
        "dynamic coordinates use the existing noise1d expression"
    );
}

#[test]
fn number_formatting_reaches_dynamic_text() {
    let compiled = compile_motion(
        r##"
export const controls = defineControls({ props: { value: number({ default: 1234 }) } });
export default function Card(ctx, props) {
  const amount = formatNumber(props.value * ctx.hold.progress, { decimals: 1, grouping: true });
  return <Text>{amount}</Text>;
}
"##,
    )
    .expect("dynamic formatting compiles");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, valle_motion::Expr::FormatNumber { .. }))
    );
}
