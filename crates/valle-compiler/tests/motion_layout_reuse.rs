#![cfg(feature = "motion")]
use std::{collections::BTreeMap, sync::Arc};
use valle_compiler::motion::compile_motion;
use valle_motion::layout::LayoutCache;
use valle_motion::{
    FontResource, Fonts, LayoutOptions, MotionValue, StyleCache, Viewport, default_font_naming,
    emit, prepare_scene,
};
use valle_timeline::FrameRate;

const SOURCE: &str = r##"
export const controls=({props:{alpha:number({default:1,min:0,max:1})}});
const items=defineRepeater({count:12,keyPrefix:"row"});
export default function Scene(ctx,props) {
 return <Scene className="w-full h-full flex flex-wrap" style={{padding:12,gap:6}}>
  {items.map(row=><View key={row.key} style={{width:140,height:54,position:"relative",backgroundColor:"#28445b",translate:point(ctx.seconds*6,row.index%3),opacity:props.alpha*interpolate(ctx.seconds,[0,1],[0,1]),scale:point(1+ctx.progress*.05,1),rotate:interpolate(ctx.progress,[0,1],["0deg","4deg"])}}>
    <Text style={{fontSize:16}}>One fixed paragraph.</Text>
    <Text style={{position:"absolute",left:4,top:30,fontSize:10}}>Stable geometry / current paint</Text>
  </View>)}
 </Scene>;
}
"##;
fn fonts(name: &str) -> Fonts {
    let mut f = Fonts::default();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts/noto")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    f.register(FontResource::new(bytes.as_slice())).unwrap();
    f
}
fn bytes(tree: &valle_motion::LayoutTree) -> Vec<u8> {
    emit(tree, &default_font_naming)
        .unwrap()
        .program
        .packed_bytes()
        .unwrap()
}

#[test]
fn cached_geometry_matches_cold_programs_for_reverse_seeks_fps_and_viewport_changes() {
    let a = compile_motion(SOURCE).unwrap().artifact;
    let p = prepare_scene(&a).unwrap();
    let fonts = fonts("NotoSans-Regular.ttf");
    let cache = LayoutCache::new(&p, &fonts).unwrap();
    let styles = StyleCache::new();
    let props = valle_motion::resolve_props(&a.controls, &BTreeMap::new()).unwrap();
    for rate in [
        FrameRate::new(24, 1).unwrap(),
        FrameRate::new(30, 1).unwrap(),
        FrameRate::new(60, 1).unwrap(),
        FrameRate::new(30000, 1001).unwrap(),
    ] {
        for viewport in [
            Viewport::new((960, 620)),
            Viewport::new((640, 480)),
            Viewport::new((960, 620))
                .with_font_size(20.0)
                .with_device_pixel_ratio(2.0),
        ] {
            cache.clear();
            for (i, frame) in [0, 60, 149, 1, 77, 60, 0].into_iter().enumerate() {
                let ctx = valle_motion::motion_context_at_frame(frame, 300, rate).unwrap();
                let plain = valle_motion::build_tree(
                    &p,
                    &ctx,
                    &props,
                    &LayoutOptions {
                        viewport,
                        fonts: &fonts,
                        styles: None,
                    },
                )
                .unwrap();
                let (reused, timing) = cache
                    .build_tree_profiled(&ctx, &props, viewport, Some(&styles))
                    .unwrap();
                assert_eq!(timing.layout_reused, i > 0);
                assert_eq!(bytes(&plain), bytes(&reused), "frame={frame} rate={rate:?}");
            }
        }
    }
}

#[test]
fn cache_replaces_geometry_on_configuration_change_and_does_not_retain_old_frames() {
    let a = compile_motion(SOURCE).unwrap().artifact;
    let p = prepare_scene(&a).unwrap();
    let fonts = fonts("NotoSans-Regular.ttf");
    let cache = LayoutCache::new(&p, &fonts).unwrap();
    let ctx =
        valle_motion::motion_context_at_frame(30, 300, FrameRate::new(30, 1).unwrap()).unwrap();
    let default = valle_motion::resolve_props(&a.controls, &BTreeMap::new()).unwrap();
    let override_props = valle_motion::resolve_props(
        &a.controls,
        &BTreeMap::from([("alpha".into(), MotionValue::Number(0.5))]),
    )
    .unwrap();
    let view = Viewport::new((960, 620));
    let (first, _) = cache
        .build_tree_profiled(&ctx, &default, view, None)
        .unwrap();
    let old = Arc::downgrade(&first.layout);
    drop(first);
    let (_, timings) = cache
        .build_tree_profiled(&ctx, &override_props, view, None)
        .unwrap();
    assert!(!timings.layout_reused);
    assert!(old.upgrade().is_none());
    let (_, timings) = cache
        .build_tree_profiled(&ctx, &override_props, view, None)
        .unwrap();
    assert!(timings.layout_reused);
    let (_, timings) = cache
        .build_tree_profiled(&ctx, &override_props, Viewport::new((800, 600)), None)
        .unwrap();
    assert!(!timings.layout_reused);
    assert!(!cache.is_for(&prepare_scene(&a).unwrap()));
}

