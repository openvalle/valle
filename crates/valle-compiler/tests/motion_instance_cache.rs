#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, MotionContext, MotionValue, PreparedScene, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

const SOURCE: &str =
    include_str!("../../../web/tests/performance/fixtures/echo-circles.motion.tsx");

fn program(prepared: &PreparedScene, ctx: &MotionContext, shift: f64, size: (u32, u32)) -> Vec<u8> {
    let props = resolve_props(
        &prepared.artifact().controls,
        &BTreeMap::from([("shift".into(), MotionValue::Number(shift))]),
    )
    .unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        prepared,
        ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new(size),
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
fn main_echo_and_worker_clones_share_exact_results_under_seek_and_eviction() {
    let artifact = compile_motion(SOURCE).unwrap().artifact;
    let cached = prepare_scene(&artifact)
        .unwrap()
        .with_instance_cache_limits(4, 1024 * 1024);
    let reference = cached.clone().with_instance_cache_limits(0, 0);
    let peer = cached.clone();
    for (request, frame) in [59, 0, 30, 30, 31, 0, 58, 20, 59, 599, 300, 300, 598]
        .into_iter()
        .enumerate()
    {
        let ctx = motion_context_at_frame(frame, 600, FrameRate::new(60, 1).unwrap()).unwrap();
        let caller = if request % 2 == 0 { &cached } else { &peer };
        assert_eq!(
            program(caller, &ctx, 0.0, (640, 360)),
            program(&reference, &ctx, 0.0, (640, 360))
        );
        let stats = cached.instance_cache_stats().unwrap();
        assert!(stats.entries <= stats.max_entries && stats.resident_bytes <= stats.max_bytes);
    }
    let stats = cached.instance_cache_stats().unwrap();
    assert!(
        stats.hits > 0 && stats.misses > 0 && stats.evictions > 0,
        "{stats:?}"
    );
    assert_eq!(stats.hits, peer.instance_cache_stats().unwrap().hits);
    assert_eq!(reference.instance_cache_stats().unwrap().entries, 0);
}

#[test]
fn props_viewport_host_and_scene_changes_do_not_reuse_stale_batches() {
    for source in [
        SOURCE.to_owned(),
        SOURCE.replace(
            "+props.shift",
            "+props.shift+ctx.host.seconds+ctx.host.progress+ctx.host.duration",
        ),
    ] {
        let artifact = compile_motion(&source).unwrap().artifact;
        let cached = prepare_scene(&artifact).unwrap();
        let reference = cached.clone().with_instance_cache_limits(0, 0);
        let mut ctx = motion_context_at_frame(30, 600, FrameRate::new(60, 1).unwrap()).unwrap();
        for (host, shift, size) in [
            (1, 0.0, (640, 360)),
            (2, 0.0, (640, 360)),
            (2, 3.0, (640, 360)),
            (2, 3.0, (1280, 720)),
            (1, 0.0, (640, 360)),
        ] {
            ctx.host = valle_motion::MotionHostContext::new(
                valle_timeline::internal::SampleTime::new(
                    valle_timeline::RationalTime::new(host, 1).unwrap(),
                ),
                valle_timeline::RationalTime::new(host + 10, 1).unwrap(),
                host as u32,
                600,
            )
            .unwrap();
            assert_eq!(
                program(&cached, &ctx, shift, size),
                program(&reference, &ctx, shift, size)
            );
        }
        assert!(cached.instance_cache_stats().unwrap().hits > 0);
        let changed = prepare_scene(
            &compile_motion(&source.replace("fill=\"#8b7bff\"", "fill=\"#22d3ee\""))
                .unwrap()
                .artifact,
        )
        .unwrap();
        assert_eq!(changed.instance_cache_stats().unwrap().entries, 0);
        assert_ne!(
            program(&changed, &ctx, 0.0, (640, 360)),
            program(&cached, &ctx, 0.0, (640, 360))
        );
    }
}

#[test]
fn small_byte_budget_bypasses_and_parallel_cold_requests_preserve_output() {
    let cached = prepare_scene(&compile_motion(SOURCE).unwrap().artifact).unwrap();
    let bypass = cached.clone().with_instance_cache_limits(32, 1);
    let ctx = motion_context_at_frame(30, 600, FrameRate::new(60, 1).unwrap()).unwrap();
    let expected = program(&bypass, &ctx, 0.0, (640, 360));
    let stats = bypass.instance_cache_stats().unwrap();
    assert!(stats.bypasses > 0 && stats.resident_bytes == 0);
    let programs = std::thread::scope(|scope| {
        (0..4)
            .map(|_| {
                let peer = cached.clone();
                scope.spawn(move || program(&peer, &ctx, 0.0, (640, 360)))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    for actual in programs {
        assert_eq!(actual, expected);
    }
    assert!(cached.instance_cache_stats().unwrap().entries <= 4);
}
