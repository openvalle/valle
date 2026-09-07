#![cfg(feature = "motion")]
//! Constant arrays, fixed dynamic tuples and bounded map expansion into scalar expressions.
use std::collections::BTreeMap;

use valle_compiler::motion::{MotionModuleGraph, compile_motion, compile_motion_modules};
use valle_motion::{
    Expr, Extrapolation, InterpolateStop, MotionEasing, NodeKind, PathValue, StyleValue,
};

fn compile_ok(source: &str) -> valle_compiler::motion::CompiledMotion {
    compile_motion(source)
        .unwrap_or_else(|diagnostics| panic!("valid array source must compile: {diagnostics:#?}"))
}

fn diagnostics(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid array source must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn interpolate_payload(
    compiled: &valle_compiler::motion::CompiledMotion,
) -> (
    Vec<InterpolateStop>,
    Vec<MotionEasing>,
    Extrapolation,
    Extrapolation,
) {
    compiled
        .artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::Interpolate {
                stops,
                easings,
                extrapolate_left,
                extrapolate_right,
                ..
            } => Some((
                stops.clone(),
                easings.clone(),
                *extrapolate_left,
                *extrapolate_right,
            )),
            _ => None,
        })
        .expect("expected Expr::Interpolate")
}

fn path_line_len(compiled: &valle_compiler::motion::CompiledMotion) -> usize {
    compiled
        .artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathLine { points } => Some(points.len()),
            _ => None,
        })
        .expect("expected Expr::PathLine")
}

fn width_source(ranges: &str) -> String {
    format!(
        r##"
export default function Card(ctx) {{
  return <View style={{{{ width: interpolate(ctx.hold.progress, {ranges}) }}}} />;
}}
"##
    )
}

#[test]
fn interpolate_module_const_matches_inline_bytes() {
    let inline = compile_ok(&width_source("[0, 1], [0, 40]"));
    let named = compile_ok(
        r##"
const INPUT = [0, 1];
const OUTPUT = [0, 40];
export default function Card(ctx) {
  return <View style={{ width: interpolate(ctx.hold.progress, INPUT, OUTPUT) }} />;
}
"##,
    );
    assert_eq!(
        interpolate_payload(&inline),
        interpolate_payload(&named),
        "const arrays and inline literals must produce the same Interpolate payload"
    );
}

#[test]
fn interpolate_as_const_and_parens_match_inline() {
    let inline = compile_ok(&width_source("[0, 1], [0, 40]"));
    let wrapped = compile_ok(&width_source("([0, 1]) as const, [0, 40] as const"));
    assert_eq!(interpolate_payload(&inline), interpolate_payload(&wrapped));
}

#[test]
fn interpolate_easing_const_string_and_array() {
    let named = compile_ok(
        r##"
const EASE = "easeOut";
export default function Card(ctx) {
  return <View style={{ width: interpolate(ctx.hold.progress, [0, 1], [0, 40], { easing: EASE }) }} />;
}
"##,
    );
    let inline = compile_ok(&width_source("[0, 1], [0, 40], { easing: \"easeOut\" }"));
    assert_eq!(interpolate_payload(&inline), interpolate_payload(&named));

    let named_arr = compile_ok(
        r##"
const EASES = ["easeIn", "easeOut"];
export default function Card(ctx) {
  return <View style={{ width: interpolate(ctx.hold.progress, [0, 0.5, 1], [0, 20, 40], { easing: EASES }) }} />;
}
"##,
    );
    assert!(
        named_arr
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Interpolate { easings, .. } if easings.len() == 2)),
        "const easing arrays must fill one easing per segment"
    );
}

#[test]
fn interpolate_cross_module_const_array() {
    let graph = MotionModuleGraph::new(
        "entry.motion.tsx",
        BTreeMap::from([
            (
                "entry.motion.tsx".into(),
                r#"
import { INPUT } from "./stops.motion";
export default function Card(ctx) {
  return <View style={{ width: interpolate(ctx.hold.progress, INPUT, [0, 1]) }} />;
}
"#
                .into(),
            ),
            (
                "stops.motion.ts".into(),
                "export const INPUT = [0, 1];\n".into(),
            ),
        ]),
    )
    .expect("graph");
    let compiled = compile_motion_modules(&graph).expect("cross-module const array");
    assert!(
        compiled
            .artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::Interpolate { stops, .. } if stops.len() == 2))
    );
}

