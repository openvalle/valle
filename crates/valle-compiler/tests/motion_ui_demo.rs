#![cfg(feature = "motion")]
//! UI source and declared-state tests.

use std::collections::BTreeMap;

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{
    ARTIFACT_FORMAT_VERSION, BatchPositions, CAMERA_CAPABILITY, ContentDigest,
    FONT_ASSET_CAPABILITY, GEOMETRY_BATCH_CAPABILITY, NodeKind, ResolvedSignals, ResourceRef,
    motion_context_at, phase_windows, resolve_props,
};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, prepare_scene,
};
use valle_timeline::FrameRate;

const SOURCE: &str = include_str!("fixtures/motion/ui/publish-flow.motion.tsx");
const FONT: &[u8] =
    include_bytes!("../../valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn compile(source: &str) -> valle_compiler::motion::CompiledMotion {
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (1280, 720),
    )
    .expect("measure");
    compile_motion_with_env(
        source,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: ContentDigest::of_bytes(FONT),
        }],
        Some(&measure),
    )
    .expect("UI Demo compiles")
}

#[test]
fn ui_demo_is_ordinary_fixed_topology_on_one_artifact_format() {
    let compiled = compile(SOURCE);
    compiled.artifact.validate().expect("valid artifact");
    assert_eq!(compiled.artifact.format_version, ARTIFACT_FORMAT_VERSION);
    assert_eq!(ARTIFACT_FORMAT_VERSION, 1);
    assert!(compiled.artifact.nodes.iter().all(|node| {
        matches!(
            node.kind,
            NodeKind::Group
                | NodeKind::Box
                | NodeKind::Text { .. }
                | NodeKind::GeometryBatch { .. }
        )
    }));
    let wire = serde_json::to_string(&compiled.artifact).expect("wire");
    for forbidden in [
        "CursorNode",
        "StatePanelNode",
        "EventLoop",
        "moveTitle",
        "focusTitle",
    ] {
        assert!(
            !wire.contains(forbidden),
            "{forbidden} leaked into Artifact"
        );
    }
}

#[test]
fn state_rows_controls_cursor_focus_and_tap_are_stably_addressed() {
    let compiled = compile(SOURCE);
    for key in [
        "app/window",
        "app/field",
        "app/typed-clip",
        "app/publish",
        "states/panel",
        "states/state-IDLE",
        "states/state-DONE",
        "focus/ring",
        "tap/ring",
        "cursor/pointer",
        "cursor/tail",
    ] {
        assert!(compiled.artifact.nodes.iter().any(|node| node.key == key));
        assert!(
            compiled
                .source_map
                .nodes
                .iter()
                .any(|mapping| mapping.key == key),
            "missing source map for {key}"
        );
    }
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
        SOURCE.contains("padNumber"),
        "prepared state indices should still use the deterministic author helper"
    );
}

#[test]
fn component_names_compile_away_and_ai_edits_keep_state_topology() {
    let renamed = SOURCE
        .replace("function Cursor(", "function PointerRecipe(")
        .replace("<Cursor key=", "<PointerRecipe key=")
        .replace("function Tap(", "function RippleRecipe(")
        .replace("<Tap key=", "<RippleRecipe key=")
        .replace("function Focus(", "function FocusRecipe(")
        .replace("<Focus key=", "<FocusRecipe key=")
        .replace("function StatePanel(", "function PlanPanel(")
        .replace("<StatePanel key=", "<PlanPanel key=")
        .replace("function AppShell(", "function ProductShell(")
        .replace("<AppShell key=", "<ProductShell key=");
    let original = compile(SOURCE);
    let renamed = compile(&renamed);
    assert_eq!(
        valle_motion::canonical_bytes(&original.artifact).unwrap(),
        valle_motion::canonical_bytes(&renamed.artifact).unwrap()
    );

    let edited = SOURCE
        .replace("default: \"#7c3aed\"", "default: \"#2563eb\"")
        .replace("Launch film / 4K master", "Review film / 4K master")
        .replace("duration: seconds(1.05)", "duration: seconds(1.18)");
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
fn events_hooks_and_side_effects_remain_fail_closed() {
    let event = include_str!("fixtures/motion/invalid-inputs/ui-demo.negative.motion.tsx");
    let diagnostics = compile_motion(event).expect_err("event callback must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("event handler") && diagnostic.message.contains("side effect")
    }));

    for source in [
        r#"export default function P(){const [x,setX]=useState(0);return <View style={{left:x}}/>;}"#,
        r#"export default function P(){useEffect(()=>publish(),[]);return <View/>;}"#,
    ] {
        compile_motion(source).expect_err("mutable hooks do not belong to a UI walkthrough");
    }
}

fn display_at(compiled: &valle_compiler::motion::CompiledMotion, frame: u32) -> Vec<u8> {
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &BTreeMap::new()).expect("props");
    let windows = phase_windows(&compiled.artifact.controls.phase_spec(), 360);
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
fn declared_ui_flow_is_random_seek_pure() {
    let compiled = compile(SOURCE);
    let later = display_at(&compiled, 252);
    let _earlier = display_at(&compiled, 24);
    assert_eq!(later, display_at(&compiled, 252));
    assert_ne!(display_at(&compiled, 48), display_at(&compiled, 205));
}
