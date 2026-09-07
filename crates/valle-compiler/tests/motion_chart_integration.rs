#![cfg(feature = "motion")]
//! Chart integration tests.

use std::collections::BTreeMap;

use valle_compiler::motion::{MeasureEnv, compile_motion_with_env};
use valle_motion::{
    BatchPositions, CAMERA_CAPABILITY, ContentDigest, FONT_ASSET_CAPABILITY,
    GEOMETRY_BATCH_CAPABILITY, NODE_ADVANCED_FILTER_CAPABILITY, NodeKind, ResolvedSignals,
    ResourceRef, motion_context_at, phase_windows, resolve_props,
};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, prepare_scene,
};
use valle_timeline::FrameRate;

const SOURCE: &str = include_str!("fixtures/motion/charts/chart-integration.motion.tsx");
const FONT: &[u8] =
    include_bytes!("../../valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn compiled() -> valle_compiler::motion::CompiledMotion {
    let hash = ContentDigest::of_bytes(FONT);
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (1280, 720),
    )
    .expect("font measure environment");
    compile_motion_with_env(
        SOURCE,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: hash,
        }],
        Some(&measure),
    )
    .expect("integration source compiles")
}

#[test]
fn chart_integration_combines_font_camera_spaces_batch_and_advanced_filter() {
    let compiled = compiled();
    compiled
        .artifact
        .validate()
        .expect("valid integration artifact");
    for capability in [
        FONT_ASSET_CAPABILITY,
        CAMERA_CAPABILITY,
        GEOMETRY_BATCH_CAPABILITY,
        NODE_ADVANCED_FILTER_CAPABILITY,
    ] {
        assert!(
            compiled
                .artifact
                .capability_set
                .names
                .iter()
                .any(|name| name == capability),
            "missing {capability}: {:?}",
            compiled.artifact.capability_set.names
        );
    }
    assert!(compiled.artifact.nodes.iter().any(|node| {
        matches!(&node.kind, NodeKind::GeometryBatch { batch }
            if matches!(&batch.positions, BatchPositions::Static { values } if values.len() == 1200))
    }));
    for key in ["primary/card", "secondary/card", "panel", "dense-markers"] {
        assert!(
            compiled.artifact.nodes.iter().any(|node| node.key == key),
            "missing {key}"
        );
        assert!(
            compiled
                .source_map
                .nodes
                .iter()
                .any(|mapping| mapping.key == key)
        );
    }
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
fn integration_is_random_seek_pure_and_multi_instance_state_does_not_leak() {
    let compiled = compiled();
    let later = display_at(&compiled, 87);
    let _earlier = display_at(&compiled, 11);
    let later_again = display_at(&compiled, 87);
    assert_eq!(later, later_again);
    assert_ne!(display_at(&compiled, 12), display_at(&compiled, 88));
}