#[test]
fn line_map_unrolls_to_existing_path_line() {
    let compiled = compile_ok(
        r##"
const OFFSETS = [0, 8, 16];
export default function Card(ctx) {
  const t = ctx.hold.frame;
  return <Path fill="none" stroke="#fff" d={line(OFFSETS.map((off) => point(off, t)))} />;
}
"##,
    );
    assert_eq!(path_line_len(&compiled), 3);
    assert!(
        compiled.artifact.nodes.iter().any(|node| matches!(
            node.kind,
            NodeKind::Path {
                d: PathValue::Expr { .. },
                ..
            }
        )),
        "frame-capturing .map() must stay a PathLine Expr, not a prepare-time PathData"
    );
}

#[test]
fn line_map_two_seventy_points_is_one_path_line() {
    let compiled = compile_ok(
        r##"
const INDICES = Array.from({ length: 270 }, (_, i) => i);
export default function Card(ctx) {
  const t = ctx.hold.frame;
  return <Path fill="none" stroke="#fff" d={line(INDICES.map((i) => point(i, t)))} />;
}
"##,
    );
    assert_eq!(path_line_len(&compiled), 270);
}

#[test]
fn fixed_dynamic_tuple_survives_component_props_and_static_map_scope() {
    let compiled = compile_ok(
        r##"
const INDICES = [0, 1, 2];
function Layers(ctx, { values }) {
  return <LayerList key="list" values={values} />;
}
function LayerList(ctx, props) {
  return <View key="layers">
    {INDICES.map((_, i) => (
      <View key={`item-${i}`} style={{ opacity: props.values[i] }} />
    ))}
  </View>;
}
export default function Card(ctx) {
  const t = ctx.hold.progress;
  const values = [t, t * 0.5, 1 - t];
  return <Scene key="scene"><Layers key="group" values={values} /></Scene>;
}
"##,
    );
    compiled
        .artifact
        .validate()
        .expect("valid tuple-expanded IR");

    for key in ["item-0", "item-1", "item-2"] {
        let opacity = compiled
            .artifact
            .nodes
            .iter()
            .find(|node| node.key.ends_with(key))
            .unwrap_or_else(|| panic!("missing {key}"))
            .styles
            .iter()
            .find(|binding| binding.property == "opacity")
            .unwrap_or_else(|| panic!("missing opacity on {key}"));
        assert!(
            matches!(opacity.value, StyleValue::Expr { .. }),
            "{key} must retain its frame-time scalar expression"
        );
    }
}

#[test]
fn static_map_can_build_a_fixed_dynamic_scalar_tuple() {
    let compiled = compile_ok(
        r##"
const HEIGHTS = [40, 80, 120];
const INDICES = [0, 1, 2];
export default function Card(ctx) {
  const f = ctx.hold.frame;
  const lifts = HEIGHTS.map((height, i) => {
    const p = Math.min(Math.max((f - i * 3) / 9, 0), 1);
    return height * (1 - p * p);
  });
  return <Scene>{INDICES.map((i) => (
    <View key={`card-${i}`} style={{ transform: `translateZ(${lifts[i]}px)` }} />
  ))}</Scene>;
}
"##,
    );
    compiled.artifact.validate().expect("valid mapped tuple IR");
    for key in ["card-0", "card-1", "card-2"] {
        assert!(
            compiled
                .artifact
                .nodes
                .iter()
                .any(|node| node.key.ends_with(key)),
            "missing {key}"
        );
    }
}

#[test]
fn inline_dynamic_tuple_can_flow_through_a_helper() {
    let compiled = compile_ok(
        r##"
const first = (values) => values[0];
export default function Card(ctx) {
  const t = ctx.hold.progress;
  return <View key="card" style={{ opacity: first([t, 1 - t]) }} />;
}
"##,
    );
    let opacity = compiled
        .artifact
        .nodes
        .iter()
        .find(|node| node.key == "card")
        .expect("card")
        .styles
        .iter()
        .find(|binding| binding.property == "opacity")
        .expect("opacity");
    assert!(matches!(opacity.value, StyleValue::Expr { .. }));
}

