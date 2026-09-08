#![cfg(feature = "motion")]
//! Camera transforms preserve world-space layout, while screen-space HUD elements remain fixed.

#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    CAMERA_CAPABILITY, NumberValue, ResolvedSignals, motion_context_at, phase_windows,
    resolve_props,
};
use valle_motion::{
    FontResource, Fonts, LayoutOptions, Viewport, build_tree, layout_boxes, prepare_scene,
};
use valle_timeline::FrameRate;

const FONT: &[u8] =
    include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(FONT.to_vec()))
        .expect("register font");
    fonts
}

/// A world-space box and screen-space HUD with an animated camera pan and zoom.
const SOURCE: &str = r##"
export default function Probe(ctx) {
  const t = ctx.hold.progress;
  const cx = interpolate(t, [0, 1], [260, 660], { easing: "linear" });
  const zoom = interpolate(t, [0, 1], [1, 2], { easing: "linear" });
  return (
    <Scene className="h-full w-full" camera={{ center: point(cx, 200), zoom, rotation: 0 }}>
      <World key="world">
        <View key="block" style={{ position: "absolute", left: 200, top: 140, width: 120, height: 120 }} />
      </World>
      <Screen key="hud">
        <View key="hud/chip" style={{ position: "absolute", left: 32, top: 32, width: 200, height: 48 }} />
      </Screen>
    </Scene>
  );
}
"##;

fn boxes_at(source: &str, frame: u32) -> std::collections::BTreeMap<String, [f32; 4]> {
    let compiled = compile_motion(source).expect("probe compiles");
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).expect("props");
    let phases = phase_windows(&compiled.artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(frame, &phases, FrameRate::new(30, 1).unwrap()).expect("frame");
    let fonts = fonts();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((960, 540)),
            fonts: &fonts,
            styles: None,
        },
    )
    .expect("layout");
    layout_boxes(&compiled.artifact, &tree).expect("boxes")
}

#[test]
fn camera_lowers_into_the_artifact_and_declares_its_capability() {
    let compiled = compile_motion(SOURCE).expect("camera scene compiles");
    let camera = compiled
        .artifact
        .camera
        .as_ref()
        .expect("camera is recorded");
    assert!(
        matches!(camera.rotation, NumberValue::Static { value } if value == 0.0),
        "static rotation folds to a literal"
    );
    assert!(
        compiled
            .artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == CAMERA_CAPABILITY),
        "a consumer that ignores the camera would render the whole scene in the wrong framing, \
         so the capability must be declared: {:?}",
        compiled.artifact.capability_set.names
    );
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn changing_the_camera_does_not_move_a_single_layout_box() {
    // Camera transforms affect drawing only; all layout boxes must remain bit-identical across frames.
    let start = boxes_at(SOURCE, 0);
    let mid = boxes_at(SOURCE, 15);
    assert_eq!(
        start, mid,
        "camera-only change must not trigger any relayout"
    );
    // Verify the boxes use the source's world coordinates.
    let block = start["block"];
    assert_eq!((block[0], block[1]), (200.0, 140.0));
}

#[test]
fn a_world_subtree_cannot_live_inside_screen() {
    let source = SOURCE.replace(
        r#"<View key="hud/chip" style={{ position: "absolute", left: 32, top: 32, width: 200, height: 48 }} />"#,
        r#"<World key="hud/world"><View key="hud/chip" style={{ width: 10, height: 10 }} /></World>"#,
    );
    let diagnostics = compile_motion(&source).expect_err("World inside Screen must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot live inside <Screen>")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn screen_must_be_a_direct_child_of_scene_and_come_last() {
    // Placement constraints keep camera wrappers and artifact/layout trees aligned.
    let nested = SOURCE
        .replace(
            r#"      <Screen key="hud">"#,
            r#"      <View key="wrap"><Screen key="hud">"#,
        )
        .replace(r#"      </Screen>"#, r#"      </Screen></View>"#);
    let diagnostics = compile_motion(&nested).expect_err("nested Screen must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("direct child of <Scene>")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );

    let reordered = r##"
export default function Probe(ctx) {
  return (
    <Scene className="h-full w-full" camera={{ center: point(0, 0), zoom: 1 }}>
      <Screen key="hud"><View key="hud/chip" style={{ width: 10, height: 10 }} /></Screen>
      <World key="world"><View key="block" style={{ width: 10, height: 10 }} /></World>
    </Scene>
  );
}
"##;
    let diagnostics = compile_motion(reordered).expect_err("Screen before World must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("after all")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_non_positive_zoom_fails_closed() {
    // Cross zero explicitly: the half-open progress interval never reaches the endpoint of [1, 0].
    let source = SOURCE.replace("[1, 2]", "[1, -1]");
    let compiled = compile_motion(&source).expect("compiles; zoom is a frame expression");
    let prepared = prepare_scene(&compiled.artifact).expect("prepare");
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).expect("props");
    let phases = phase_windows(&compiled.artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(29, &phases, FrameRate::new(30, 1).unwrap()).expect("last frame");
    let fonts = fonts();
    let result = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((960, 540)),
            fonts: &fonts,
            styles: None,
        },
    );
    let message = match result {
        Ok(_) => "a layout tree (a world collapsed to a point!)".to_owned(),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains("zoom must be finite and positive"),
        "got: {message}"
    );
}

#[test]
fn a_static_non_positive_zoom_is_refused_at_admission() {
    let source = r##"
export default function Probe(ctx) {
  return (
    <Scene className="h-full w-full" camera={{ center: point(0, 0), zoom: 0 }}>
      <World key="world"><View key="block" style={{ width: 10, height: 10 }} /></World>
    </Scene>
  );
}
"##;
    let diagnostics = compile_motion(source).expect_err("static zoom 0 must be refused");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("zoom must be finite and positive")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn camera_is_only_allowed_on_the_scene_root() {
    let source = SOURCE.replace(
        r#"<World key="world">"#,
        r#"<World key="world" camera={{ center: point(0, 0), zoom: 1 }}>"#,
    );
    let diagnostics = compile_motion(&source).expect_err("camera on a non-Scene node");
    assert!(
        !diagnostics.is_empty(),
        "camera must not be silently accepted on an inner node"
    );
}
