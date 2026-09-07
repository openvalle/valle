#![cfg(feature = "motion")]
//! Map source and integration tests.

use std::collections::BTreeMap;

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{
    ARTIFACT_FORMAT_VERSION, BatchPositions, CAMERA_CAPABILITY, ContentDigest,
    FONT_ASSET_CAPABILITY, GEOMETRY_BATCH_CAPABILITY, NodeKind, PathValue, ResolvedSignals,
    ResourceRef, motion_context_at, phase_windows, resolve_props,
};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, prepare_scene,
};
use valle_timeline::FrameRate;

const SOURCE: &str = include_str!("fixtures/motion/maps/map-atlas.motion.tsx");
const FONT: &[u8] =
    include_bytes!("../../valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn compile(source: &str) -> valle_compiler::motion::CompiledMotion {
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (1280, 720),
    )
    .expect("font measure environment");
    compile_motion_with_env(
        source,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: ContentDigest::of_bytes(FONT),
        }],
        Some(&measure),
    )
    .expect("Map source compiles")
}

#[test]
fn map_package_compiles_to_ordinary_static_geometry_on_one_artifact_format() {
    let compiled = compile(SOURCE);
    compiled.artifact.validate().expect("valid Map artifact");
    assert_eq!(compiled.artifact.format_version, ARTIFACT_FORMAT_VERSION);
    assert_eq!(ARTIFACT_FORMAT_VERSION, 1);
    assert!(compiled.artifact.nodes.iter().all(|node| {
        matches!(
            node.kind,
            NodeKind::Group
                | NodeKind::Box
                | NodeKind::Text { .. }
                | NodeKind::Path { .. }
                | NodeKind::GeometryBatch { .. }
        )
    }));
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Path {
                    d: PathValue::Static { .. },
                    ..
                } => Some(()),
                _ => None,
            })
            .count()
            >= 15,
        "projected regions and prepared flows remain ordinary Path nodes"
    );

    let wire = serde_json::to_string(&compiled.artifact).expect("wire");
    for forbidden in ["MapNode", "RegionNode", "FlowNode"] {
        assert!(
            !wire.contains(forbidden),
            "{forbidden} leaked into the Artifact"
        );
    }
}

#[test]
fn hole_antimeridian_labels_and_two_themes_reach_the_source_package() {
    let compiled = compile(SOURCE);
    let region_paths = compiled
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Path {
                d: PathValue::Static { value },
                ..
            } if node.key.contains("region-") && node.key.ends_with("/shape") => {
                Some((node.key.as_str(), value))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        region_paths.len(),
        12,
        "six stable regions in two map instances"
    );
    assert!(
        region_paths.iter().any(|(key, path)| key.contains("aurora")
            && path
                .verbs
                .iter()
                .filter(|verb| format!("{verb:?}") == "Move")
                .count()
                == 2),
        "the Aurora outer ring and hole must survive as two subpaths"
    );
    for id in [
        "aurora", "meridian", "solace", "delta", "pelagic", "dateline",
    ] {
        assert!(
            region_paths.iter().any(|(key, _)| key.contains(id)),
            "missing {id}"
        );
    }
    for key_fragment in [
        "main-map",
        "mini-map",
        "label-aurora",
        "flow-0",
        "marker-aurora",
    ] {
        assert!(
            compiled
                .artifact
                .nodes
                .iter()
                .any(|node| node.key.contains(key_fragment)),
            "missing {key_fragment}"
        );
        assert!(
            compiled
                .source_map
                .nodes
                .iter()
                .any(|mapping| mapping.key.contains(key_fragment)),
            "missing source map for {key_fragment}"
        );
    }
}

#[test]
fn map_theme_is_explicit_and_integration_capabilities_are_real_consumers() {
    let compiled = compile(SOURCE);
    for name in ["ocean", "land", "accent", "signal", "label"] {
        assert!(matches!(
            compiled
                .artifact
                .controls
                .props
                .get(name)
                .map(|value| &value.control),
            Some(valle_motion::ControlType::Color)
        ));
    }
    assert!(!SOURCE.contains("ThemeProvider"));
    assert!(!SOURCE.contains("useTheme"));
    for capability in [
        FONT_ASSET_CAPABILITY,
        CAMERA_CAPABILITY,
        GEOMETRY_BATCH_CAPABILITY,
    ] {
        assert!(
            compiled
                .artifact
                .capability_set
                .names
                .iter()
                .any(|name| name == capability),
            "missing {capability}"
        );
    }
    assert!(compiled.artifact.nodes.iter().any(|node| {
        matches!(&node.kind, NodeKind::GeometryBatch { batch }
            if matches!(&batch.positions, BatchPositions::Static { values } if values.len() == 1200))
    }));
    assert!(
        serde_json::to_string(&compiled.artifact)
            .expect("wire")
            .contains("drop-shadow"),
        "integration fixture must keep a real filter consumer"
    );
}

