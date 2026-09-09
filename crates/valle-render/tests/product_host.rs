mod support;

use std::{io::Cursor, sync::Arc};

use valle_engine::render::FrameKey;
use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        FrameSchedule, NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
    },
};

fn encoded_image() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ])
            .unwrap();
    }
    bytes
}

fn project() -> NativeProject {
    let image = encoded_image();
    let digest = ContentDigest::of_bytes(&image);
    let digest_wire = digest.to_wire();
    let render = support::opened_image_render(&digest_wire);
    let mut catalog = NativeResourceCatalog::new();
    catalog
        .insert_bytes(digest, Arc::<[u8]>::from(image))
        .unwrap();
    NativeProject::from_render(render, Arc::new(catalog))
}

#[test]
fn native_project_schedule_and_preview_keep_one_render_id() {
    let project = project();
    let render = project.render_id();
    assert_eq!(project.render().render_id(), render);
    assert_eq!(project.compiled().render_id(), render);

    let frames = FrameSchedule::Range {
        from_seconds: 0.5,
        to_seconds: 0.6,
    }
    .frames(project.compiled().canvas())
    .unwrap();
    assert_eq!(
        frames.iter().map(|frame| frame.index()).collect::<Vec<_>>(),
        [15, 16, 17]
    );

    let output = OutputSpec::srgb_preview(OutputBackground::opaque_srgb([12, 34, 56])).unwrap();
    let spec = RenderSpec::new(2, 2, RenderQuality::Preview, output).unwrap();
    let mut runner = project.frame_runner(SkiaBackendKind::Raster).unwrap();
    let (frame, evidence) = runner.render_rgba8(FrameKey::new(0), spec).unwrap();
    assert_eq!(evidence.render_id, render);
    assert_eq!((frame.width, frame.height), (2, 2));
    assert!(frame.data.iter().any(|channel| *channel != 0));
}

#[test]
fn native_renderer_summary_reports_the_pinned_render() {
    let project = project();
    let render = project.render_id();
    let mut options = NativeRenderOptions::default();
    options.background = OutputBackground::opaque_srgb([12, 34, 56]);
    options.raster_workers = Some(1);
    let output = std::env::temp_dir().join(format!(
        "valle-native-fixed-render-{}-{}.png",
        std::process::id(),
        render
    ));
    let summary = NativeRenderer::new(project, options)
        .preview_frame_key(FrameKey::new(0), &output)
        .unwrap();
    assert_eq!(summary.render_id, render);
    assert_eq!(summary.pipeline.as_ref().unwrap().render_id, render);
    assert_eq!(summary.frames, 1);
    assert!(std::fs::metadata(&output).unwrap().len() > 0);
    std::fs::remove_file(output).unwrap();
}

#[test]
fn native_renderer_rejects_a_frame_outside_the_pinned_canvas() {
    let project = project();
    let frame_count = project.compiled().canvas().frame_count();
    let output = std::env::temp_dir().join(format!(
        "valle-native-out-of-range-{}-{frame_count}.png",
        std::process::id()
    ));
    let error = NativeRenderer::new(project, NativeRenderOptions::default())
        .preview_frame_key(FrameKey::new(frame_count), &output)
        .unwrap_err();
    assert!(matches!(
        error,
        valle_render::host::NativeRenderError::FrameOutOfRange {
            frame,
            frame_count: limit,
        } if frame == frame_count && limit == frame_count
    ));
    assert!(!output.exists());
}

