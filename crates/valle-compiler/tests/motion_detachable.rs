#![cfg(feature = "motion")]
//! Chart, map and graph fixtures use only public compute functions and visual primitives. They require no domain-specific compiler builtins.

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{FontResource, Fonts, LayoutOptions, Viewport, build_tree, prepare_scene};
use valle_motion::{ResolvedSignals, motion_context_at, phase_windows, resolve_props};
use valle_timeline::FrameRate;

const FONT: &[u8] =
    include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

const BAR_CHART: &str = include_str!("fixtures/motion/charts/bar-chart.motion.tsx");
const WORLD_MAP: &str = include_str!("fixtures/motion/charts/world-map.motion.tsx");
const DIAGRAM: &str = include_str!("fixtures/motion/charts/architecture-diagram.motion.tsx");

const FILMS: &[(&str, &str)] = &[
    ("bar-chart", BAR_CHART),
    ("world-map", WORLD_MAP),
    ("architecture-diagram", DIAGRAM),
];

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(FONT.to_vec()))
        .expect("register font");
    fonts
}

/// Layout a scene at a given viewport and return its node boxes.
fn boxes_at(source: &str, width: u32, height: u32) -> BTreeMap<String, [f32; 4]> {
    let compiled = compile_motion(source).expect("reference film compiles");
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).expect("props");
    let phases = phase_windows(&compiled.artifact.controls.phase_spec(), 60);
    let ctx = motion_context_at(30, &phases, FrameRate::new(30, 1).unwrap()).expect("frame 30");
    let fonts = fonts();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((width, height)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    valle_motion::layout_boxes(&compiled.artifact, &tree).expect("boxes")
}

// Data-driven compilation.

#[test]
fn all_three_reference_films_compile() {
    for (name, source) in FILMS {
        let compiled =
            compile_motion(source).unwrap_or_else(|d| panic!("{name} must compile: {d:#?}"));
        compiled
            .artifact
            .validate()
            .unwrap_or_else(|e| panic!("{name} must validate: {e:#?}"));
    }
}

/// The node count must follow the input data length.
#[test]
fn node_counts_follow_the_data_not_a_hand_written_list() {
    let six = compile_motion(BAR_CHART).expect("compiles");
    // Remove only two data rows.
    let four = compile_motion(&BAR_CHART.replace(
        r#"  { label: "Q5", value: 411 },
  { label: "Q6", value: 523 },
"#,
        "",
    ))
    .expect("a shorter dataset compiles unchanged");
    let bars = |artifact: &valle_motion::SceneArtifact| {
        artifact
            .nodes
            .iter()
            .filter(|node| node.key.starts_with("bar-"))
            .count()
    };
    assert_eq!(bars(&six.artifact), 6);
    assert_eq!(
        bars(&four.artifact),
        4,
        "removing two data points must remove two bars — nothing else changed"
    );
}

// Resolution independence.

/// Doubling the viewport doubles boxes within a two-pixel rounding tolerance.
#[test]
fn every_film_lays_out_proportionally_at_two_resolutions() {
    for (name, source) in FILMS {
        let small = boxes_at(source, 1920, 1080);
        let large = boxes_at(source, 3840, 2160);
        assert_eq!(
            small.len(),
            large.len(),
            "{name}: the two resolutions must produce the same topology"
        );
        assert!(small.len() > 5, "{name}: suspiciously few nodes");
        for (key, a) in &small {
            let b = large
                .get(key)
                .unwrap_or_else(|| panic!("{name}: {key} missing at 4K"));
            for axis in 0..4 {
                assert!(
                    (b[axis] - a[axis] * 2.0).abs() <= 2.0,
                    "{name}: {key}[{axis}] must double with the viewport — {} vs {}",
                    a[axis],
                    b[axis]
                );
            }
        }
    }
}

// Public API dependencies.

