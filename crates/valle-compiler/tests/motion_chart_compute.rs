#![cfg(feature = "motion")]
//! Chart compute returns prepared numbers; paths use the generic line, area, sector and curve primitives.

use std::collections::BTreeMap;

use valle_compiler::motion::{MotionModuleGraph, compile_motion, compile_motion_modules};
use valle_draw::{PathVerb, Rgba};
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

fn static_path<'a>(
    artifact: &'a valle_motion::SceneArtifact,
    key: &str,
) -> &'a valle_motion::PathData {
    artifact
        .nodes
        .iter()
        .find_map(|node| match (&node.key, &node.kind) {
            (
                node_key,
                valle_motion::NodeKind::Path {
                    d: PathValue::Static { value },
                    ..
                },
            ) if node_key == key => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| panic!("static path `{key}`"))
}

fn static_color(artifact: &valle_motion::SceneArtifact, key: &str, property: &str) -> Rgba {
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
            value: MotionValue::Color(value),
        } => *value,
        other => panic!("{key}.{property} should be a prepared color, got {other:?}"),
    }
}

#[test]
fn pie_and_sector_compile_with_stable_topology_and_zero_identity() {
    let source = r##"
const SLICES = pie([1, 0, 3], { startAngle: 0, endAngle: TAU, padAngle: 0 });
const SOLID = sector({ center: point(200, 200), outer: 80, start: SLICES[0].startAngle, end: SLICES[0].endAngle });
const RING = sector({ center: point(200, 200), inner: 30, outer: 80, start: 0, end: TAU, cornerRadius: 8 });
const ZERO = sector({ center: point(200, 200), inner: 0, outer: 80, start: SLICES[1].startAngle, end: SLICES[1].endAngle });
export default function Pie() {
  return <Scene key="scene">
    <Path key="solid" d={SOLID} />
    <Path key="ring" d={RING} />
    <Path key="zero" d={ZERO} />
    <View key="frac0" style={{ left: SLICES[0].fraction * 100, top: SLICES[0].index }} />
    <View key="frac1" style={{ left: SLICES[1].fraction * 100, top: SLICES[1].index }} />
    <View key="frac2" style={{ left: SLICES[2].fraction * 100, top: SLICES[2].index }} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).expect("pie compiles").artifact;
    assert_eq!(static_number(&artifact, "frac0", "left"), 25.0);
    assert_eq!(static_number(&artifact, "frac1", "left"), 0.0);
    assert_eq!(static_number(&artifact, "frac1", "top"), 1.0);
    assert_eq!(static_number(&artifact, "frac2", "left"), 75.0);
    let solid = static_path(&artifact, "solid");
    let ring = static_path(&artifact, "ring");
    let zero = static_path(&artifact, "zero");
    assert_eq!(solid.verbs, ring.verbs);
    assert_eq!(solid.verbs, zero.verbs);
    assert_eq!(solid.points.len(), ring.points.len());
    assert!(solid.verbs.iter().any(|verb| *verb == PathVerb::Close));
    assert!(
        solid
            .points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite())
    );
}

#[test]
fn curve_area_and_area_band_compile_from_the_author_surface() {
    let source = r##"
const UPPER = curve([point(0, 10), point(40, 2), point(80, 8)], { type: "monotoneX" });
const LOWER = curve([point(0, 30), point(40, 28), point(80, 32)], { type: "monotoneX" });
const FILL = area(UPPER, 40);
const BAND = areaBand(UPPER, LOWER);
export default function Smooth() {
  return <Scene key="scene">
    <Path key="stroke" d={UPPER} fill="none" stroke="#fff" />
    <Path key="fill" d={FILL} />
    <Path key="band" d={BAND} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).expect("curve compiles").artifact;
    let stroke = static_path(&artifact, "stroke");
    assert!(stroke.verbs.iter().any(|verb| *verb == PathVerb::Cubic));
    assert!(!stroke.verbs.contains(&PathVerb::Close));
    let fill = static_path(&artifact, "fill");
    assert!(fill.verbs.contains(&PathVerb::Cubic));
    assert_eq!(fill.verbs.last(), Some(&PathVerb::Close));
    let band = static_path(&artifact, "band");
    assert_eq!(band.verbs.last(), Some(&PathVerb::Close));
    assert!(
        band.verbs
            .iter()
            .filter(|verb| **verb == PathVerb::Cubic)
            .count()
            >= 2
    );
}

