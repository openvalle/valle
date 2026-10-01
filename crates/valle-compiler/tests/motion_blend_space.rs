#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{BlendMode, BlendSpace, Node};
use valle_motion::{
    Fonts, LayoutOptions, MotionValue, StyleValue, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

const LINEAR_LIGHT_BLEND: &str =
    include_str!("fixtures/motion/composition/linear-light-blend.motion.tsx");

#[test]
fn blend_space_survives_lowering_and_geometry_reuse_without_becoming_inherited() {
    for (source, expected) in [
        (LINEAR_LIGHT_BLEND.to_owned(), [BlendSpace::Linear; 4]),
        (
            LINEAR_LIGHT_BLEND.replace(", mixBlendSpace: \"linear\"", ""),
            [BlendSpace::Srgb; 4],
        ),
        (
            LINEAR_LIGHT_BLEND.replace(
                "mixBlendSpace: \"linear\"",
                "mixBlendSpace: ctx.seconds < 0.5 ? \"linear\" : \"srgb\"",
            ),
            [
                BlendSpace::Srgb,
                BlendSpace::Linear,
                BlendSpace::Srgb,
                BlendSpace::Srgb,
            ],
        ),
        (
            LINEAR_LIGHT_BLEND
                .replace(
                    "backgroundColor: \"#101010\"",
                    "backgroundColor: \"#101010\", mixBlendSpace: \"linear\"",
                )
                .replace(", mixBlendSpace: \"linear\" }} />", " }} />"),
            [BlendSpace::Srgb; 4],
        ),
    ] {
        let artifact = compile_motion(&source).unwrap().artifact;
        let scene = prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let mut first = None;
        for (frame, space) in [30, 0, 59, 30].into_iter().zip(expected) {
            let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
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
            let program = emit(&tree, &default_font_naming).unwrap().program;
            let blends: Vec<_> = program
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    Node::Group(group) if group.internal_blend == BlendMode::Screen => Some(group),
                    _ => None,
                })
                .collect();
            assert_eq!(blends.len(), 1);
            assert_eq!(blends[0].blend_space, space);
            let bytes = program.packed_bytes().unwrap();
            if frame == 30 {
                if let Some(first) = &first {
                    assert_eq!(first, &bytes);
                }
                first = Some(bytes);
            }
        }
    }
}

#[test]
fn invalid_spaces_are_rejected_before_frame_evaluation() {
    for value in [
        "\"unknown\"",
        "1",
        "true",
        "ctx.seconds < 0.5 ? \"srgb\" : \"unknown\"",
        "`${ctx.seconds}`",
    ] {
        let source = LINEAR_LIGHT_BLEND.replace(
            "mixBlendSpace: \"linear\"",
            &format!("mixBlendSpace: {value}"),
        );
        let error = compile_motion(&source).unwrap_err();
        assert!(
            error.iter().any(|diagnostic| {
                diagnostic.message.contains("mixBlendSpace")
                    || diagnostic
                        .style
                        .as_ref()
                        .is_some_and(|style| style.property == "mix-blend-space")
            }),
            "{value}: {error:?}"
        );
    }
    let mut artifact = compile_motion(LINEAR_LIGHT_BLEND).unwrap().artifact;
    let style = artifact
        .nodes
        .iter_mut()
        .flat_map(|node| &mut node.styles)
        .find(|style| style.property == "mix-blend-space")
        .unwrap();
    style.value = StyleValue::Static {
        value: MotionValue::Str("rec2020".into()),
    };
    assert!(artifact.validate().is_err());
}
