//! Real shared-surface scheduling checks; CPU pixel access here is test-only.
#![cfg(all(feature = "native", target_os = "macos"))]

use std::{ffi::c_void, sync::Arc, time::Instant};

use serde_json::json;
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
use valle_media::{Muxer, SharedVideoFrameHandle, SharedVideoFramePool, VideoFrameTransport};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        DeliveredFrame, DeliveredSharedFrame, FrameDelivery, FrameSink, NativeProject,
        NativeResourceCatalog, PipelineError, RenderControl, render_schedule,
    },
};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

fn project() -> NativeProject {
    let artifact = Arc::new(valle_compiler::motion::compile_motion(r##"
export const composition = { width: 96, height: 64, fps: 30, duration: 2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 16 });
export default function Animated() { const ctx = useTime();
  return <Scene style={{backgroundColor: "#102030"}}>
    <Scene3D key="depth" style={{width:96,height:64}}
      camera={{position:[0,0,5],target:[0,0,0],fov:45,depthOfField:{focusDistance:5,maxBlurRadius:4}}}>
      <Mesh key="far" geometry={BALL} position={[0.4,0,-1]} material={{type:"unlit",color:"#3366EE"}} />
      <Mesh key="near" geometry={BALL} position={[ctx.seconds-1,0,1]} scale={[0.45,0.45,0.45]} material={{type:"unlit",color:"#FF553399"}} />
    </Scene3D>
    <View style={{position:"absolute",left:ctx.localFrame,top:2,width:8,height:4,backgroundColor:"#33CC99"}} />
  </Scene>;
}
"##).unwrap().artifact);
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
        &bundle,
        &profile,
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

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
}

struct InspectSink {
    muxer: Muxer,
    delivery: FrameDelivery,
    pixels: Vec<Vec<u8>>,
    keys: Vec<i64>,
    fail_at: Option<usize>,
    finished: bool,
}

impl InspectSink {
    fn new(path: &std::path::Path) -> Self {
        Self {
            muxer: Muxer::open_ext_with_transport(
                path,
                WIDTH,
                HEIGHT,
                30,
                None,
                true,
                None,
                VideoFrameTransport::SharedGpu,
            )
            .unwrap(),
            delivery: FrameDelivery::SharedGpu,
            pixels: Vec::new(),
            keys: Vec::new(),
            fail_at: None,
            finished: false,
        }
    }
}

impl FrameSink for InspectSink {
    fn push(&mut self, _: DeliveredFrame<'_>) -> anyhow::Result<()> {
        panic!("shared GPU scheduling must never deliver CPU frames")
    }

    fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        self.muxer.shared_frame_pool()
    }

    fn push_shared(&mut self, delivered: DeliveredSharedFrame<'_>) -> anyhow::Result<()> {
        if self.fail_at == Some(delivered.sequence) {
            anyhow::bail!("intentional sink failure");
        }
        assert_eq!(delivered.sequence, self.keys.len());
        assert_eq!(delivered.evidence.frame, delivered.key.index());
        assert_eq!(delivered.evidence.delivery, self.delivery);
        assert_eq!(delivered.evidence.delivery_readback_bytes, 0);
        assert_eq!(delivered.evidence.timings.delivery_readback_us, None);
        assert_eq!(
            delivered.evidence.timings.gpu_completion_wait_us.is_some(),
            self.delivery == FrameDelivery::SharedGpu
        );
        assert_eq!(delivered.evidence.scene3d.requests, 1);
        let SharedVideoFrameHandle::VideoToolbox(buffer) = delivered.frame.handle() else {
            panic!("expected VideoToolbox frame")
        };
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        // push_shared is called only after Ganesh's completion fence. Retain the owning
        // SharedVideoFrame throughout this read, then pass that same surface to the encoder.
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(buffer, 1), 0);
            let data = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(buffer);
            assert!(!data.is_null());
            assert!(stride >= WIDTH as usize * 4);
            for y in 0..HEIGHT as usize {
                pixels.extend_from_slice(std::slice::from_raw_parts(
                    data.add(y * stride),
                    WIDTH as usize * 4,
                ));
            }
            assert_eq!(CVPixelBufferUnlockBaseAddress(buffer, 1), 0);
        }
        self.keys.push(delivered.key.index());
        self.pixels.push(pixels);
        self.muxer.encode_shared(delivered.frame)?;
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.muxer.finish()?;
        self.finished = true;
        Ok(())
    }
}

