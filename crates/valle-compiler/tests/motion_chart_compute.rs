#![cfg(feature = "motion")]
//! Chart compute returns prepared numbers; paths use the generic line, area and arc primitives.

use valle_compiler::motion::compile_motion;
use valle_motion::{MotionValue, PathValue, StyleValue};

fn static_number(artifact: &valle_motion::SceneArtifact, key: &str, property: &str) -> f64 {
    let binding = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .unwrap_or_else(|| panic!("missing {key}"))
        .styles
        .iter()
        .find(|binding| binding.property == property)
        .unwrap_or_else(|| panic!("missing {key}.{property}"));
    match &binding.value {
        StyleValue::Static {
            value: MotionValue::Number(value),
        } => *value,
        StyleValue::Static {
            value: MotionValue::Length(value),
        } => value.value,
        other => panic!("{key}.{property} should be prepared, got {other:?}"),
    }
}

#[test]
fn stack_reaches_the_author_as_prepared_diverging_geometry() {
    let source = r##"
const S = stack([[20, -10], [30, -15]], { offset: "zero" });
const Y = scaleLinear({ domain: [-25, 50], range: [600, 100] });
export default function Stacked() {
  return <Scene key="scene">
    <View key="a0" style={{position:"absolute",left:100,top:Y.map(S.layers[0][0][1]),width:80,height:Y.map(S.layers[0][0][0])-Y.map(S.layers[0][0][1])}} />
    <View key="b0" style={{position:"absolute",left:100,top:Y.map(S.layers[1][0][1]),width:80,height:Y.map(S.layers[1][0][0])-Y.map(S.layers[1][0][1])}} />
    <View key="a1" style={{position:"absolute",left:240,top:Y.map(S.layers[0][1][1]),width:80,height:Y.map(S.layers[0][1][0])-Y.map(S.layers[0][1][1])}} />
    <View key="b1" style={{position:"absolute",left:240,top:Y.map(S.layers[1][1][1]),width:80,height:Y.map(S.layers[1][1][0])-Y.map(S.layers[1][1][1])}} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).expect("stack compiles").artifact;
    assert_eq!(static_number(&artifact, "a0", "top"), Y(20.0));
    assert_eq!(static_number(&artifact, "b0", "top"), Y(50.0));
    assert_eq!(static_number(&artifact, "a1", "top"), Y(0.0));
    assert_eq!(static_number(&artifact, "b1", "top"), Y(-10.0));
}

#[allow(non_snake_case)]
fn Y(value: f64) -> f64 {
    600.0 + (value + 25.0) / 75.0 * (100.0 - 600.0)
}

#[test]
fn stack_expand_normalizes_and_existing_path_primitives_consume_the_result() {
    let source = r##"
const S = stack([[1, 2], [3, 2]], { offset: "expand" });
const P = line([point(100, 500 - S.layers[0][0][1] * 400), point(500, 500 - S.layers[0][1][1] * 400)]);
export default function Normalized(){return <Path key="line" d={P} fill="none" stroke="#fff"/>;}
"##;
    let artifact = compile_motion(source)
        .expect("normalized stack feeds generic paths")
        .artifact;
    let path = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            valle_motion::NodeKind::Path {
                d: PathValue::Static { value },
                ..
            } => Some(value),
            _ => None,
        })
        .expect("stack result reaches a prepared Path");
    assert_eq!(
        path.verbs,
        [valle_draw::PathVerb::Move, valle_draw::PathVerb::Line]
    );
    assert_eq!(
        path.points,
        [
            valle_draw::Point::new(100.0, 400.0),
            valle_draw::Point::new(500.0, 300.0),
        ],
        "expand values must determine the actual generic path coordinates"
    );
}

#[test]
fn malformed_or_runtime_stack_inputs_fail_closed() {
    for (source, needle) in [
        (
            r#"const S=stack([[1],[2,3]]); export default function P(){return <View/>;}"#,
            "rectangular",
        ),
        (
            r#"export default function P(ctx){const S=stack([[ctx.hold.progress]]);return <View style={{left:S.layers[0][0][1]}}/>;}"#,
            "prepare",
        ),
        (
            r#"const S=stack([[-1]],{offset:"expand"}); export default function P(){return <View/>;}"#,
            "non-negative",
        ),
    ] {
        let diagnostics = compile_motion(source).expect_err("bad stack input must fail");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "wanted {needle}, got {diagnostics:#?}"
        );
    }
}
