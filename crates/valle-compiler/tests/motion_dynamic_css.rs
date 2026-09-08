#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, ResolvedSignals, Viewport, build_tree, default_font_naming, emit,
    motion_context_at, phase_windows, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn source(style: &str) -> String {
    format!(
        r##"export default function Demo(ctx) {{
      const t = ctx.localFrame / 30;
      return <Scene style={{{{ width: 320, height: 180 }}}}>
        <View style={{{{ width: 160, height: 100, {style} }}}} />
      </Scene>;
    }}"##
    )
}

fn display(source: &str, frame: u32) -> Vec<u8> {
    let artifact = compile_motion(source).expect("compile").artifact;
    let prepared = prepare_scene(&artifact).expect("prepare");
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = phase_windows(&artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(frame, &windows, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming)
        .unwrap()
        .program
        .packed_bytes()
        .unwrap()
}

#[test]
fn dynamic_gradient_geometry_and_finite_filter_branches_are_seek_pure() {
    let source = source(
        r##"
        backgroundImage: t < 0.5
            ? `linear-gradient(${30 + t * 90}deg in srgb, red ${t * 20}%, blue)`
            : `radial-gradient(circle at ${20 + t * 60}% 50% in srgb, red, blue)`,
        filter: t < 0.25 ? "none" : t < 0.5 ? `blur(${t * 4}px)` : `brightness(${0.5 + t}) contrast(1.2)`
    "##,
    );
    let reference = [0, 10, 20, 29].map(|frame| display(&source, frame));
    for (frame, index) in [(29, 3), (0, 0), (20, 2), (10, 1), (29, 3)] {
        assert_eq!(display(&source, frame), reference[index]);
    }
    assert_ne!(reference[0], reference[1]);
    assert_ne!(reference[1], reference[2]);
}

#[test]
fn every_gradient_branch_is_checked_and_css_structure_injection_stays_closed() {
    for style in [
        r##"backgroundImage: t < 0.5 ? `linear-gradient(${t}deg, red, blue)` : "url(https://example.com/image.png)""##,
        r##"backgroundImage: `linear-gradient(${t}deg in oklab, red, blue)`"##,
        r##"backgroundImage: `linear-gradient(${t < 0.5 ? "in srgb" : "in oklab"}, red, blue)`"##,
        r##"filter: `${t < 0.5 ? "blur(2px)" : "brightness(2)"}`"##,
    ] {
        let admitted = compile_motion(&source(style))
            .ok()
            .is_some_and(|compiled| prepare_scene(&compiled.artifact).is_ok());
        assert!(!admitted, "unexpectedly admitted {style}");
    }
}

#[test]
fn map_index_can_choose_a_finite_filter_structure() {
    let source = r##"export default function Demo(ctx) {
        return <Scene>{[0,1].map((i) => <View key={`card-${i}`} style={{
            width: 100, height: 100, backgroundColor: "red",
            filter: i === 0 ? `blur(${ctx.localFrame / 30}px)` : `contrast(${1 + ctx.localFrame / 30})`
        }} />)}</Scene>;
    }"##;
    assert_ne!(display(source, 0), display(source, 20));
}

#[test]
fn gradient_can_switch_to_none_without_enum_string_type_conflicts() {
    let source = source(
        r##"backgroundImage: t < 0.5 ? "none" : `linear-gradient(${t * 90}deg, red, blue)`"##,
    );
    assert_ne!(display(&source, 0), display(&source, 20));
}
