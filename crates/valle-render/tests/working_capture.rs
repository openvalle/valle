//! Capture the real composited working image, not an F16-encoded delivery image.
#![cfg(feature = "native")]
use serde_json::json;
use std::sync::Arc;
use valle_compiler::timeline_contract::{
    ResourceEntryWire, canonical_bytes, decode_canonical, decode_resource_manifest,
};
use valle_engine::{
    fixed_package::*,
    frame::{RenderQuality, RenderSpec},
    render::{
        Capabilities, FrameKey, ResourceBinding, ResourceBindings, VerifiedHandleId,
        VerifiedResourceFacts, VisualFootprint,
    },
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{NativeProject, NativeResourceCatalog},
};
const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
fn project_from_source(source: &str) -> NativeProject {
    let artifact = Arc::new(
        valle_compiler::motion::compile_motion(source)
            .unwrap()
            .artifact,
    );
    let digest = ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
    let manifest = decode_resource_manifest(&serde_json::to_vec(&json!({"entries":{
        "component:shared":{"kind":"motion-artifact","digest":digest.to_wire(),"abi":"valle.motion/artifact@1",
        "descriptor":{"readsDestination":false,"boundarySampling":"left-limit"}}
    }})).unwrap()).unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi, descriptor, ..
    } = &manifest.entries()["component:shared"]
    else {
        unreachable!()
    };
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:shared",
            ResourceBinding::new(
                digest,
                VerifiedHandleId::new(1).unwrap(),
                VerifiedResourceFacts::MotionArtifact {
                    abi: *abi,
                    descriptor: descriptor.clone(),
                    artifact,
                    temporal_footprint: VisualFootprint::default(),
                },
            ),
        )
        .unwrap();
    let constant = |value| json!({"type":"constant","value":value});
    let document = decode_canonical(&json!({"document":{
        "canvas":{"width":WIDTH,"height":HEIGHT,"fps":"30/1","sampleRate":48000,"channelLayout":"stereo","colorSpace":"srgb","duration":"2/1"},
        "background":{"color":"#000000ff"},"visual":{"tracks":[{"id":"track:shared","items":[{
            "type":"clip","id":"clip:shared","duration":"2/1",
            "layer":{"transform":{"position":constant(json!([0.5,0.5])),"scale":constant(json!([1,1])),"rotation":constant(json!(0)),"anchor":[0.5,0.5]},"opacity":constant(json!(1)),"mask":null,"filters":[],"blend":"normal"},
            "source":{"type":"motion","component":"component:shared","sourceStart":"0/1","role":{"type":"clip"},"sourceDuration":"2/1","rate":"1/1","endBehavior":"hold","fit":"contain","props":{},"resources":{}}
        }]}]},"audio":{"tracks":[]},"adjustments":[],"captions":{"tracks":[]},"camera":null,"metadata":{}
    }}).to_string()).unwrap();
    let timeline = String::from_utf8(canonical_bytes(&document).unwrap()).unwrap();
    let bundle = canonical_verified_binding_bundle(
        &bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
    )
    .unwrap();
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(
        &timeline,
        std::str::from_utf8(manifest.canonical_bytes()).unwrap(),
        &bundle.json,
        &profile,
        &bundle.resource_bytes,
    );
    let outer = canonical_fixed_package_manifest(&files).unwrap();
    NativeProject::from_render(
        open_verified_fixed_package(&outer, &files)
            .unwrap()
            .engine_render(),
        Arc::new(NativeResourceCatalog::new()),
    )
}

fn exercise(backend: SkiaBackendKind) {
    let project = project_from_source(include_str!(
        "../../../web/tests/performance/fixtures/working-hdr.motion.tsx"
    ));
    let mut runner = project.frame_runner(backend).unwrap();
    let memory = valle_engine::motion::memory::MemoryBudget::new(Some(64 * 1024 * 1024));
    let cpu =
        valle_engine::motion::cpu::CpuBudget::new_with_memory(4, Some(memory.clone())).unwrap();
    let spec = |background| {
        RenderSpec::new(
            WIDTH,
            HEIGHT,
            RenderQuality::Final,
            OutputSpec::srgb_delivery(background).unwrap(),
        )
        .unwrap()
    };
    let transparent = spec(OutputBackground::Transparent);
    cpu.run(|| runner.render_rgba8(FrameKey::new(0), transparent))
        .unwrap();
    assert!(runner.take_working_capture().is_none());
    let mut expected = None;
    for frame in [0, 30, 0] {
        let capture = cpu.run(|| {
            runner.capture_next_working_frame();
            runner
                .render_rgba8(FrameKey::new(frame), transparent)
                .unwrap();
            runner.take_working_capture().unwrap()
        });
        assert_eq!(capture.rgba16f_le.len(), (WIDTH * HEIGHT * 8) as usize);
        let channels: Vec<_> = capture
            .rgba16f_le
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert!(channels.iter().all(|bits| bits & 0x7c00 != 0x7c00));
        assert!(
            channels
                .iter()
                .any(|bits| bits & 0x8000 == 0 && *bits > 0x3c00),
            "HDR was clipped before capture"
        );
        assert!(
            channels
                .iter()
                .any(|bits| bits & 0x8000 != 0 && bits & 0x7fff != 0),
            "negative working channels were lost"
        );
        if frame == 0 {
            if let Some(expected) = &expected {
                assert!(
                    &capture.rgba16f_le == expected,
                    "repeated working capture changed"
                );
            } else {
                expected = Some(capture.rgba16f_le.clone());
            }
        }
        let before = memory.report().used_bytes;
        let bytes = capture.rgba16f_le.len() as u64;
        drop(capture);
        assert_eq!(
            before - memory.report().used_bytes,
            bytes,
            "capture lease must survive rendering and release with pixels"
        );
    }
    cpu.run(|| runner.render_rgba8(FrameKey::new(1), transparent))
        .unwrap();
    assert!(
        runner.take_working_capture().is_none(),
        "capture must be opt-in for each execution"
    );
    cpu.run(|| {
        runner.capture_next_working_frame();
        runner
            .render_rgba8(
                FrameKey::new(0),
                RenderSpec::new(
                    WIDTH,
                    HEIGHT,
                    RenderQuality::Final,
                    OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    });
    // Background is part of the planned composition; only change the terminal output
    // conversion here. Preview removes delivery dithering without changing working content.
    assert!(
        runner.take_working_capture().unwrap().rgba16f_le == expected.unwrap(),
        "terminal output conversion changed working capture"
    );
}
#[test]
fn raster_capture_precedes_output_transform_and_owns_its_budget() {
    exercise(SkiaBackendKind::Raster);
}
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires real Metal; validates pre-delivery GPU working readback"]
fn metal_capture_precedes_output_transform_and_owns_its_budget() {
    exercise(SkiaBackendKind::Metal);
}
