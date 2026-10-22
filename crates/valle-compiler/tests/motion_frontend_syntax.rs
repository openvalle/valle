#![cfg(feature = "motion")]
//! React and JavaScript forms that Motion sources accept as written.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, MotionValue, NodeKind, NumberValue, Scene3DLightBinding, SceneArtifact, StyleValue,
    motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

fn compile(source: &str) -> SceneArtifact {
    match compile_motion(source) {
        Ok(compiled) => compiled.artifact,
        Err(diagnostics) => panic!(
            "source compiles: {:#?}",
            diagnostics
                .iter()
                .map(|diagnostic| &diagnostic.message)
                .collect::<Vec<_>>()
        ),
    }
}

fn children(artifact: &SceneArtifact, key: &str) -> Vec<String> {
    let node = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .unwrap_or_else(|| panic!("node `{key}` exists"));
    artifact.node_children[node.children.start as usize..node.children.end as usize]
        .iter()
        .map(|id| artifact.nodes[id.0 as usize].key.clone())
        .collect()
}

/// The value of `property` on node `key` at frame 30 of 60.
fn style_number(artifact: &SceneArtifact, key: &str, property: &str) -> f64 {
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let ctx = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).expect("context");
    let values = valle_motion::eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: Some((320.0, 180.0)),
        },
    )
    .expect("evaluate");
    let binding = node(artifact, key)
        .styles
        .iter()
        .find(|style| style.property == property)
        .unwrap_or_else(|| panic!("{key}.{property} exists"));
    match &binding.value {
        StyleValue::Static { value } => number(value),
        StyleValue::Expr { expr } => number(&values[expr.0 as usize]),
    }
}

fn number(value: &MotionValue) -> f64 {
    match value {
        MotionValue::Number(value) => *value,
        MotionValue::Length(length) => length.value,
        other => panic!("expected a number, got {other:?}"),
    }
}

fn node<'a>(artifact: &'a SceneArtifact, key: &str) -> &'a valle_motion::SceneNode {
    artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .unwrap_or_else(|| panic!("node `{key}` exists"))
}

const BOX: &str = "style={{ width: 20, height: 20 }}";

#[test]
fn fragments_put_their_children_in_their_place() {
    let artifact = compile(&format!(
        r##"
const theme = {{ colors: {{ accent: "#38bdf8" }} }};
const ROWS = [{{ id: "x" }}, {{ id: "y" }}];
function Pair() {{ const clock = useCurrentTime();
  return <><View key="a" {BOX} /><View key="b" {BOX} /></>;
}}
export default function D() {{ const clock = useCurrentTime();
  return <Scene>
    <View key="row" className="flex">
      <><View key="first" {BOX} /><View key="second" {BOX} /></>
      <Pair key="pair" />
      {{clock.progress > 0.5 && <><View key="late" {BOX} /><View key="later" {BOX} /></>}}
      {{ROWS.map((row) => <><View key={{`${{row.id}}-a`}} {BOX} /><View key={{`${{row.id}}-b`}} {BOX} /></>)}}
      <ThemeProvider value={{theme}}>
        <View key="themed-a" style={{{{ width: 20, height: 20, backgroundColor: useTheme().colors.accent }}}} />
        <View key="themed-b" {BOX} />
      </ThemeProvider>
    </View>
  </Scene>;
}}
"##
    ));
    assert_eq!(
        children(&artifact, "row"),
        [
            "first", "second", "pair/a", "pair/b", "late", "later", "x-a", "x-b", "y-a", "y-b",
            "themed-a", "themed-b"
        ]
    );
    let gated = |key: &str| {
        artifact
            .nodes
            .iter()
            .find(|node| node.key == key)
            .is_some_and(|node| node.visibility.is_some())
    };
    assert!(gated("late") && gated("later") && !gated("first"));
}

#[test]
fn a_root_fragment_becomes_the_scene_root() {
    let artifact = compile(&format!(
        r##"
export default function D() {{
  return <><View key="a" {BOX} /><View key="b" {BOX} /></>;
}}
"##
    ));
    let root = &artifact.nodes[artifact.root.0 as usize];
    assert_eq!(children(&artifact, &root.key), ["a", "b"]);
}

