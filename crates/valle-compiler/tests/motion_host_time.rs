#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::layout::LayoutCache;
use valle_motion::{
    Fonts, LayoutOptions, MotionContext, Viewport, build_tree, default_font_naming, emit,
    prepare_scene, resolve_props,
};
use valle_timeline::{ClipClock, ClipMap, FrameRate, RationalRate, RationalTime, SourceEnd};

const BODY: &str = r##"<View key="bar" className="absolute" style={{left:time.clipSeconds*40,
    width:comp.clipDuration*20,height:20,backgroundColor:"white",opacity:time.clipProgress}} />"##;

fn source(body: &str) -> String {
    format!(
        "export default function Main() {{ const time = useCurrentTime(); const comp = useComposition(); return <Scene style={{{{width:160,height:90}}}}>{body}</Scene>; }}"
    )
}

/// A clip at the start of the Timeline that plays `source` seconds once and holds the end.
fn clock(duration: i64, source: i64) -> ClipClock {
    ClipClock::new(
        FrameRate::new(30, 1).unwrap(),
        RationalTime::ZERO,
        RationalTime::new(duration, 1).unwrap(),
        ClipMap::Clip {
            trim: RationalTime::ZERO,
            rate: RationalRate::ONE,
            end: SourceEnd::Hold,
        },
        RationalTime::new(source, 1).unwrap(),
        false,
    )
    .unwrap()
}

fn render(input: &str, ctx: &MotionContext) -> Vec<u8> {
    let artifact = compile_motion(input).unwrap().artifact;
    let scene = prepare_scene(&artifact).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &scene,
        ctx,
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
}

#[test]
fn temporal_samples_read_the_clip_clock_at_their_own_time() {
    // A translation keeps the subframe positions that a layout would round.
    let body = r##"<View key="bar" className="absolute" style={{left:0,
    translate:point(time.clipSeconds*40,0),width:comp.clipDuration*20,height:20,
    backgroundColor:"white"}} />"##;
    for wrapper in [
        "<Shutter key=\"sampling\" samples={4} angle={180}>BODY</Shutter>",
        "<Echo key=\"sampling\" count={3} delay={2 / 30} decay={0.5}>BODY</Echo>",
    ] {
        let dynamic = source(&wrapper.replace("BODY", body));
        // Inside the clip of an untrimmed source, every sample's clip clock is its scene clock;
        // the clip's length stays the host's.
        let scene = dynamic
            .replace("time.clipSeconds", "time.seconds")
            .replace("comp.clipDuration", "2");
        let ctx = MotionContext::new(clock(2, 3), 30).unwrap();
        assert_eq!(render(&dynamic, &ctx), render(&scene, &ctx), "{wrapper}");
        // The clip's start is a cut: samples before it hold the scene clock at the start, and
        // the clip clock reads their own time below 0.
        let first = MotionContext::new(clock(2, 3), 0).unwrap();
        let clamped = dynamic.replace("time.clipSeconds", "Math.max(time.clipSeconds, 0)");
        assert_eq!(
            render(&clamped, &first),
            render(&scene, &first),
            "{wrapper}"
        );
        assert_ne!(
            render(&dynamic, &first),
            render(&scene, &first),
            "{wrapper}"
        );
        // Past the source's end the scene clock holds and the clip clock runs on.
        let held = MotionContext::new(clock(2, 1), 45).unwrap();
        assert_ne!(render(&dynamic, &held), render(&scene, &held), "{wrapper}");
    }
}

#[test]
fn held_source_recomputes_host_geometry_in_any_request_order() {
    for body in [
        BODY.to_owned(),
        BODY.replace(
            "left:time.clipSeconds*40",
            "left:0,translate:point(time.clipSeconds*40,0)",
        )
        .replace(
            "width:comp.clipDuration*20",
            "width:40,scale:point(comp.clipDuration/2,1)",
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
                // The source holds its end after a second while the host keeps advancing.
                let ctx = MotionContext::new(clock(duration, 1), frame).unwrap();
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