#[test]
fn dynamic_tuple_requires_a_static_in_bounds_index() {
    let dynamic_index = diagnostics(
        r##"
export default function Card(ctx) {
  const t = ctx.hold.progress;
  const values = [t, 1 - t];
  return <View style={{ opacity: values[Math.floor(t * 2)] }} />;
}
"##,
    );
    assert!(
        dynamic_index
            .iter()
            .any(|message| message.contains("known at compile time")),
        "dynamic tuple indexing must fail closed: {dynamic_index:?}"
    );

    let out_of_bounds = diagnostics(
        r##"
export default function Card(ctx) {
  const t = ctx.hold.progress;
  const values = [t, 1 - t];
  return <View style={{ opacity: values[2] }} />;
}
"##,
    );
    assert!(
        out_of_bounds
            .iter()
            .any(|message| message.contains("out of bounds")),
        "out-of-bounds tuple indexing must fail closed: {out_of_bounds:?}"
    );
}

#[test]
fn map_spread_elision_runtime_and_over_budget_fail_closed() {
    let spread = diagnostics(
        r##"
const OFFSETS = [0, 8];
export default function Card(ctx) {
  const t = ctx.hold.frame;
  return <Path fill="none" stroke="#fff" d={line([...OFFSETS].map((off) => point(off, t)))} />;
}
"##,
    );
    assert!(
        spread.iter().any(|message| message.contains("spread")),
        "spread must fail closed: {spread:?}"
    );
    let holes = diagnostics(
        r##"
export default function Card(ctx) {
  const t = ctx.hold.frame;
  return <Path fill="none" stroke="#fff" d={line([0, , 2].map((off) => point(off, t)))} />;
}
"##,
    );
    assert!(
        holes
            .iter()
            .any(|message| message.contains("holes") || message.contains("hole")),
        "elision must fail closed: {holes:?}"
    );
    let runtime = diagnostics(
        r##"
export default function Card(ctx) {
  return <Path fill="none" stroke="#fff" d={line(ctx.hold.progress.map((off) => point(off, 0)))} />;
}
"##,
    );
    assert!(
        runtime
            .iter()
            .any(|message| message.contains("ctx") || message.contains("frame")),
        "frame-dependent collection length must fail closed: {runtime:?}"
    );
    let over = format!(
        r##"
const N = Array.from({{ length: {} }}, (_, i) => i);
export default function Card(ctx) {{
  const t = ctx.hold.frame;
  return <Path fill="none" stroke="#fff" d={{line(N.map((i) => point(i, t)))}} />;
}}
"##,
        4097
    );
    let over_msgs = diagnostics(&over);
    assert!(
        over_msgs
            .iter()
            .any(|message| message.contains("4096") || message.contains("budget")),
        "unroll over 4096 must fail closed: {over_msgs:?}"
    );
}

#[test]
fn mapped_path_is_shared_by_multiple_nodes() {
    let compiled = compile_ok(
        r##"
const SAMPLES = [0, 10, 20, 30];
export default function Probe(ctx) {
  const path = line(SAMPLES.map((x) => point(x, ctx.hold.frame + x)));
  return <Scene>
    <Path key="front" d={path} fill="none" stroke="#fff" />
    <Path key="back" d={path} fill="none" stroke="#888" />
  </Scene>;
}
"##,
    );
    compiled.artifact.validate().unwrap();
    assert_eq!(path_line_len(&compiled), 4);
    assert_eq!(
        compiled
            .artifact
            .exprs
            .iter()
            .filter(|expr| matches!(expr, Expr::PathLine { .. }))
            .count(),
        1,
        "reusing a local path must not duplicate its expression"
    );
    let paths: Vec<_> = compiled
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Path { d, .. } => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(paths.len(), 2);
    assert!(matches!(paths[0], PathValue::Expr { .. }));
    assert_eq!(paths[0], paths[1]);
}