#[test]
fn spread_attributes_set_what_the_object_holds_and_the_last_setting_wins() {
    let artifact = compile(
        r##"
const SHARED = { className: "absolute", style: { width: 10, height: 10 } };
const ITEMS = [{ id: "one", label: "One", size: 12 }, { id: "two", label: "Two", size: 14 }];
function Tag({ id, label, size, ...rest }) {
  return <View key="tag" {...rest} style={{ width: size, height: size }}>
    <Text key="label">{label}</Text>
  </View>;
}
function Box(props) {
  return <View key="box" {...props} />;
}
export default function D() { const clock = useCurrentTime();
  const local = { style: { width: 30, height: 30, opacity: clock.progress } };
  return <Scene>
    <View key="shared" {...SHARED} />
    <View key="local" {...local} />
    <View key="override" {...SHARED} className="relative" />
    <View key="kept" className="relative" {...SHARED} />
    {ITEMS.map((item) => <Tag key={item.id} {...item} className="flex" />)}
    <Box key="boxed" className="absolute">
      <View key="inside" style={{ width: 5, height: 5 }} />
    </Box>
  </Scene>;
}
"##,
    );
    let classes = |key: &str| node(&artifact, key).class_names.clone();
    assert_eq!(classes("shared"), ["absolute"]);
    assert_eq!(style_number(&artifact, "shared", "width"), 10.0);
    assert_eq!(style_number(&artifact, "local", "opacity"), 0.5);
    assert_eq!(classes("override"), ["relative"]);
    assert_eq!(classes("kept"), ["absolute"]);
    assert_eq!(classes("one/tag"), ["flex"]);
    assert_eq!(style_number(&artifact, "two/tag", "width"), 14.0);
    assert_eq!(classes("boxed/box"), ["absolute"]);
    assert_eq!(children(&artifact, "boxed/box"), ["boxed/inside"]);
}

#[test]
fn spread_attributes_reach_scene3d_children_and_spans() {
    let artifact = compile(
        r##"
const LAMBERT = { material: { type: "lambert", color: "#ffffff" }, rotateY: 15 };
const STRONG = { style: { fontWeight: 700 } };
export default function D() {
  return <Scene style={{ width: 128, height: 128 }}>
    <Scene3D key="s" camera={{ position: [0, 0, 5], target: [0, 0, 0] }} style={{ width: 128, height: 128 }}>
      <Mesh key="shape" geometry={extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"), { depth: 0.8 })} {...LAMBERT} />
      <AmbientLight {...{ intensity: 0.3 }} />
    </Scene3D>
    <Text key="t">plain <Span {...STRONG}>bold</Span></Text>
  </Scene>;
}
"##,
    );
    let frame = artifact
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Scene3D { frame, .. } => Some(frame),
            _ => None,
        })
        .expect("a Scene3D node");
    assert_eq!(
        frame.meshes[0].transform.rotation_degrees[1],
        NumberValue::Static { value: 15.0 }
    );
    assert!(matches!(
        &frame.lights[0],
        Scene3DLightBinding::Ambient { intensity: NumberValue::Static { value }, .. } if *value == 0.3
    ));
    let bold = artifact
        .nodes
        .iter()
        .find(|node| {
            node.styles
                .iter()
                .any(|style| style.property == "font-weight")
        })
        .expect("the Span run carries its weight");
    assert_eq!(style_number(&artifact, &bold.key, "font-weight"), 700.0);
}

#[test]
fn destructuring_takes_rests_and_computed_keys() {
    let artifact = compile(
        r##"
export const controls = { props: { size: number({ default: 24 }), tint: number({ default: 0.5 }) } };
const POINT = { x: 4, y: 6, z: 8 };
const LIST = [1, 2, 3, 4];
const FIELD = "z";
function Swatch({ label, ...style }) {
  return <View key="swatch" style={{ width: style.size, height: style.size, opacity: style.tint }} />;
}
export default function D({ size, ...others }) {
  const { x, ...rest } = POINT;
  const { [FIELD]: picked, ...flat } = POINT;
  const [first, ...more] = LIST;
  return <Scene>
    <View key="data" style={{ width: rest.y + rest.z, height: first + more.length, left: picked + flat.x, opacity: others.tint }} />
    <Swatch key="s" label="A" size={size} tint={0.25} />
  </Scene>;
}
"##,
    );
    assert_eq!(style_number(&artifact, "data", "width"), 14.0);
    assert_eq!(style_number(&artifact, "data", "height"), 4.0);
    assert_eq!(style_number(&artifact, "data", "left"), 12.0);
    assert_eq!(style_number(&artifact, "data", "opacity"), 0.5);
    assert_eq!(style_number(&artifact, "s/swatch", "width"), 24.0);
    assert_eq!(style_number(&artifact, "s/swatch", "opacity"), 0.25);
}

#[test]
fn array_spreads_write_out_prepared_arrays_and_fixed_tuples() {
    let artifact = compile(
        r##"
const BASE = [10, 20];
const INPUT = [0, 1];
export default function D() { const clock = useCurrentTime();
  const moving = [...BASE, clock.progress * 100];
  const both = [...moving, ...BASE];
  return <Scene>
    <View key="a" style={{ left: moving[2], top: both[4], width: interpolate(clock.progress, [...INPUT], [...BASE]), height: 5 }} />
  </Scene>;
}
"##,
    );
    assert_eq!(style_number(&artifact, "a", "left"), 50.0);
    assert_eq!(style_number(&artifact, "a", "top"), 20.0);
    assert_eq!(style_number(&artifact, "a", "width"), 15.0);
}

