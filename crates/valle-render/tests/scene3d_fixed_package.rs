//! A real pinned model package must render without any host asset path or catalog entry.
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
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

#[test]
fn frozen_model_package_preserves_pixels_through_seeks_and_cache_reset() {
    verify_fixed_model_package(false);
}

#[test]
fn frozen_pbr_package_preserves_material_lighting_pixels_through_seeks_and_cache_reset() {
    verify_fixed_model_package(true);
}

fn verify_fixed_model_package(pbr: bool) {
    // Select Metal explicitly outside the sandbox; an unavailable GPU must fail the test.
    let backend = match std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("raster") => SkiaBackendKind::Raster,
        #[cfg(target_os = "macos")]
        Ok("metal") => SkiaBackendKind::Metal,
        other => panic!("unsupported test backend: {other:?}"),
    };
    let (source, model_bytes): (&str, &[u8]) = if pbr {
        (
            include_str!("../../valle-motion/tests/fixtures/scene3d/pbr.motion.tsx"),
            include_bytes!("../../valle-motion/tests/fixtures/scene3d/pbr.glb"),
        )
    } else {
        (
            include_str!("../../valle-motion/tests/fixtures/scene3d/triangle.motion.tsx"),
            include_bytes!("../../valle-motion/tests/fixtures/scene3d/triangle.glb"),
        )
    };
    let bytes: Arc<[u8]> = Arc::from(model_bytes);
    let model_digest = ContentDigest::of_bytes(&bytes);
    let artifact = Arc::new(
        valle_compiler::motion::compile_motion_with_resources(
            source,
            &[valle_motion::ResourceRef {
                control: "model".to_owned(),
                content_hash: model_digest,
            }],
        )
        .unwrap()
        .artifact,
    );
    let artifact_digest =
        ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
    let manifest = decode_resource_manifest(&serde_json::to_vec(&json!({"entries":{
        "component:model":{"kind":"motion-artifact","digest":artifact_digest.to_wire(),"abi":"valle.motion/artifact@1","descriptor":{"readsDestination":false,"boundarySampling":"left-limit"}},
        "asset:model":{"kind":"model3d","digest":model_digest.to_wire(),"descriptor":{"byteLength":bytes.len(),"vertexCount":3,"triangleCount":1}}
    }})).unwrap()).unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi, descriptor, ..
    } = &manifest.entries()["component:model"]
    else {
        unreachable!()
    };
    let motion_binding = ResourceBinding::new(
        artifact_digest,
        VerifiedHandleId::new(1).unwrap(),
        VerifiedResourceFacts::MotionArtifact {
            abi: *abi,
            descriptor: descriptor.clone(),
            artifact,
            temporal_footprint: VisualFootprint::default(),
        },
    )
    .with_dependency(
        valle_engine::render::ResourceDependency::new("model", "asset:model").unwrap(),
    );
    let ResourceEntryWire::Model3d { descriptor, .. } = &manifest.entries()["asset:model"] else {
        unreachable!()
    };
    let model_binding = ResourceBinding::new(
        model_digest,
        VerifiedHandleId::new(2).unwrap(),
        VerifiedResourceFacts::Model3d {
            descriptor: descriptor.clone(),
            bytes,
        },
    );
    let bindings = ResourceBindings::new()
        .with_binding("component:model", motion_binding)
        .unwrap()
        .with_binding("asset:model", model_binding)
        .unwrap();
    let constant = |value| json!({"type":"constant","value":value});
    let document = decode_canonical(&json!({"document":{
      "canvas":{"width":64,"height":64,"fps":"10/1","sampleRate":48000,"channelLayout":"stereo","colorSpace":"srgb","duration":"3/5"},
      "background":{"color":"#000000ff"},"visual":{"tracks":[{"id":"track:model","items":[{
        "type":"clip","id":"clip:model","duration":"3/5",
        "layer":{"transform":{"position":constant(json!([0.5,0.5])),"scale":constant(json!([1,1])),"rotation":constant(json!(0)),"anchor":[0.5,0.5]},"opacity":constant(json!(1)),"mask":null,"filters":[],"blend":"normal"},
        "source":{"type":"motion","component":"component:model","sourceStart":"0/1","sourceDuration":"3/5","rate":"1/1","endBehavior":"hold","props":{},"cues":{},"resources":{"model":"asset:model"},"phases":{"enterDuration":null,"exitDuration":null}}
      }]}]},"audio":{"tracks":[]},"adjustments":[],"captions":{"tracks":[]},"camera":null,"metadata":{}
    }}).to_string()).unwrap();
    let timeline = String::from_utf8(canonical_bytes(&document).unwrap()).unwrap();
    let manifest = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
    let bundle = canonical_verified_binding_bundle(
        &bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
    )
    .unwrap();
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(&timeline, manifest, &bundle, &profile);
    let outer = canonical_fixed_package_manifest(&files).unwrap();
    let render = open_verified_fixed_package(&outer, &files)
        .unwrap()
        .engine_render();
    let admitted = render.admitted_models().collect::<Vec<_>>();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].1.images().len(), if pbr { 5 } else { 0 });
    let project = NativeProject::from_render(render, Arc::new(NativeResourceCatalog::new()));
    let spec = RenderSpec::new(
        64,
        64,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut baseline = BTreeMap::new();
    for frame in [0, 1, 3, 5] {
        let mut cold = project.frame_runner(backend).unwrap();
        let (pixels, evidence) = cold.render_rgba8(FrameKey::new(frame), spec).unwrap();
        assert_eq!(evidence.scene3d.requests, 1);
        assert_eq!(evidence.scene3d.prepared_cache_misses, 1);
        assert!(
            pixels
                .data
                .chunks_exact(4)
                .filter(|p| p[0] > if pbr { 50 } else { 150 })
                .count()
                > 100
        );
        baseline.insert(frame, pixels.data);
    }
    assert_ne!(baseline[&0], baseline[&5]);
    let mut runner = project.frame_runner(backend).unwrap();
    runner.render_rgba8(FrameKey::new(0), spec).unwrap();
    let (_, warm) = runner.render_rgba8(FrameKey::new(1), spec).unwrap();
    assert_eq!(
        warm.scene3d.prepared_cache_hits, 1,
        "pose changes reuse preparation"
    );
    for reset in [false, true] {
        if reset {
            runner.invalidate_surface_generation().unwrap();
        }
        for frame in [5, 3, 1, 0, 5, 0, 3] {
            let (pixels, _) = runner.render_rgba8(FrameKey::new(frame), spec).unwrap();
            assert_eq!(
                pixels.data, baseline[&frame],
                "frame={frame}, reset={reset}"
            );
        }
    }
}