#[test]
fn nested_parameter_maps_expand_distinct_dynamic_nodes() {
    let compiled = compile_ok(
        r##"
const GROUPS = [{ count: 2, delay: 0 }, { count: 3, delay: 4 }];
export default function Probe(ctx) {
  return <Scene>{GROUPS.map((group, g) => (
    <View key={`group-${g}`}>
      {Array.from({ length: group.count }, (_, i) => i).map((i) => (
        <View key={`dot-${g}-${i}`} visible={ctx.hold.frame >= group.delay + i}
          style={{ left: i * 8, top: ctx.hold.frame + g }} />
      ))}
    </View>
  ))}</Scene>;
}
"##,
    );
    compiled.artifact.validate().unwrap();
    let dots: Vec<_> = compiled
        .artifact
        .nodes
        .iter()
        .filter(|node| node.key.starts_with("dot-"))
        .collect();
    let keys: std::collections::BTreeSet<_> = dots.iter().map(|node| node.key.as_str()).collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from(["dot-0-0", "dot-0-1", "dot-1-0", "dot-1-1", "dot-1-2"])
    );
    assert_eq!(dots.len(), 5);
    for node in dots {
        let top = node
            .styles
            .iter()
            .find(|binding| binding.property == "top")
            .unwrap();
        assert!(matches!(top.value, StyleValue::Expr { .. }));
    }
}

#[test]
fn repeated_components_share_frame_expressions() {
    let compiled = compile_ok(
        r##"
const WIDTHS = [12, 24, 36];
function Tile(ctx, { width }) {
  const size = width * ctx.hold.progress;
  return <View key="tile" style={{ width: size, height: size }} />;
}
export default function Probe(ctx) {
  return <Scene>{WIDTHS.map((width, i) => <Tile key={`instance-${i}`} width={width} />)}</Scene>;
}
"##,
    );
    compiled.artifact.validate().unwrap();
    let tiles: Vec<_> = compiled
        .artifact
        .nodes
        .iter()
        .filter(|node| node.key.ends_with("tile"))
        .collect();
    assert_eq!(tiles.len(), 3);
    let mut expressions = Vec::new();
    for node in tiles {
        let width = &node
            .styles
            .iter()
            .find(|binding| binding.property == "width")
            .unwrap()
            .value;
        let height = &node
            .styles
            .iter()
            .find(|binding| binding.property == "height")
            .unwrap()
            .value;
        assert!(matches!(width, StyleValue::Expr { .. }));
        assert_eq!(
            width, height,
            "one local scalar must be reused across style channels"
        );
        expressions.push(width);
    }
    assert_ne!(
        expressions[0], expressions[1],
        "component props must remain distinct"
    );
    assert_ne!(expressions[1], expressions[2]);
    assert_ne!(expressions[0], expressions[2]);
}

#[test]
fn sparse_prepared_ids_preserve_keys_and_content_kinds() {
    let compiled = compile_ok(
        r##"
const IDS = [0, 2, 5];
const CELLS = IDS.map((id, kind) => ({ id, kind }));
export default function Probe(ctx) {
  return <Scene>{CELLS.map((cell) => (
    <View key={`cell-${cell.id}`} style={{ left: cell.id * 10 }}>
      {cell.kind === 0 ? <Text key="label">A</Text> :
       cell.kind === 1 ? <Path key="mark" d="M0 0 L8 8" fill="none" stroke="#fff" /> :
       <View key="block" style={{ width: 8, height: 8 }} />}
    </View>
  ))}</Scene>;
}
"##,
    );
    compiled.artifact.validate().unwrap();
    let keys: std::collections::BTreeSet<_> = compiled
        .artifact
        .nodes
        .iter()
        .filter(|node| node.key.starts_with("cell-"))
        .map(|node| node.key.as_str())
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from(["cell-0", "cell-2", "cell-5"])
    );
    for key in ["label", "mark", "block"] {
        assert_eq!(
            compiled
                .artifact
                .nodes
                .iter()
                .filter(|node| node.key == key)
                .count(),
            1,
            "prepare-time branches must only emit their selected content"
        );
    }
}

#[test]
fn lookup_table_is_still_not_a_builtin() {
    let messages = diagnostics(
        r##"
const TABLE = [1, 2, 3];
export default function Card(ctx) {
  return <View style={{ opacity: lookupTable(TABLE, ctx.hold.frame, { rounding: "floor", bounds: "clamp" }) }} />;
}
"##,
    );
    assert!(
        messages.iter().any(|message| {
            message.contains("lookupTable")
                || message.contains("unknown")
                || message.contains("not")
        }),
        "{messages:?}"
    );
}