/// Keep an explicit public API allowlist so fixture-only builtins cannot silently become dependencies.
#[test]
fn the_reference_films_only_use_the_public_surface() {
    // Public compute functions.
    const COMPUTE: &[&str] = &[
        "ticks",
        "niceDomain",
        "extent",
        "extentOrDefault",
        "scaleLinear",
        "scaleBand",
        "scalePoint",
        "geoProject",
        "graphLayout",
        "seededRandom",
        "noise1d",
        "noise2d",
        // Public scale methods.
        "map",
        "invert",
        "band",
        "center",
        "bandwidth",
        "step",
        "at",
    ];
    // Language and visual primitives.
    const PRIMITIVES: &[&str] = &[
        "interpolate",
        "clamp",
        "point",
        "rect",
        "line",
        "cubic",
        "arc",
        "area",
        "path",
        "offsetPath",
        "motionPath",
        "spring",
        "measureText",
        "bounds",
        "anchor",
        "connect",
        "pointAt",
        "tangentAt",
        "pathLength",
        "pathTrajectory",
        "morphPath",
        "boolean",
        "linearGradient",
        "radialGradient",
        "gradientStop",
        "defineControls",
        "frames",
        "optionalFrames",
        "number",
        "string",
        "boolean",
        "color",
        "length",
        "angle",
        "select",
        "nodeTarget",
        "asset",
        "spanCue",
    ];

    for (name, source) in FILMS {
        // Collect identifier calls, excluding language builtins and local helpers.
        let mut called: Vec<String> = Vec::new();
        let bytes = source.as_bytes();
        let mut start = None;
        for (index, byte) in bytes.iter().enumerate() {
            let is_word = byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$';
            match (start, is_word) {
                (None, true) => start = Some(index),
                (Some(from), false) => {
                    // Function declarations are not external calls.
                    let defined = source[..from].trim_end().ends_with("function");
                    if *byte == b'(' && !defined {
                        called.push(source[from..index].to_string());
                    }
                    start = None;
                }
                _ => {}
            }
        }
        // Exclude language builtins and locally defined names.
        let local: Vec<&str> = source
            .match_indices("const ")
            .filter_map(|(at, _)| {
                let rest = &source[at + 6..];
                let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
                Some(&rest[..end])
            })
            .collect();
        const JS: &[&str] = &[
            "map",
            "filter",
            "reduce",
            "slice",
            "concat",
            "join",
            "push",
            "function",
            "if",
            "for",
            "while",
            "return",
            "switch",
            "catch",
            "typeof",
            "Math",
            "Number",
            "String",
            "Array",
            "Object",
            "JSON",
            "parseFloat",
            "parseInt",
            "isFinite",
            "isNaN",
        ];
        for callee in called {
            let known = COMPUTE.contains(&callee.as_str())
                || PRIMITIVES.contains(&callee.as_str())
                || JS.contains(&callee.as_str())
                || local.contains(&callee.as_str())
                // Parenthesized helper arguments are not calls.
                || callee.len() <= 2;
            assert!(
                known,
                "{name} calls `{callee}`, which is neither a public compute function nor a \
                 visual primitive nor defined in the film itself — that breaks detachability"
            );
        }
    }
}

/// Reject domain-specific tags such as BarChart.
#[test]
fn no_domain_specific_jsx_tag_exists() {
    for (name, _) in FILMS {
        let source = FILMS.iter().find(|(n, _)| n == name).unwrap().1;
        for forbidden in ["<BarChart", "<WorldMap", "<Chart", "<Diagram", "<Map "] {
            assert!(
                !source.contains(forbidden),
                "{name} uses `{forbidden}` — a domain-specific tag cannot be deleted, so \
                 detachability would be untestable"
            );
        }
    }
}

// Computed geometry.

/// Verify evenly spaced bars and data-driven heights in the final layout.
#[test]
fn geometry_comes_from_the_compute_functions_not_from_magic_numbers() {
    let boxes = boxes_at(BAR_CHART, 1920, 1080);
    let lefts: Vec<f32> = (0..6).map(|i| boxes[&format!("bar-{i}")][0]).collect();
    for pair in lefts.windows(2) {
        assert!(
            pair[1] > pair[0],
            "bars must march left to right: {lefts:?}"
        );
    }
    let steps: Vec<f32> = lefts.windows(2).map(|w| w[1] - w[0]).collect();
    for step in &steps {
        assert!(
            (step - steps[0]).abs() <= 1.0,
            "band steps must be uniform — that is the scale doing the work: {steps:?}"
        );
    }
    // The largest value produces the tallest bar.
    let heights: Vec<f32> = (0..6).map(|i| boxes[&format!("bar-{i}")][3]).collect();
    let tallest = heights
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i);
    assert_eq!(
        tallest,
        Some(5),
        "the largest datum (523, index 5) must be the tallest bar: {heights:?}"
    );
}
