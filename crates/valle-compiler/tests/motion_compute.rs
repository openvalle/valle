#![cfg(feature = "motion")]
//! Compile Motion chart sources and verify that public compute functions produce static artifact coordinates.

use valle_compiler::motion::compile_motion;
use valle_motion::{MotionValue, StyleValue};

fn diagnostics_of(source: &str) -> Vec<String> {
    compile_motion(source)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Read a static numeric style value from a node.
fn static_number(artifact: &valle_motion::SceneArtifact, key: &str, property: &str) -> f64 {
    let node = artifact
        .nodes
        .iter()
        .find(|node| node.key == key)
        .unwrap_or_else(|| panic!("node `{key}` exists"));
    let binding = node
        .styles
        .iter()
        .find(|binding| binding.property == property)
        .unwrap_or_else(|| panic!("`{key}` binds `{property}`"));
    match &binding.value {
        StyleValue::Static {
            value: MotionValue::Number(value),
        } => *value,
        StyleValue::Static {
            value: MotionValue::Length(length),
        } => length.value,
        other => panic!("`{key}.{property}` should be a compile-time constant, got {other:?}"),
    }
}

/// Derive a bar chart's domain, ticks, positions and heights from data.
const BAR_CHART: &str = r##"
const DATA = [12, 45, 78, 33];
const [lo, hi] = niceDomain(0, extent(DATA)[1], 5);
const x = scaleBand({ range: [80, 1840], count: DATA.length, paddingInner: 0.2 });
const y = scaleLinear({ domain: [lo, hi], range: [960, 120] });

export default function Chart(ctx) {
  return (
    <Scene className="h-full w-full">
      {DATA.map((v, i) => (
        <View
          key={`bar-${i}`}
          style={{
            position: "absolute",
            left: x.band(i)[0],
            top: y.map(v),
            width: x.bandwidth(),
            height: y.map(lo) - y.map(v),
          }}
        />
      ))}
    </Scene>
  );
}
"##;

#[test]
fn an_author_can_compute_a_whole_bar_chart_at_prepare_time() {
    let compiled = compile_motion(BAR_CHART).expect("compute builtins reach the author");
    let artifact = &compiled.artifact;

    // The rounded [0, 80] domain maps zero to the bottom and 80 to the top.
    let base = static_number(artifact, "bar-0", "top");
    let tallest = static_number(artifact, "bar-2", "top");
    assert!(
        tallest < base,
        "78 must sit higher than 12: {tallest} vs {base}"
    );
    // The bar height is (960 - 120) * 78 / 80 = 819.
    let height = static_number(artifact, "bar-2", "height");
    assert!(
        (height - 819.0).abs() < 0.5,
        "the tallest bar should be 819px tall on a niced [0, 80] domain, got {height}"
    );

    // Four bands with 0.2 padding give a step of 1760 / 3.8.
    let width = static_number(artifact, "bar-0", "width");
    assert!(
        (width - 370.526_315).abs() < 0.01,
        "bandwidth should be step*(1-0.2), got {width}"
    );

    // All computed values must be static.
    for index in 0..4 {
        for property in ["left", "top", "width", "height"] {
            static_number(artifact, &format!("bar-{index}"), property);
        }
    }
}

/// Verify exact d3 reference outputs through the complete author-to-artifact conversion path.
#[test]
fn ticks_reaching_the_author_match_the_d3_golden_exactly() {
    let compiled = compile_motion(
        r##"
const T = ticks(0, 1, 5);
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    {T.map((v, i) => (
      <View key={`t-${i}`} style={{ position: "absolute", left: v * 1000, top: 0, width: 4, height: 10 }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("compiles");
    // Scaled d3 ticks must be exact integers, without floating-point drift.
    let lefts: Vec<f64> = (0..6)
        .map(|i| static_number(&compiled.artifact, &format!("t-{i}"), "left"))
        .collect();
    assert_eq!(lefts, vec![0.0, 200.0, 400.0, 600.0, 800.0, 1000.0]);
}

/// Invalid inputs produce diagnostics naming the argument.
#[test]
fn bad_compute_input_fails_closed_with_a_pointed_message() {
    let messages = diagnostics_of(
        r##"
const S = scaleLinear({ domain: [0], range: [0, 100] });
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: S.map(1), top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("domain") || message.contains("two")),
        "got {messages:#?}"
    );
}

/// Unknown operators report the supported names.
#[test]
fn an_unknown_scale_name_is_just_an_undefined_identifier_not_a_silent_zero() {
    let messages = diagnostics_of(
        r##"
const S = scaleLogarithmic({ domain: [1, 100], range: [0, 100] });
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: S.map(1), top: 0, width: 10, height: 10 }} />
  </Scene>);
}
"##,
    );
    assert!(
        !messages.is_empty(),
        "an undefined builtin must fail the compile, not silently produce nothing"
    );
}

/// Pure compute builtins require no external resources or measurement environment.
#[test]
fn compute_builtins_need_no_capability_flag_or_font_bundle() {
    compile_motion(BAR_CHART).expect("no --font, no capability, still compiles");
}

// End-to-end projection, graph layout and noise tests.

/// Project synthetic geographic coordinates into static canvas coordinates.
#[test]
fn an_author_can_project_coordinates_at_prepare_time() {
    let compiled = compile_motion(
        r##"
const RING = [[100, 20], [110, 20], [110, 40], [100, 40]];
const PTS = geoProject({ points: RING, projection: "albers", width: 1920, height: 1080 });
export default function Map(ctx) {
  return (<Scene className="h-full w-full">
    {PTS.map((p, i) => (
      <View key={`p-${i}`} style={{ position: "absolute", left: p[0], top: p[1], width: 8, height: 8 }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("geoProject reaches the author");
    let artifact = &compiled.artifact;
    let x = |i: usize| static_number(artifact, &format!("p-{i}"), "left");
    let y = |i: usize| static_number(artifact, &format!("p-{i}"), "top");
    // Contain all points within the canvas and fill one dimension.
    for i in 0..4 {
        assert!((0.0..=1920.0).contains(&x(i)), "point {i} x = {}", x(i));
        assert!((0.0..=1080.0).contains(&y(i)), "point {i} y = {}", y(i));
    }
    // Higher latitudes map to smaller canvas y coordinates.
    assert!(y(3) < y(0), "north must be up: {} vs {}", y(3), y(0));
    // The ring is symmetric around the canvas centerline.
    assert!(
        ((x(0) + x(1)) / 2.0 - 960.0).abs() < 1.0,
        "the ring should be centred: {} {}",
        x(0),
        x(1)
    );
}

/// Prepare graph coordinates from node sizes and edges.
#[test]
fn an_author_can_lay_out_a_graph_at_prepare_time() {
    let compiled = compile_motion(
        r##"
const SIZES = [[160, 60], [160, 60], [160, 60], [160, 60]];
const EDGES = [[0, 1], [0, 2], [1, 3], [2, 3]];
const G = graphLayout({ sizes: SIZES, edges: EDGES, nodeGap: 40, rankGap: 80 });
export default function Diagram(ctx) {
  return (<Scene className="h-full w-full">
    {G.centers.map((c, i) => (
      <View key={`n-${i}`} style={{ position: "absolute", left: c[0] - 80, top: c[1] - 30, width: 160, height: 60 }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("graphLayout reaches the author");
    let artifact = &compiled.artifact;
    let top = |i: usize| static_number(artifact, &format!("n-{i}"), "top");
    let left = |i: usize| static_number(artifact, &format!("n-{i}"), "left");
    // Nodes 1 and 2 share a layer between nodes 0 and 3.
    assert!(top(0) < top(1), "rank 0 above rank 1");
    assert!((top(1) - top(2)).abs() < 0.01, "1 and 2 share a rank");
    assert!(top(2) < top(3), "rank 1 above rank 2");
    // Nodes on the same layer are separated by at least nodeGap.
    let gap = (left(1) - left(2)).abs() - 160.0;
    assert!(gap >= 39.9, "same-rank nodes must keep the gap, got {gap}");
    // Center the merge node between its predecessors.
    let middle = (left(1) + left(2)) / 2.0;
    assert!(
        (left(3) - middle).abs() < 40.0,
        "the join should sit between its parents: {} vs {middle}",
        left(3)
    );
}

/// Random values require an explicit seed; Math.random remains unavailable.
#[test]
fn randomness_is_available_only_with_an_explicit_seed() {
    let jitter = |seed: u32| {
        let source = format!(
            r##"
const J = seededRandom({seed}, 4, [-10, 10]);
export default function P(ctx) {{
  return (<Scene className="h-full w-full">
    {{J.map((v, i) => (
      <View key={{`j-${{i}}`}} style={{{{ position: "absolute", left: 100 + v, top: 0, width: 8, height: 8 }}}} />
    ))}}
  </Scene>);
}}
"##
        );
        let compiled = compile_motion(&source).expect("seededRandom compiles");
        (0..4)
            .map(|i| static_number(&compiled.artifact, &format!("j-{i}"), "left"))
            .collect::<Vec<_>>()
    };
    // The same seed produces the same sequence.
    assert_eq!(jitter(7), jitter(7));
    assert_ne!(jitter(7), jitter(8));
    // Values remain within the requested range.
    for value in jitter(7) {
        assert!((90.0..=110.0).contains(&value), "{value} escaped the range");
    }

    // Accessing Math.random must still fail.
    let messages = diagnostics_of(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: Math.random() * 10, top: 0, width: 8, height: 8 }} />
  </Scene>);
}
"##,
    );
    assert!(
        !messages.is_empty(),
        "Math.random must stay forbidden even after seededRandom lands"
    );
}

/// Nearby noise inputs produce nearby outputs.
#[test]
fn value_noise_reaching_the_author_is_continuous() {
    let compiled = compile_motion(
        r##"
const STEPS = [0, 1, 2, 3, 4, 5, 6, 7];
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    {STEPS.map((s, i) => (
      <View key={`n-${i}`} style={{ position: "absolute", left: i * 40, top: 500 + noise1d(3, i * 0.05) * 100, width: 8, height: 8 }} />
    ))}
  </Scene>);
}
"##,
    )
    .expect("noise1d reaches the author");
    let tops: Vec<f64> = (0..8)
        .map(|i| static_number(&compiled.artifact, &format!("n-{i}"), "top"))
        .collect();
    for pair in tops.windows(2) {
        assert!(
            (pair[1] - pair[0]).abs() < 10.0,
            "noise must be continuous, jumped from {} to {}",
            pair[0],
            pair[1]
        );
    }
    assert!(
        tops.windows(2).any(|w| (w[1] - w[0]).abs() > 1e-9),
        "noise must actually vary"
    );
}

/// Scale methods must remain available when the scale is bound inside a component.
#[test]
fn a_scale_bound_inside_the_component_keeps_its_methods() {
    let compiled = compile_motion(
        r##"
export default function P(ctx) {
  const s = scaleLinear({ domain: [0, 1], range: [0, 100] });
  const v = s.map(0.5);
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: v }} />
  </Scene>);
}
"##,
    )
    .expect("a scale bound in the component body must keep working");
    let width = compiled.artifact.nodes.iter().find_map(|node| {
        node.styles
            .iter()
            .find(|entry| entry.property == "width")
            .map(|entry| entry.value.clone())
    });
    assert_eq!(
        width,
        Some(StyleValue::Static {
            value: MotionValue::Number(50.0)
        }),
        "the scale must still compute, and at compile time"
    );
}

/// All installed compute builtins must be listed in AUTHOR_SURFACE so preparation-time diagnostics remain accurate.
#[test]
fn every_author_facing_builtin_is_listed_in_the_shared_surface() {
    for name in valle_motion::compute::AUTHOR_SURFACE {
        let source = format!(
            r##"
export default function P(ctx) {{
  const v = {name};
  return (<Scene className="h-full w-full">
    <View key="a" style={{{{ position: "absolute", left: 0, top: 0, width: 10, height: 10 }}}} />
  </Scene>);
}}
"##
        );
        // Referencing each name proves it is installed in the sandbox.
        let diagnostics = compile_motion(&source).err().unwrap_or_default();
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("is not defined")),
            "`{name}` is listed in AUTHOR_SURFACE but the JS shim does not define it"
        );
    }
}

/// Frame-time arguments to preparation builtins must explain the preparation boundary and canvas-space alternative.
#[test]
fn feeding_a_frame_time_value_to_a_prepare_time_builtin_points_at_the_canvas_space_pattern() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  const y = scaleLinear({ domain: [0, 100], range: [ctx.viewport.height * 0.86, 0] });
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10, width: y.map(50) }} />
  </Scene>);
}
"##,
    )
    .expect_err("a prepare-time builtin cannot take ctx.*");
    let message = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .find(|message| message.contains("scaleLinear"))
        .expect("the builtin must be named");
    assert!(
        message.contains("prepare-time builtin") && message.contains("Canvas space"),
        "the author needs to be told what to do instead: {message}"
    );
    assert!(
        !message.contains("MJ1.10") && !message.contains("pure helper"),
        "a builtin is not the author's own helper, and internal milestone numbers mean \
         nothing to them: {message}"
    );
}

/// Constant-folding probes must preserve builtin validation diagnostics.
#[test]
fn a_builtin_rejection_survives_the_folding_probe_in_a_stop_position() {
    let diagnostics = compile_motion(
        r##"
export default function P(ctx) {
  return (<Scene className="h-full w-full">
    <View key="a" style={{ position: "absolute", left: 0, top: 0, height: 10,
      width: interpolate(ctx.hold.progress, [0, scaleLinear({ domain: [0] }).map(0)], [0, 1]) }} />
  </Scene>);
}
"##,
    )
    .expect_err("a malformed builtin call must fail");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("`domain` must have exactly two numbers")),
        "the builtin's own words must reach the author: {diagnostics:?}"
    );
}
