#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Node};
use valle_motion::layout::build_tree_profiled;
use valle_motion::{
    ECHO_CAPABILITY, Fonts, LayoutOptions, NodeKind, Viewport, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::{FrameRate, RationalTime};

const ECHO_TRAILS: &str = include_str!("fixtures/motion/composition/echo-trails.motion.tsx");

fn tree(source: &str, frame: u32) -> valle_motion::LayoutTree {
    let artifact = compile_motion(source).unwrap().artifact;
    let has_echo = artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Echo { .. }));
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    if has_echo {
        assert!(timings.sample_static_computed > 0);
        assert!(timings.sample_static_reused > 0);
    }
    tree
}

fn program(source: &str, frame: u32) -> DrawProgram {
    emit(&tree(source, frame), &default_font_naming)
        .unwrap()
        .program
}

#[test]
fn echo_samples_past_frames_and_decays() {
    let artifact = compile_motion(ECHO_TRAILS).unwrap().artifact;
    let window = prepare_scene(&artifact)
        .unwrap()
        .dependencies()
        .temporal_window();
    assert_eq!(window.past_frames, RationalTime::new(12, 1).unwrap());
    assert_eq!(window.future_frames, RationalTime::ZERO);
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == ECHO_CAPABILITY)
    );
    let frame30 = tree(ECHO_TRAILS, 30);
    let times = frame30.echo_sample_times("echo").unwrap();
    assert_eq!(times.len(), 4);
    for (time, frame) in times.iter().zip([18, 21, 24, 27]) {
        assert_eq!(time.composition(), RationalTime::new(frame, 30).unwrap());
    }
    let p30 = emit(&frame30, &default_font_naming).unwrap().program;
    let opacities = p30
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Group(group) if group.opacity < 1.0 => Some(group.opacity),
            _ => None,
        })
        .collect::<Vec<_>>();
    for opacity in [0.0625, 0.125, 0.25, 0.5] {
        assert!(
            opacities
                .iter()
                .any(|actual| (actual - opacity).abs() < 1e-6)
        );
    }
    assert_eq!(
        p30.packed_bytes().unwrap(),
        program(ECHO_TRAILS, 30).packed_bytes().unwrap()
    );
    let frame50 = tree(ECHO_TRAILS, 50);
    let times = frame50.echo_sample_times("echo").unwrap();
    for (time, frame) in times.iter().zip([38, 41, 44, 47]) {
        assert_eq!(time.composition(), RationalTime::new(frame, 30).unwrap());
    }
}

#[test]
fn echo_admission_rejects_invalid_parameters_and_nested_sampling() {
    assert!(compile_motion(&ECHO_TRAILS.replace("count={4}", "count={0}")).is_err());
    assert!(compile_motion(&ECHO_TRAILS.replace("interval={3}", "interval={0}")).is_err());
    assert!(compile_motion(&ECHO_TRAILS.replace("decay={0.5}", "decay={1.5}")).is_err());
    assert!(
        compile_motion(&ECHO_TRAILS.replace(
            "<Echo key=\"echo\"",
            "<Echo key=\"echo\" className=\"relative\""
        ))
        .is_err()
    );
    let mut invalid = compile_motion(ECHO_TRAILS).unwrap().artifact;
    let echo = invalid
        .nodes
        .iter_mut()
        .find(|node| node.key == "echo")
        .unwrap();
    echo.kind = valle_motion::NodeKind::Echo {
        count: 4,
        interval_frames: 0,
        decay: 0.5,
    };
    assert!(invalid.validate().is_err());
    let nested = ECHO_TRAILS
        .replace(
            "<View key=\"dot\"",
            "<Shutter key=\"nested\" samples={8} angle={180}><View key=\"dot\"",
        )
        .replace("</Echo>", "</Shutter></Echo>");
    assert!(compile_motion(&nested).is_err());
}
