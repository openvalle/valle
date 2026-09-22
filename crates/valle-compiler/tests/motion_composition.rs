#![cfg(feature = "motion")]

use std::collections::BTreeMap;

use valle_compiler::motion::{MotionModuleGraph, compile_motion, compile_motion_modules};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, motion_context_at_frame, prepare_scene,
    resolve_props,
};
use valle_timeline::FrameRate;

const BODY: &str = "export default function Demo() { return <View key=\"box\" style={{ width: \"2rem\", height: 10 }} />; }";

fn compile_entry(declaration: &str) -> Result<valle_motion::SceneArtifact, String> {
    let source = format!("{declaration}\n{BODY}");
    let graph = MotionModuleGraph::new(
        "demo.motion.tsx",
        BTreeMap::from([("demo.motion.tsx".to_string(), source)]),
    )
    .unwrap();
    compile_motion_modules(&graph)
        .map(|compiled| compiled.artifact)
        .map_err(|errors| format!("{errors:#?}"))
}

#[test]
fn reusable_entry_has_no_implicit_work_metadata() {
    let artifact = compile_entry("").expect("component compiles without a work definition");
    assert!(artifact.composition.is_none());
    assert!(compile_motion(BODY).unwrap().artifact.composition.is_none());
}

#[test]
fn duration_and_optional_fps_are_recorded_exactly() {
    let artifact =
        compile_entry("export const composition = { width: 1080, height: 1080, duration: 4.6 };")
            .unwrap();
    let composition = artifact.composition.as_ref().unwrap();
    assert_eq!(composition.viewport().tuple(), (1080, 1080));
    assert_eq!(composition.duration, "23/5");
    assert_eq!(composition.frame_rate().unwrap(), None);
    assert_eq!(
        composition
            .duration_frames(FrameRate::new(24, 1).unwrap())
            .unwrap(),
        110
    );
    assert_eq!(
        composition
            .duration_frames(FrameRate::new(25, 1).unwrap())
            .unwrap(),
        115
    );
    let reopened: valle_motion::SceneArtifact =
        serde_json::from_str(&serde_json::to_string(&artifact).unwrap()).unwrap();
    assert_eq!(reopened.composition, artifact.composition);

    for (declared, numerator, denominator) in [
        ("30", 30, 1),
        ("29.97", 2997, 100),
        ("\"30000/1001\"", 30_000, 1_001),
    ] {
        let artifact = compile_entry(&format!("export const composition = {{ width: 640, height: 360, duration: 1, fps: {declared} }};")).unwrap();
        let fps = artifact.composition.unwrap().frame_rate().unwrap().unwrap();
        assert_eq!(
            (fps.numerator(), fps.denominator()),
            (numerator, denominator)
        );
    }
}

#[test]
fn static_arithmetic_is_accepted_and_removed_fields_are_rejected() {
    let artifact = compile_entry("const W = 800; export const composition = { width: W, height: W / 2, fps: 30000 / 1001, duration: 3 + 0.5 };").unwrap();
    assert_eq!(artifact.composition.unwrap().duration, "7/2");
    for field in ["durationInFrames: 30", "rootFontSize: 24"] {
        let source = format!(
            "export const composition = {{ width: 320, height: 180, duration: 1, {field} }};"
        );
        assert!(compile_entry(&source).unwrap_err().contains("no field"));
    }
    assert!(
        compile_entry("export const composition = { width: 320, height: 180 };")
            .unwrap_err()
            .contains("needs `duration`")
    );
    assert!(
        compile_entry("export const composition = { width: 320, height: 180, duration: 0 };")
            .unwrap_err()
            .contains("duration must be positive")
    );
}

#[test]
fn rem_is_fixed_to_sixteen_source_pixels() {
    let artifact =
        compile_entry("export const composition = { width: 320, height: 180, duration: 1 };")
            .unwrap();
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let ctx = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((320, 180)).with_font_size(24.0),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let boxes = valle_motion::layout::layout_boxes(&artifact, &tree).unwrap();
    assert!((boxes["box"][2] - 32.0).abs() < 0.01);
    let program = valle_motion::emit(&tree, &valle_motion::default_font_naming)
        .unwrap()
        .program;
    assert_eq!(
        program.viewport(),
        valle_draw::Rect::new(0.0, 0.0, 320.0, 180.0)
    );
}