#[test]
#[ignore = "requires real Metal and VideoToolbox"]
fn parallel_shared_delivery_preserves_seek_order_exact_pixels_and_no_readback() {
    let project = project();
    let dir = tempfile::tempdir().unwrap();
    let indices = [
        0, 7, 20, 59, 12, 45, 7, 1, 30, 2, 58, 0, 59, 30, 12, 7, 48, 5, 1, 20, 30, 59, 0, 7,
    ];
    let frames = indices.map(FrameKey::new);
    let mut baseline = None;
    for workers in [1, 2, 4, 8] {
        let mut sink = InspectSink::new(&dir.path().join(format!("workers-{workers}.mp4")));
        let report = render_schedule(
            &project,
            &frames,
            spec(),
            &mut sink,
            &RenderControl::default()
                .with_budget(valle_render::host::NativeRenderBudget::new(workers, None).unwrap()),
            None,
            SkiaBackendKind::Metal,
        )
        .unwrap();
        assert_eq!(report.preparation_tasks, workers.min(4));
        assert_eq!(report.frames, frames.len());
        assert_eq!(report.delivery, Some(FrameDelivery::SharedGpu));
        assert_eq!(report.delivery_readback_bytes, 0);
        assert!(report.maximum_in_flight_frames <= workers + 3);
        assert!(sink.finished);
        assert_eq!(sink.keys, indices);
        assert_ne!(sink.pixels[0], sink.pixels[3], "fixture must animate");
        if let Some(expected) = &baseline {
            assert_eq!(&sink.pixels, expected, "workers={workers}");
        } else {
            baseline = Some(sink.pixels);
        }
    }
}

#[test]
#[ignore = "requires real VideoToolbox"]
fn raster_shared_delivery_writes_the_readback_pixels_in_place() {
    struct Readback(Vec<Vec<u8>>);
    impl FrameSink for Readback {
        fn push(&mut self, frame: DeliveredFrame<'_>) -> anyhow::Result<()> {
            self.0.push(frame.pixels.data.clone());
            Ok(())
        }
        fn finish(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let project = project();
    let dir = tempfile::tempdir().unwrap();
    let indices = [0, 7, 20, 59, 12, 45, 7, 1, 30, 2];
    let frames = indices.map(FrameKey::new);
    let mut readback = Readback(Vec::new());
    render_schedule(
        &project,
        &frames,
        spec(),
        &mut readback,
        &RenderControl::default(),
        None,
        SkiaBackendKind::Raster,
    )
    .unwrap();
    // The encoder's surfaces are BGRA; the same codes must land there, swizzled.
    let expected = readback
        .0
        .iter()
        .map(|rgba| {
            rgba.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[red, green, blue, alpha]| [blue, green, red, alpha])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for workers in [1, 4] {
        let mut sink = InspectSink::new(&dir.path().join(format!("raster-{workers}.mp4")));
        sink.delivery = FrameDelivery::SharedCpu;
        let report = render_schedule(
            &project,
            &frames,
            spec(),
            &mut sink,
            &RenderControl::default()
                .with_budget(valle_render::host::NativeRenderBudget::new(workers, None).unwrap()),
            None,
            SkiaBackendKind::Raster,
        )
        .unwrap();
        assert_eq!(report.delivery, Some(FrameDelivery::SharedCpu));
        assert_eq!(report.delivery_readback_bytes, 0);
        assert!(sink.finished);
        assert_eq!(sink.keys, indices);
        assert_eq!(sink.pixels, expected, "workers={workers}");
    }
}

#[test]
#[ignore = "requires real Metal and VideoToolbox"]
fn shared_delivery_cancellation_and_sink_failure_drain_blocked_preparers() {
    let project = project();
    let dir = tempfile::tempdir().unwrap();
    let frames = (0..24).map(FrameKey::new).collect::<Vec<_>>();
    for failure in [false, true] {
        let mut sink = InspectSink::new(&dir.path().join(format!("failure-{failure}.mp4")));
        sink.fail_at = failure.then_some(1);
        let control = RenderControl::default();
        let cancel = control.clone();
        let progress: valle_render::host::ProgressCallback = Arc::new(move |done, _| {
            if !failure && done == 1 {
                cancel.cancel();
            }
        });
        let error = render_schedule(
            &project,
            &frames,
            spec(),
            &mut sink,
            &control,
            Some(&progress),
            SkiaBackendKind::Metal,
        )
        .unwrap_err();
        if failure {
            assert!(
                matches!(error, PipelineError::Sink(ref text) if text.contains("intentional sink failure"))
            );
        } else {
            assert!(matches!(error, PipelineError::Cancelled));
        }
        assert_eq!(sink.keys.len(), 1);
        assert!(!sink.finished);
    }
}

#[test]
#[ignore = "requires real Metal and VideoToolbox"]
fn shared_delivery_deadline_does_not_finish_the_sink() {
    let project = project();
    let dir = tempfile::tempdir().unwrap();
    for workers in [1, 8] {
        let mut sink = InspectSink::new(&dir.path().join(format!("deadline-{workers}.mp4")));
        let control = RenderControl::with_deadline(Instant::now());
        let error = render_schedule(
            &project,
            &[FrameKey::new(0), FrameKey::new(1)],
            spec(),
            &mut sink,
            &control,
            None,
            SkiaBackendKind::Metal,
        )
        .unwrap_err();
        assert!(matches!(error, PipelineError::Deadline));
        assert!(sink.keys.is_empty());
        assert!(!sink.finished);
    }
}