#[test]
fn component_names_compile_away_and_ai_edits_keep_the_frozen_topology() {
    let renamed = SOURCE
        .replace("function Region(", "function LandPart(")
        .replace("<Region key=", "<LandPart key=")
        .replace("function Marker(", "function PulsePart(")
        .replace("<Marker key=", "<PulsePart key=")
        .replace("function Flow(", "function RoutePart(")
        .replace("<Flow key=", "<RoutePart key=")
        .replace("function MapLabel(", "function LabelPart(")
        .replace("<MapLabel key=", "<LabelPart key=")
        .replace("function WorldMap(", "function AtlasShell(")
        .replace("<WorldMap key=", "<AtlasShell key=");
    let original = compile(SOURCE);
    let renamed = compile(&renamed);
    assert_eq!(
        valle_motion::canonical_bytes(&original.artifact).unwrap(),
        valle_motion::canonical_bytes(&renamed.artifact).unwrap(),
        "component vocabulary must not become Map IR"
    );

    let edited = SOURCE
        .replace("default: \"#22d3ee\"", "default: \"#34d399\"")
        .replace(
            "SYNTHETIC MOBILITY ATLAS / 01",
            "SYNTHETIC SIGNAL ATLAS / 02",
        )
        .replace(
            "const lift = 68 + index * 18",
            "const lift = 84 + index * 18",
        );
    let edited = compile(&edited);
    assert_eq!(original.artifact.nodes.len(), edited.artifact.nodes.len());
    assert_eq!(
        original
            .artifact
            .nodes
            .iter()
            .map(|node| &node.key)
            .collect::<Vec<_>>(),
        edited
            .artifact
            .nodes
            .iter()
            .map(|node| &node.key)
            .collect::<Vec<_>>()
    );
    assert_ne!(
        valle_motion::canonical_bytes(&original.artifact).unwrap(),
        valle_motion::canonical_bytes(&edited.artifact).unwrap()
    );
}

#[test]
fn runtime_projection_host_math_and_duplicate_region_ids_fail_closed() {
    let runtime = include_str!("fixtures/motion/invalid-inputs/map.negative.motion.tsx");
    let diagnostics =
        compile_motion(runtime).expect_err("host transcendental math stays forbidden");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Math.log")
                || diagnostic.message.contains("Math.tan"))
    );

    let frame_projection = r##"
export default function BadMap(ctx) {
  const projected = geoPath({polygons: [[[[0,0],[10,0],[0,10]]]], width: 100 + ctx.frame, height: 100});
  return <Path key="region" d={path(projected[0].d)} fill="#fff"/>;
}
"##;
    let diagnostics = compile_motion(frame_projection).expect_err("projection is prepare-only");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("prepare-time"))
    );

    let duplicate = SOURCE.replace("{ id: \"dateline\"", "{ id: \"aurora\"");
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (1280, 720),
    )
    .expect("font measure environment");
    let diagnostics = compile_motion_with_env(
        &duplicate,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: ContentDigest::of_bytes(FONT),
        }],
        Some(&measure),
    )
    .expect_err("duplicate region ids must fail at the existing stable-key gate");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate node key"))
    );
}

fn display_at(compiled: &valle_compiler::motion::CompiledMotion, frame: u32) -> Vec<u8> {
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &BTreeMap::new()).expect("props");
    let windows = phase_windows(&compiled.artifact.controls.phase_spec(), 120);
    let ctx = motion_context_at(frame, &windows, FrameRate::new(60, 1).unwrap()).expect("context");
    let mut fonts = Fonts::default();
    fonts
        .register(valle_motion::FontResource::new(FONT.to_vec()))
        .expect("font");
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((1280, 720)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    emit(&tree, &default_font_naming)
        .expect("emit")
        .program
        .packed_bytes()
        .expect("canonical DrawProgram")
}

#[test]
fn map_integration_is_random_seek_pure() {
    let compiled = compile(SOURCE);
    let later = display_at(&compiled, 91);
    let _earlier = display_at(&compiled, 7);
    assert_eq!(later, display_at(&compiled, 91));
    assert_ne!(display_at(&compiled, 14), display_at(&compiled, 96));
}
