#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::layout::LayoutCache;
use valle_motion::{
    Fonts, LayoutOptions, MotionHostContext, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::internal::SampleTime;
use valle_timeline::{FrameRate, RationalTime};

const BODY: &str = r##"<View key="bar" className="absolute" style={{left:ctx.host.seconds*40,
    width:ctx.host.duration*20,height:20,backgroundColor:"white",opacity:ctx.host.progress}} />"##;

fn source(body: &str) -> String {
    format!("export default function Main(ctx) {{ return <Scene>{body}</Scene>; }}")
}

#[test]
fn temporal_samples_freeze_host_inputs_even_at_source_boundaries() {
    for wrapper in [
        "<Shutter key=\"sampling\" samples={4} angle={180}>BODY</Shutter>",
        "<Echo key=\"sampling\" count={3} interval={2} decay={0.5}>BODY</Echo>",
    ] {
        for frame in [0, 29] {
            let fps = FrameRate::new(30, 1).unwrap();
            let mut ctx = motion_context_at_frame(frame, 30, fps).unwrap();
            ctx.host = MotionHostContext::new(
                SampleTime::new(RationalTime::new(3, 2).unwrap()),
                RationalTime::new(2, 1).unwrap(),
                45,
                60,
            )
            .unwrap();
            let dynamic = source(&wrapper.replace("BODY", BODY));
            let frozen = dynamic
                .replace("ctx.host.seconds", "1.5")
                .replace("ctx.host.duration", "2")
                .replace("ctx.host.progress", "(45/59)");
            let render = |input: &str| {
                let artifact = compile_motion(input).unwrap().artifact;
                let scene = prepare_scene(&artifact).unwrap();
                let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
                let fonts = Fonts::default();
                let tree = build_tree(
                    &scene,
                    &ctx,
                    &props,
                    &LayoutOptions {
                        viewport: Viewport::new((160, 90)),
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
            };
            assert_eq!(
                render(&dynamic),
                render(&frozen),
                "{wrapper}, source frame {frame}"
            );
        }
    }
}

#[test]
fn held_source_recomputes_host_geometry_in_any_request_order() {
    for body in [
        BODY.to_owned(),
        BODY.replace(
            "left:ctx.host.seconds*40",
            "left:0,translate:point(ctx.host.seconds*40,0)",
        )
        .replace(
            "width:ctx.host.duration*20",
            "width:40,scale:point(ctx.host.duration/2,1)",
        ),
    ] {
        let artifact = compile_motion(&source(&body)).unwrap().artifact;
        let prepared = prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let cache = LayoutCache::new(&prepared, &fonts);
        assert_eq!(cache.is_some(), body.contains("translate:"));
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let viewport = Viewport::new((160, 90));
        let mut seen = std::collections::BTreeMap::new();
        for duration in [2, 3, 2] {
            for frame in [59, 0, 30, 59, 1, 0] {
                // The source is held on one frame while the host keeps advancing.
                let mut ctx =
                    motion_context_at_frame(29, 30, FrameRate::new(30, 1).unwrap()).unwrap();
                ctx.host = MotionHostContext::new(
                    SampleTime::new(RationalTime::new(frame, 30).unwrap()),
                    RationalTime::new(duration, 1).unwrap(),
                    frame as u32,
                    duration as u32 * 30,
                )
                .unwrap();
                let cold = build_tree(
                    &prepared,
                    &ctx,
                    &props,
                    &LayoutOptions {
                        viewport,
                        fonts: &fonts,
                        styles: None,
                    },
                )
                .unwrap();
                if let Some(cache) = &cache {
                    let warm = cache
                        .build_tree_profiled(&ctx, &props, viewport, None)
                        .unwrap()
                        .0;
                    assert_eq!(
                        emit(&cold, &default_font_naming)
                            .unwrap()
                            .program
                            .packed_bytes()
                            .unwrap(),
                        emit(&warm, &default_font_naming)
                            .unwrap()
                            .program
                            .packed_bytes()
                            .unwrap()
                    );
                }
                let bytes = |tree: &valle_motion::LayoutTree| {
                    emit(tree, &default_font_naming)
                        .unwrap()
                        .program
                        .packed_bytes()
                        .unwrap()
                };
                let result = bytes(&cold);
                if let Some(previous) = seen.insert((duration, frame), result.clone()) {
                    assert_eq!(previous, result);
                }
            }
        }
        assert_ne!(seen[&(2, 0)], seen[&(2, 59)]);
        assert_ne!(seen[&(2, 30)], seen[&(3, 30)]);
    }
}