#[test]
fn color_scales_compile_and_survive_module_rebind() {
    let source = r##"
const SEQ = scaleSequential({ domain: [0, 10], colors: ["#000000", "#ffffff"] });
const Q = scaleQuantize({ domain: [0, 3], colors: ["#000000", "#ff0000", "#ffffff"] });
export default function Colors() {
  return <Scene key="scene">
    <View key="lo" style={{ backgroundColor: SEQ.map(0) }} />
    <View key="mid" style={{ backgroundColor: SEQ.map(5) }} />
    <View key="hi" style={{ backgroundColor: SEQ.map(99) }} />
    <View key="bin" style={{ backgroundColor: Q.map(1) }} />
    <View key="last" style={{ backgroundColor: Q.map(3) }} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source)
        .expect("color scales compile")
        .artifact;
    assert_eq!(
        static_color(&artifact, "lo", "background-color"),
        Rgba::rgb(0, 0, 0)
    );
    assert_eq!(
        static_color(&artifact, "mid", "background-color"),
        Rgba::rgb(0, 0, 0).mix(Rgba::rgb(255, 255, 255), 0.5)
    );
    assert_eq!(
        static_color(&artifact, "hi", "background-color"),
        Rgba::rgb(255, 255, 255)
    );
    assert_eq!(
        static_color(&artifact, "bin", "background-color"),
        Rgba::rgb(255, 0, 0)
    );
    assert_eq!(
        static_color(&artifact, "last", "background-color"),
        Rgba::rgb(255, 255, 255)
    );

    let graph = MotionModuleGraph::new(
        "main.motion.tsx",
        BTreeMap::from([
            (
                "palette.ts".into(),
                r##"export const tone = scaleQuantize({ domain: [0, 3], colors: ["#000000", "#ff0000", "#ffffff"] });"##.into(),
            ),
            (
                "main.motion.tsx".into(),
                r##"
import { tone } from "./palette";
export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export default function Main() {
  return <Scene key="scene"><View key="rebound" style={{ backgroundColor: tone.map(1) }} /></Scene>;
}
"##.into(),
            ),
        ]),
    )
    .expect("graph");
    let rebound = compile_motion_modules(&graph)
        .expect("rebind compiles")
        .artifact;
    assert_eq!(
        static_color(&rebound, "rebound", "background-color"),
        Rgba::rgb(255, 0, 0)
    );
}

#[test]
fn invalid_pie_curve_and_area_band_fail_closed() {
    for (source, needle) in [
        (
            r#"const S=pie([1e308,1e308], {endAngle:1}); export default function P(){return <View/>;}"#,
            "sum must be finite",
        ),
        (
            r#"const S=pie([-1]); export default function P(){return <View/>;}"#,
            "non-negative",
        ),
        (
            r#"const C=curve([point(0,0), point(0,1)], { type: "monotoneX" }); export default function P(){return <Path d={C}/>;}"#,
            "strictly increasing",
        ),
        (
            r#"const C=curve([point(0,0)]); export default function P(){return <Path d={C}/>;}"#,
            "at least two",
        ),
        (
            r#"export default function P(){return <Path d={areaBand(line([point(0,0), point(10,0)]), line([point(0,1), point(5,1), point(10,1)]))}/>;}"#,
            "areaBand",
        ),
        (
            r#"const B=areaBand(line([point(0,0),point(10,0)]),line([point(100,1),point(110,1)])); export default function P(){return <Path d={B}/>;}"#,
            "areaBand",
        ),
        (
            r#"export default function P(){return <Path d={areaBand(line([point(0,0),point(5,0),point(10,0)]),line([point(0,1),point(6,1),point(10,1)]))}/>;}"#,
            "areaBand",
        ),
    ] {
        let diagnostics = compile_motion(source).expect_err("invalid chart helper must fail");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "wanted {needle}, got {diagnostics:#?}"
        );
    }
}
