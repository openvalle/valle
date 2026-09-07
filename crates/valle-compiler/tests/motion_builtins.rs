#![cfg(feature = "motion")]
//! Deterministic math, noise and bounded number formatting.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, NodeKind, ResolvedSignals, StyleValue, TextValue,
    motion_context_at, phase_windows, resolve_props,
};
use valle_timeline::FrameRate;

fn evaluate(
    source: &str,
    overrides: BTreeMap<String, MotionValue>,
) -> (valle_motion::SceneArtifact, Vec<MotionValue>) {
    let artifact = compile_motion(source).expect("compile").artifact;
    let props = resolve_props(&artifact.controls, &overrides).expect("props");
    let windows = phase_windows(&artifact.controls.phase_spec(), 60);
    let ctx = motion_context_at(10, &windows, FrameRate::new(30, 1).unwrap()).expect("ctx");
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
    .expect("eval");
    (artifact, values)
}

fn style_number(artifact: &valle_motion::SceneArtifact, values: &[MotionValue], key: &str) -> f64 {
    let style = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .and_then(|node| node.styles.iter().find(|style| style.property == "opacity"))
        .expect("opacity style");
    let value = match &style.value {
        StyleValue::Static { value } => value,
        StyleValue::Expr { expr } => &values[expr.0 as usize],
    };
    let MotionValue::Number(value) = value else {
        panic!("number style");
    };
    *value
}

fn text_value(artifact: &valle_motion::SceneArtifact, values: &[MotionValue], key: &str) -> String {
    let text = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .and_then(|node| match &node.kind {
            NodeKind::Text { text, .. } => Some(text),
            _ => None,
        })
        .expect("text node");
    match text {
        TextValue::Static { value } => value.clone(),
        TextValue::Expr { expr } => match &values[expr.0 as usize] {
            MotionValue::Str(value) => value.clone(),
            other => panic!("text is not a string: {other:?}"),
        },
    }
}

#[test]
fn every_math_primitive_and_dynamic_noise_reaches_the_shared_evaluator() {
    let source = r##"
export const controls = defineControls({ props: {
  x: number({ default: -1.25 }), y: number({ default: 0.75 }), period: number({ default: 5 })
}});
export default function P(ctx, props) {
  return <Scene>
    <View key="sin" style={{ opacity: sin(props.y) }} />
    <View key="cos" style={{ opacity: cos(props.y) }} />
    <View key="atan2" style={{ opacity: atan2(props.y, props.x) }} />
    <View key="floor" style={{ opacity: floor(props.x) }} />
    <View key="ceil" style={{ opacity: ceil(props.x) }} />
    <View key="round" style={{ opacity: round(props.x) }} />
    <View key="mod" style={{ opacity: mod(props.x, props.period) }} />
    <View key="fract" style={{ opacity: fract(props.x) }} />
    <View key="ping" style={{ opacity: pingPong(props.x, props.period) }} />
    <View key="noise" style={{ opacity: noise2d(7, props.x, props.y) }} />
  </Scene>;
}
"##;
    let (artifact, values) = evaluate(source, BTreeMap::new());
    assert_eq!(
        style_number(&artifact, &values, "sin").to_bits(),
        valle_draw::math::sin(0.75).to_bits()
    );
    assert_eq!(
        style_number(&artifact, &values, "cos").to_bits(),
        valle_draw::math::cos(0.75).to_bits()
    );
    assert_eq!(
        style_number(&artifact, &values, "atan2").to_bits(),
        valle_draw::math::atan2(0.75, -1.25).to_bits()
    );
    assert_eq!(style_number(&artifact, &values, "floor"), -2.0);
    assert_eq!(style_number(&artifact, &values, "ceil"), -1.0);
    assert_eq!(style_number(&artifact, &values, "round"), -1.0);
    assert_eq!(style_number(&artifact, &values, "mod"), 3.75);
    assert_eq!(style_number(&artifact, &values, "fract"), 0.75);
    assert_eq!(style_number(&artifact, &values, "ping"), 1.25);
    assert_eq!(
        style_number(&artifact, &values, "noise").to_bits(),
        valle_motion::compute::noise::value_noise_2d(7, -1.25, 0.75).to_bits()
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::MathUnary { .. }))
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::MathBinary { .. }))
    );
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Noise2D { seed: 7, .. }))
    );
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == valle_motion::MOTION_MATH_CAPABILITY)
    );
}

#[test]
fn ecmascript_round_keeps_negative_zero_on_static_and_dynamic_paths() {
    let static_source =
        r#"export default function P(){return <View key="v" style={{opacity:round(-0.5)}}/>;}"#;
    let dynamic_source = r#"
export const controls=defineControls({props:{x:number({default:-0.5})}});
export default function P(ctx,props){return <View key="v" style={{opacity:round(props.x)}}/>;}
"#;
    let (static_artifact, static_values) = evaluate(static_source, BTreeMap::new());
    let (dynamic_artifact, dynamic_values) = evaluate(dynamic_source, BTreeMap::new());
    let a = style_number(&static_artifact, &static_values, "v");
    let b = style_number(&dynamic_artifact, &dynamic_values, "v");
    assert_eq!(a.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(a.to_bits(), b.to_bits());
    assert!(
        !static_artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == valle_motion::MOTION_MATH_CAPABILITY),
        "fully static math folds away"
    );
}

#[test]
fn formatting_is_locale_free_bounded_and_composes_with_text() {
    let source = r##"
export const controls=defineControls({props:{n:number({default:1234.5}),p:number({default:.125}),i:number({default:-7})}});
export default function P(ctx,props){return <Scene>
  <Text key="number">{formatNumber(props.n,{decimals:2,grouping:true})}</Text>
  <Text key="percent">{formatPercent(props.p,{decimals:1})}</Text>
  <Text key="pad">{padNumber(props.i,{width:3})}</Text>
</Scene>;}
"##;
    let (artifact, values) = evaluate(source, BTreeMap::new());
    assert_eq!(text_value(&artifact, &values, "number"), "1,234.50");
    assert_eq!(text_value(&artifact, &values, "percent"), "12.5%");
    assert_eq!(text_value(&artifact, &values, "pad"), "-007");
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == valle_motion::NUMBER_FORMAT_CAPABILITY)
    );
}

#[test]
fn bad_domains_options_and_dynamic_seeds_fail_closed() {
    let cases = [
        (
            r#"export default function P(ctx){return <View style={{opacity:mod(ctx.localFrame,0)}}/>;}"#,
            "period",
        ),
        (
            r#"export default function P(ctx){return <Text>{formatNumber(ctx.localFrame,{decimals:13})}</Text>;}"#,
            "0..=12",
        ),
        (
            r#"export default function P(){return <Text>{formatNumber(1,{decimals:200})}</Text>;}"#,
            "0..=12",
        ),
        (
            r#"export default function P(ctx){return <Text>{padNumber(ctx.localFrame,{width:0})}</Text>;}"#,
            "1..=64",
        ),
        (
            r#"export default function P(ctx){return <View style={{opacity:noise1d(ctx.localFrame,.5)}}/>;}"#,
            "seed",
        ),
    ];
    for (source, needle) in cases {
        let diagnostics = compile_motion(source).expect_err("bad builtin must fail");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected {needle:?}, got {diagnostics:#?}"
        );
    }
}