#[test]
fn motion_seek_pixels_survive_warm_caches_eviction_and_surface_reset() {
    use serde_json::json;
    use valle_compiler::timeline_contract::{
        ResourceEntryWire, canonical_bytes, decode_canonical, decode_resource_manifest,
    };
    use valle_engine::fixed_package::*;
    use valle_engine::render::{
        Capabilities, ResourceBinding, ResourceBindings, VerifiedHandleId, VerifiedResourceFacts,
        VisualFootprint,
    };
    let artifact = Arc::new(valle_compiler::motion::compile_motion(r##"
export default function Demo(ctx) {
  const f = ctx.localFrame;
  return <Scene style={{width:48,height:48,backgroundColor:'#14213d'}}>
    <View style={{position:'absolute',left:3,top:4,width:35,height:36,overflow:'hidden',borderRadius:7.3,opacity:0.63 + f / 3600}}>
      <View style={{position:'absolute',left:f / 60,top:0,width:32,height:30,backgroundColor:'#fca311'}} />
      <View style={{position:'absolute',left:12,top:8,width:30,height:30,backgroundColor:'#00aaff88'}} />
    </View>
  </Scene>;
}
"##).unwrap().artifact);
    let digest = ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
    let manifest = decode_resource_manifest(
        &serde_json::to_vec(&json!({"entries":{"component:seek":{
          "kind":"motion-artifact","digest":digest.to_wire(),"abi":"valle.motion/artifact@1",
          "descriptor":{"readsDestination":false,"boundarySampling":"left-limit"}
        }}}))
        .unwrap(),
    )
    .unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi, descriptor, ..
    } = &manifest.entries()["component:seek"]
    else {
        unreachable!()
    };
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:seek",
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
      "canvas":{"width":48,"height":48,"fps":"30/1","sampleRate":48000,"channelLayout":"stereo","colorSpace":"srgb","duration":"12/1"},
      "background":{"color":"#000000ff"},"visual":{"tracks":[{"id":"track:seek","items":[{
        "type":"clip","id":"clip:seek","duration":"12/1",
        "layer":{"transform":{"position":constant(json!([0.5,0.5])),"scale":constant(json!([1,1])),"rotation":constant(json!(0)),"anchor":[0.5,0.5]},"opacity":constant(json!(1)),"mask":null,"filters":[],"blend":"normal"},
        "source":{"type":"motion","component":"component:seek","sourceStart":"0/1","sourceDuration":"12/1","rate":"1/1","endBehavior":"hold","props":{},"cues":{},"resources":{},"phases":{"enterDuration":null,"exitDuration":null}}
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
    let project = NativeProject::from_render(render, Arc::new(NativeResourceCatalog::new()));
    let spec = RenderSpec::new(
        48,
        48,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut runner = project.frame_runner(SkiaBackendKind::Raster).unwrap();
    let mut baseline = std::collections::BTreeMap::new();
    for frame in [0, 45, 359, 120] {
        let mut cold = project.frame_runner(SkiaBackendKind::Raster).unwrap();
        baseline.insert(
            frame,
            cold.render_rgba8(FrameKey::new(frame), spec)
                .unwrap()
                .0
                .data,
        );
    }
    assert_ne!(
        baseline[&0], baseline[&359],
        "fixture must change its pixels"
    );
    let key = FrameKey::new(0);
    runner.render_rgba8(key, spec).unwrap();
    let (_, warm) = runner.render_rgba8(key, spec).unwrap();
    assert!(warm.execution.program_cache_hits > 0);
    assert!(warm.execution.program_cache_entries > 0);
    assert!(warm.execution.program_cache_cost_bytes > 0);
    for frame in 1..360 {
        let (_, evidence) = runner.render_rgba8(FrameKey::new(frame), spec).unwrap();
        assert!(evidence.execution.program_cache_entries <= 256);
        assert!(evidence.execution.program_cache_cost_bytes <= 32 * 1024 * 1024);
    }
    let (_, evicted) = runner.render_rgba8(key, spec).unwrap();
    assert!(
        evicted.execution.program_cache_misses > 0,
        "360 unique programs must evict frame zero"
    );
    for reset in [false, true] {
        if reset {
            runner.invalidate_surface_generation().unwrap();
        }
        for frame in [359, 45, 0, 120, 45, 359, 0] {
            let (pixels, _) = runner.render_rgba8(FrameKey::new(frame), spec).unwrap();
            assert_eq!(
                pixels.data, baseline[&frame],
                "frame {frame}, reset={reset}"
            );
        }
    }
}
