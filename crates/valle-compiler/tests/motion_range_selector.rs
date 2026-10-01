#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, motion_context_at_frame, prepare_scene,
    register_default_motion_fonts, resolve_props,
};
use valle_timeline::FrameRate;

const RANGE_SELECTOR: &str =
    include_str!("fixtures/motion/composition/text-range-selector.motion.tsx");

fn units(source: &str, frame: u32) -> Vec<valle_motion::layout::ResolvedUnit> {
    let artifact = compile_motion(source).unwrap().artifact;
    artifact.validate().unwrap();
    let scene = prepare_scene(&artifact).unwrap();
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let tree = build_tree(
        &scene,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    tree.units["t"].clone()
}

#[test]
fn selects_individual_letters_and_blurs_only_the_front() {
    assert!(
        units(RANGE_SELECTOR, 0)
            .iter()
            .all(|unit| unit.opacity == Some(0.0))
    );
    let sample = units(RANGE_SELECTOR, 30);
    assert_eq!(sample.len(), 8);
    for unit in &sample[..4] {
        assert_eq!(unit.opacity, Some(1.0));
        assert_eq!(unit.blur, Some(0.0));
    }
    let front = &sample[4];
    assert!((0.2..0.8).contains(&front.opacity.unwrap()));
    assert!((2.0..6.0).contains(&front.blur.unwrap()));
    for unit in &sample[5..] {
        assert_eq!(unit.opacity, Some(0.0));
        assert_eq!(unit.blur, Some(8.0));
    }
    assert_eq!(sample, units(RANGE_SELECTOR, 30));
}

#[test]
fn selector_shapes_and_offset_have_distinct_unit_profiles() {
    let square = RANGE_SELECTOR.replace("shape: \"ramp\"", "shape: \"square\"");
    let smooth = RANGE_SELECTOR.replace("shape: \"ramp\"", "shape: \"smooth\"");
    let triangle = RANGE_SELECTOR.replace("shape: \"ramp\"", "shape: \"triangle\"");
    let shifted = RANGE_SELECTOR.replace("softness: 0.15", "offset: 0.25, softness: 0.15");
    let ramp_front = units(RANGE_SELECTOR, 31)[4].opacity.unwrap();
    assert_eq!(units(&square, 31)[4].opacity, Some(1.0));
    assert_ne!(units(&smooth, 31)[4].opacity, Some(ramp_front));
    assert_ne!(units(&triangle, 31)[4].opacity, Some(ramp_front));
    assert_eq!(units(&shifted, 30)[4].opacity, Some(1.0));
}

#[test]
fn selector_rejects_invalid_options_and_unit_use_outside_text() {
    for invalid in [
        RANGE_SELECTOR.replace("shape: \"ramp\"", "shape: \"other\""),
        RANGE_SELECTOR.replace("softness: 0.15", "softness: -1"),
        RANGE_SELECTOR.replace("start: 0", "start: 2"),
    ] {
        assert!(compile_motion(&invalid).is_err());
    }
    let outside = r#"export default function Bad() {
      return <View style={{ opacity: rangeSelector({start:0,end:1}) }} />;
    }"#;
    assert!(compile_motion(outside).is_err());
}