#[test]
fn the_time_is_read_where_it_is_used_and_passes_on_like_a_value() {
    let artifact = compile(
        r##"
const later = (time, at) => time.seconds - at;
const now = () => useCurrentTime().seconds;
function Probe({ time }) {
  return <View key="probe" style={{ width: 10, height: 10, left: time.seconds * 10 }} />;
}
function Clock(props) {
  return <View key="clock" style={{ width: 10, height: 10, left: props.time.seconds * 10 }} />;
}
function Own() {
  const { seconds } = useCurrentTime();
  return <View key="own" style={{ width: 10, height: 10, left: seconds * 10 }} />;
}
export default function D() {
  const clock = useCurrentTime();
  const { seconds } = useCurrentTime();
  const { fps: rate } = useComposition();
  return <Scene>
    <View key="alias" style={{ width: 10, height: 10, left: clock.seconds * 10 }} />
    <View key="inline" style={{ width: 10, height: 10, left: useCurrentTime().seconds * 10 }} />
    <View key="destructured" style={{ width: 10, height: 10, left: seconds * 10, top: rate }} />
    <View key="helper" style={{ width: 10, height: 10, left: (later(clock, 0.5) + 0.5) * 10 }} />
    <View key="hook-helper" style={{ width: 10, height: 10, left: now() * 10 }} />
    <Probe key="prop" time={clock} />
    <Clock key="member" time={clock} />
    <TimeScope key="scope" offset={0.25} speed={1}>
      <Own key="scoped-hook" />
    </TimeScope>
  </Scene>;
}
"##,
    );
    // Frame 30 at 30 fps is one second in.
    for key in [
        "alias",
        "inline",
        "destructured",
        "helper",
        "hook-helper",
        "prop/probe",
        "member/clock",
    ] {
        assert_eq!(style_number(&artifact, key, "left"), 10.0, "{key}");
    }
    assert_eq!(style_number(&artifact, "destructured", "top"), 30.0);
    // A component drawn inside the TimeScope reads the scope's clock.
    assert_eq!(style_number(&artifact, "scoped-hook/own", "left"), 7.5);
}

#[test]
fn scene_time_of_an_outer_component_cannot_be_drawn_inside_a_time_scope() {
    for read in ["clock.seconds", "seconds"] {
        let source = format!(
            r##"
export default function D() {{
  const clock = useCurrentTime();
  const {{ seconds }} = useCurrentTime();
  return <Scene>
    <TimeScope key="scope" offset={{0.25}} speed={{1}}>
      <View key="scoped" style={{{{ width: 10, height: 10, left: {read} * 10 }}}} />
    </TimeScope>
  </Scene>;
}}
"##
        );
        let diagnostics = compile_motion(&source).expect_err(read);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("drawn inside a TimeScope")),
            "{read}: {:#?}",
            diagnostics
                .iter()
                .map(|diagnostic| &diagnostic.message)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn one_source_imports_builtins_from_valle() {
    let artifact = compile(
        r##"
import { Scene, View, useCurrentTime } from "valle";
export default function D() {
  const clock = useCurrentTime();
  return <Scene><View key="a" style={{ width: 10, height: 10, left: clock.seconds * 10 }} /></Scene>;
}
"##,
    );
    assert_eq!(style_number(&artifact, "a", "left"), 10.0);
    let messages = compile_motion(
        "import { useState } from \"valle\";\nexport default function D() { const [n] = useState(0); return <Scene />; }",
    )
    .err()
    .unwrap_or_default();
    assert!(
        messages
            .iter()
            .any(|diagnostic| diagnostic.message.contains("`useState` is not available")),
        "{messages:#?}"
    );
}

#[test]
fn one_source_reads_its_typescript_as_javascript() {
    let artifact = compile(
        r##"
type Size = { width: number; height: number };
interface Tinted { opacity: number }
const BASE: number = 10;
const SCALE = { small: 2, large: 3 } satisfies Record<string, number>;
function scaled<T extends number>(value: T, factor?: number): number {
  return value * (factor ?? SCALE.small);
}
const half = (value: number): number => (value as number) / 2;
function tinted(size: Size): Size & Tinted {
  return { width: size.width!, height: size.height, opacity: 0.5 };
}
export default function D() {
  const size = tinted({ width: scaled(BASE), height: half(BASE) + SCALE.large });
  return <Scene>
    <View key="a" style={{ width: size.width, height: size.height, opacity: size.opacity }} />
  </Scene>;
}
"##,
    );
    assert_eq!(style_number(&artifact, "a", "width"), 20.0);
    assert_eq!(style_number(&artifact, "a", "height"), 8.0);
    assert_eq!(style_number(&artifact, "a", "opacity"), 0.5);
}