#[test]
fn fonts_are_frozen_per_cache_and_new_font_configuration_rebuilds_geometry() {
    let a = compile_motion(SOURCE).unwrap().artifact;
    let p = prepare_scene(&a).unwrap();
    let regular = fonts("NotoSans-Regular.ttf");
    let mono = fonts("NotoSansMono-Regular.ttf");
    let ctx =
        valle_motion::motion_context_at_frame(30, 300, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = valle_motion::resolve_props(&a.controls, &BTreeMap::new()).unwrap();
    let render = |fonts: &Fonts| {
        let cache = LayoutCache::new(&p, fonts).unwrap();
        let cached = cache
            .build_tree(&ctx, &props, Viewport::new((960, 620)), None)
            .unwrap();
        let full = valle_motion::build_tree(
            &p,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((960, 620)),
                fonts,
                styles: None,
            },
        )
        .unwrap();
        assert_eq!(bytes(&cached), bytes(&full));
        bytes(&cached)
    };
    assert_ne!(render(&regular), render(&mono));
}

#[test]
fn unsupported_and_geometry_changing_scenes_use_the_full_path() {
    let fonts = fonts("NotoSans-Regular.ttf");
    for (from, to) in [
        ("width:140", "width:140+ctx.seconds"),
        ("One fixed paragraph.", "{ctx.localFrame}"),
        (
            "backgroundColor:\"#28445b\"",
            "backgroundColor:\"#28445b\",filter:\"blur(2px)\"",
        ),
        ("rotate:interpolate", "rotateX:interpolate"),
    ] {
        let source = SOURCE.replace(from, to);
        assert_ne!(source, SOURCE);
        let a = compile_motion(&source).unwrap().artifact;
        let p = prepare_scene(&a).unwrap();
        assert!(LayoutCache::new(&p, &fonts).is_none(), "{to}");
    }
}

#[test]
fn independent_worker_caches_preserve_the_requested_frame() {
    let a = compile_motion(SOURCE).unwrap().artifact;
    let p = prepare_scene(&a).unwrap();
    let props = valle_motion::resolve_props(&a.controls, &BTreeMap::new()).unwrap();
    let regular = fonts("NotoSans-Regular.ttf");
    let fps = FrameRate::new(60, 1).unwrap();
    let viewport = Viewport::new((960, 620));
    let expected: Vec<_> = [0, 17, 60, 149]
        .into_iter()
        .map(|frame| {
            let ctx = valle_motion::motion_context_at_frame(frame, 300, fps).unwrap();
            bytes(
                &valle_motion::build_tree(
                    &p,
                    &ctx,
                    &props,
                    &LayoutOptions {
                        viewport,
                        fonts: &regular,
                        styles: None,
                    },
                )
                .unwrap(),
            )
        })
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = [0, 17, 60, 149]
            .into_iter()
            .map(|frame| {
                let p = &p;
                let props = &props;
                scope.spawn(move || {
                    let fonts = fonts("NotoSans-Regular.ttf");
                    let cache = LayoutCache::new(p, &fonts).unwrap();
                    let warm = valle_motion::motion_context_at_frame(90, 300, fps).unwrap();
                    let _ = cache.build_tree(&warm, props, viewport, None).unwrap();
                    let ctx = valle_motion::motion_context_at_frame(frame, 300, fps).unwrap();
                    bytes(&cache.build_tree(&ctx, props, viewport, None).unwrap())
                })
            })
            .collect();
        for (handle, expected) in handles.into_iter().zip(expected) {
            assert_eq!(handle.join().unwrap(), expected);
        }
    });
}

#[test]
fn static_layout_props_invalidate_geometry_instead_of_reusing_old_sizes() {
    let artifact = compile_motion(&SOURCE.replace("width:140", "width:100+props.alpha*40"))
        .unwrap()
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = fonts("NotoSans-Regular.ttf");
    let cache = LayoutCache::new(&prepared, &fonts).unwrap();
    let viewport = Viewport::new((960, 620));
    let ctx =
        valle_motion::motion_context_at_frame(30, 300, FrameRate::new(60, 1).unwrap()).unwrap();
    let mut previous = None;
    for alpha in [1.0, 0.0, 0.5, 1.0] {
        let props = valle_motion::resolve_props(
            &artifact.controls,
            &BTreeMap::from([("alpha".into(), MotionValue::Number(alpha))]),
        )
        .unwrap();
        let (cached, timing) = cache
            .build_tree_profiled(&ctx, &props, viewport, None)
            .unwrap();
        assert!(!timing.layout_reused);
        let full = valle_motion::build_tree(
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
        assert_eq!(bytes(&cached), bytes(&full));
        if let Some(old) = previous {
            assert_ne!(old, bytes(&cached));
        }
        previous = Some(bytes(&cached));
        assert!(
            cache
                .build_tree_profiled(&ctx, &props, viewport, None)
                .unwrap()
                .1
                .layout_reused
        );
    }
    fn assert_send<T: Send>() {}
    assert_send::<LayoutCache>();
}
