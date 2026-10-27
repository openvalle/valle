//! Vulkan submission, asynchronous readback and ordered delivery on real hardware.
#![cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]

use std::{sync::Arc, time::{Duration, Instant}};
use serde_json::json;
use valle_compiler::timeline_contract::{ResourceEntryWire, canonical_bytes, decode_canonical, decode_resource_manifest};
use valle_engine::{fixed_package::*, frame::{RenderQuality, RenderSpec}, render::{Capabilities, FrameKey,
    ResourceBinding, ResourceBindings, VerifiedHandleId, VerifiedResourceFacts, VisualFootprint},
    resource::{ContentDigest, OutputBackground, OutputSpec}};
use valle_render::{executor::skia::SkiaBackendKind, host::{DeliveredFrame, FrameDelivery, FrameSink,
    NativeProject, NativeResourceCatalog, NativeRenderBudget, PipelineError, RenderControl, RenderTrace, render_schedule}};

const WIDTH: u32 = 98;
const HEIGHT: u32 = 64;

fn project() -> NativeProject {
    project_with_gpu_raster(false)
}

fn project_with_gpu_raster(gpu_raster: bool) -> NativeProject {
    project_with_raster_segments(gpu_raster, 3)
}

fn project_with_raster_segments(gpu_raster: bool, segments: u32) -> NativeProject {
    let source = r##"
export const composition = { width: 98, height: 64, fps: 30, duration: 2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 16 });
export default function Animated() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return <Scene style={{backgroundColor: "#102030"}}>
    <Scene3D key="depth" style={{width:98,height:64}}
      camera={{position:[0,0,5],target:[0,0,0],fov:45,depthOfField:{focusDistance:5,maxBlurRadius:4}}}>
      <Mesh key="far" geometry={BALL} position={[0.4,0,-1]} material={{type:"unlit",color:"#3366EE"}} />
      <Mesh key="near" geometry={BALL} position={[time.seconds-1,0,1]} scale={[0.45,0.45,0.45]} material={{type:"unlit",color:"#FF553399"}} />
    </Scene3D>
    <View style={{position:"absolute",left:frame,top:2,width:8,height:4,backgroundColor:"#33CC99"}} />
  </Scene>;
}
"##;
    let source = if gpu_raster {
        source
            .replace(",depthOfField:{focusDistance:5,maxBlurRadius:4}", "")
            .replace("#FF553399", "#FF5533")
            .replace("segments: 16", &format!("segments: {segments}"))
            .replace("<View style={{position:\"absolute\",left:frame,top:2,width:8,height:4,backgroundColor:\"#33CC99\"}} />", "")
    } else {
        source.to_owned()
    };
    project_from_source(&source)
}

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

fn spec() -> RenderSpec {
    RenderSpec::new(
        WIDTH,
        HEIGHT,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap()
}

#[derive(Default)]
struct InspectSink {
    pixels: Vec<Vec<u8>>,
    keys: Vec<i64>,
    fail_at: Option<usize>,
    cancel: Option<RenderControl>,
    slow: bool,
    finished: bool,
}

impl FrameSink for InspectSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> anyhow::Result<()> {
        assert_eq!(frame.sequence, self.pixels.len());
        assert_eq!(frame.evidence.delivery, FrameDelivery::AsyncCpuReadback);
        assert_eq!(frame.evidence.delivery_readback_bytes, u64::from(WIDTH * HEIGHT * 4));
        assert!(frame.evidence.timings.gpu_completion_wait_us.is_some());
        if self.fail_at == Some(frame.sequence) { anyhow::bail!("intentional sink failure"); }
        if self.slow { std::thread::sleep(Duration::from_millis(8)); }
        self.keys.push(frame.key.index());
        self.pixels.push(frame.pixels.data.clone());
        if frame.sequence == 2 && let Some(control) = &self.cancel { control.cancel(); }
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> { self.finished = true; Ok(()) }
}

#[test]
#[ignore = "requires a hardware Vulkan GPU"]
fn asynchronous_readback_preserves_pixels_seek_order_and_budget() {
    for gpu in [false, true] {
        let project = project_with_gpu_raster(gpu);
        let frames = [0, 7, 20, 59, 12, 45, 7, 1, 30, 2, 58, 0, 59, 30, 12, 7].map(FrameKey::new);
        let mut reference = project.frame_runner(SkiaBackendKind::Vulkan).unwrap();
        let expected: Vec<_> = frames.iter().map(|&key| reference.render_rgba8(key, spec()).unwrap().0.data).collect();
        for threads in [1, 2, 4] {
            let trace = RenderTrace::new(100_000);
            let control = RenderControl::default().with_trace(trace.clone()).with_budget(
                NativeRenderBudget::new(threads, Some(24 * 1024 * 1024)).unwrap());
            let mut sink = InspectSink { slow: threads == 4, ..Default::default() };
            let report = render_schedule(&project, &frames, spec(), &mut sink, &control, None,
                SkiaBackendKind::Vulkan).unwrap();
            assert!(sink.finished);
            assert_eq!(sink.keys, frames.map(|key| key.index()));
            for (index, (actual, expected)) in sink.pixels.iter().zip(&expected).enumerate() {
                let different = actual.iter().zip(expected).filter(|(a, b)| a != b).count();
                let maximum = actual.iter().zip(expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert_eq!(different, 0, "gpu={gpu}, threads={threads}, frame={index}, max difference={maximum}");
            }
            assert_eq!(report.frames, frames.len());
            assert_eq!(report.scene3d_gpu_rasters, if gpu { frames.len() as u64 } else { 0 });
            assert_eq!(report.scene3d_cpu_fallbacks, if gpu { 0 } else { frames.len() as u64 });
            let memory = report.budget.unwrap().memory;
            assert_eq!(memory.used_bytes, 0);
            assert!(memory.peak_bytes <= 24 * 1024 * 1024);
            let events = trace.snapshot(json!({}));
            let queues: Vec<_> = events["traceEvents"].as_array().unwrap().iter()
                .filter(|e| e["name"] == "vulkan.queue").collect();
            assert_eq!(queues.len(), frames.len());
            assert!(queues.iter().all(|e| e["args"]["pending"].as_u64().unwrap() <= threads.min(3) as u64));
        }
    }
}

#[test]
#[ignore = "requires a hardware Vulkan GPU"]
fn cancellation_and_sink_failure_retire_pending_gpu_resources() {
    let project = project();
    let frames: Vec<_> = (0..60).map(FrameKey::new).collect();
    for cancel in [false, true] {
        let budget = NativeRenderBudget::new(4, Some(32 * 1024 * 1024)).unwrap();
        let control = RenderControl::default().with_budget(budget);
        let mut sink = InspectSink {
            fail_at: (!cancel).then_some(3), cancel: cancel.then(|| control.clone()), ..Default::default()
        };
        let started = Instant::now();
        let error = render_schedule(&project, &frames, spec(), &mut sink, &control, None,
            SkiaBackendKind::Vulkan).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(!sink.finished);
        assert_eq!(sink.keys, [0, 1, 2]);
        assert!(matches!(error, PipelineError::Cancelled) || (!cancel && error.to_string().contains("intentional sink failure")), "{error}");
        // A fresh context must remain usable after all pending work was discarded.
        let mut recovery = InspectSink::default();
        render_schedule(&project, &frames[..4], spec(), &mut recovery, &RenderControl::default(), None,
            SkiaBackendKind::Vulkan).unwrap();
        assert!(recovery.finished);
    }
}
