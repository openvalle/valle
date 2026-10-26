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
    project_with_gpu_raster(false)
}

fn project_with_gpu_raster(gpu_raster: bool) -> NativeProject {
    project_with_raster_segments(gpu_raster, 3)
}

fn project_with_raster_segments(gpu_raster: bool, segments: u32) -> NativeProject {
    let source = r##"
export const composition = { width: 96, height: 64, fps: 30, duration: 2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 16 });
export default function Animated() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return <Scene style={{backgroundColor: "#102030"}}>
    <Scene3D key="depth" style={{width:96,height:64}}
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
    width: u32,
    height: u32,
    muxer: Muxer,
    delivery: FrameDelivery,
    pixels: Vec<Vec<u8>>,
    keys: Vec<i64>,
    fail_at: Option<usize>,
    finished: bool,
}

impl InspectSink {
    fn new(path: &std::path::Path) -> Self {
        Self::new_sized(path, WIDTH, HEIGHT)
    }

    fn new_sized(path: &std::path::Path, width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            muxer: Muxer::open_ext_with_transport(
                path,
                width,
                height,
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
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        // push_shared is called only after Ganesh's completion fence. Retain the owning
        // SharedVideoFrame throughout this read, then pass that same surface to the encoder.
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(buffer, 1), 0);
            let data = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(buffer);
            assert!(!data.is_null());
            assert!(stride >= self.width as usize * 4);
            for y in 0..self.height as usize {
                pixels.extend_from_slice(std::slice::from_raw_parts(
                    data.add(y * stride),
                    self.width as usize * 4,
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
    verify_parallel_shared_delivery(false);
}

#[test]
#[ignore = "requires real Metal and VideoToolbox"]
fn gpu_raster_shared_delivery_retains_textures_through_completion_and_seeks() {
    verify_parallel_shared_delivery(true);
}

fn verify_parallel_shared_delivery(gpu_raster: bool) {
    let project = project_with_gpu_raster(gpu_raster);
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
            &RenderControl::default().with_budget(
                valle_render::host::NativeRenderBudget::new(
                    workers,
                    gpu_raster.then_some(16 * 1024 * 1024),
                )
                .unwrap(),
            ),
            None,
            SkiaBackendKind::Metal,
        )
        .unwrap();
        assert_eq!(report.preparation_tasks, workers.min(4));
        assert_eq!(report.frames, frames.len());
        assert_eq!(report.delivery, Some(FrameDelivery::SharedGpu));
        assert_eq!(report.delivery_readback_bytes, 0);
        assert_eq!(
            report.scene3d_gpu_rasters,
            if gpu_raster { frames.len() as u64 } else { 0 }
        );
        assert_eq!(
            report.scene3d_cpu_fallbacks,
            if gpu_raster { 0 } else { frames.len() as u64 }
        );
        assert!(report.maximum_in_flight_frames <= workers + 3);
        assert_eq!(
            report.budget.unwrap().memory.used_bytes,
            0,
            "all GPU leases must return after delivery"
        );
        if gpu_raster {
            assert!(report.budget.unwrap().memory.peak_bytes <= 16 * 1024 * 1024);
        }
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
#[ignore = "requires real Metal; measures native Scene3D including final output readback"]
fn gpu_raster_scales_to_delivery_resolution_without_pixel_uploads() {
    // Keep the CPU-reference environment isolated from concurrently running tests.
    const REFERENCE_DIR: &str = "VALLE_TEST_SCENE3D_REFERENCE_DIR";
    let child_directory = std::env::var_os(REFERENCE_DIR);
    let directory = tempfile::tempdir().unwrap();
    if child_directory.is_none() {
        let reference = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "gpu_raster_scales_to_delivery_resolution_without_pixel_uploads",
                "--ignored",
                "--nocapture",
            ])
            .env("VALLE_SCENE3D_RASTER_CPU_REFERENCE", "1")
            .env(REFERENCE_DIR, directory.path())
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "CPU reference failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&reference.stderr));
    }
    let path = child_directory
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().to_path_buf());
    let project = project_with_gpu_raster(true);
    let tiny = RenderSpec::new(
        8,
        8,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let (_, evidence) = project_with_raster_segments(true, 16)
        .frame_runner(SkiaBackendKind::Metal)
        .unwrap()
        .render_rgba8(FrameKey::new(0), tiny)
        .unwrap();
    assert_eq!(
        evidence.execution.scene3d_gpu_rasters, 0,
        "dense geometry at tiny sizes retains the CPU admission bound"
    );
    assert_eq!(evidence.execution.scene3d_cpu_fallbacks, 1);
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let output = RenderSpec::new(
            width,
            height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
        )
        .unwrap();
        let mut runner = project.frame_runner(SkiaBackendKind::Metal).unwrap();
        runner.render_rgba8(FrameKey::new(0), output).unwrap();
        let mut samples = Vec::new();
        let mut upload_bytes = Vec::new();
        for frame in [0, 7, 20, 59, 12, 7] {
            let started = Instant::now();
            let (pixels, evidence) = runner.render_rgba8(FrameKey::new(frame), output).unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            upload_bytes.push(evidence.execution.cpu_upload_bytes);
            let file = path.join(format!("{width}x{height}-{frame}.rgba"));
            if child_directory.is_some() {
                assert_eq!(evidence.execution.scene3d_gpu_rasters, 0);
                assert_eq!(evidence.execution.scene3d_cpu_fallbacks, 1);
                std::fs::write(file, pixels.data).unwrap();
            } else {
                assert_eq!(evidence.execution.scene3d_gpu_rasters, 1);
                assert_eq!(evidence.execution.scene3d_cpu_fallbacks, 0);
                let expected = std::fs::read(file).unwrap();
                assert_eq!(pixels.data.len(), expected.len());
                let mismatches = pixels
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0.iter())
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(mismatches, 0, "{width}x{height}, frame={frame}");
                assert!(evidence.execution.cpu_upload_bytes < u64::from(width) * u64::from(height));
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "Scene3D Metal cpu_reference={} {width}x{height}: warm median {:.3}ms including final readback, upload_bytes={upload_bytes:?}",
            child_directory.is_some(),
            samples[samples.len() / 2]
        );
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

fn lit_project(kind: &str, aces: bool) -> NativeProject {
    lit_project_with_dof(kind, aces, false)
}

fn lit_project_with_dof(kind: &str, aces: bool, dof: bool) -> NativeProject {
    lit_project_features(kind, aces, dof, false)
}

fn lit_project_features(kind: &str, aces: bool, dof: bool, advanced: bool) -> NativeProject {
    let source = r##"
export const composition = { width:96, height:64, fps:30, duration:2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), {segments:16});
export default function Lit() { const time = useCurrentTime(); return <Scene style={{backgroundColor:"#102030"}}>
<Scene3D key="lit" style={{width:96,height:64}} pbr={{toneMapping:"__TONE__",exposure:0.25+time.seconds*0.6}}
 camera={{position:[0,0,5],target:[0,0,0],fov:45}}>
 <Mesh key="main" geometry={BALL} rotateY={time.seconds*90} position={[-0.4,0,0]}
  material={{type:"__KIND__",color:"#cf7338",metallic:0.1+time.seconds*0.4,roughness:0.0525+time.seconds*0.3,emissive:"#173421",emissiveIntensity:0.2}} />
 <Mesh key="mixed" geometry={BALL} scale={[-0.5,0.5,0.5]} position={[0.8,0,0.6]}
  material={{type:"unlit",color:"#3366ee",doubleSided:true}} />
 <AmbientLight color="#aabbcc" intensity={0.15}/>
 <HemisphereLight skyColor="#bad6ff" groundColor="#222c15" direction={[0.2,1,0]} intensity={0.3}/>
 <DirectionalLight direction={[0.8,0.2,1]} color="#ffe4c1" intensity={1.6}/>
 <DirectionalLight direction={[-1,0.2,1]} color="#7faaff" intensity={0.8}/>
</Scene3D></Scene>; }
"##.replace("__KIND__", kind).replace("__TONE__", if aces { "aces" } else { "none" });
    let source = if dof {
        source.replace(
            "fov:45",
            "fov:45,depthOfField:{focusDistance:5,maxBlurRadius:8}",
        )
    } else {
        source
    };
    let source = if advanced {
        source.replace("segments:16", "segments:8").replace("pbr={{toneMapping", "pbr={{shadows:true,toneMapping")
            .replace(" <AmbientLight",r##" <Mesh key="glass" geometry={BALL} position={[0.1+time.seconds*0.15,0,1.1]} scale={[0.45,0.45,0.45]}
 material={{type:"pbr",alphaMode:"blend",color:"#45aadd66",metallic:0.4,roughness:0.2}} />
 <AmbientLight"##)
    } else {
        source
    };
    project_from_source(&source)
}

#[test]
#[ignore = "requires real Metal; compares GPU Lambert/PBR with the CPU raster on the same compositor"]
fn lit_gpu_raster_matches_cpu_reference_through_seeks_and_cache_reset() {
    const DIR: &str = "VALLE_TEST_LIT_REFERENCE_DIR";
    let child = std::env::var_os(DIR);
    let directory = tempfile::tempdir().unwrap();
    if child.is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lit_gpu_raster_matches_cpu_reference_through_seeks_and_cache_reset",
                "--ignored",
                "--nocapture",
            ])
            .env("VALLE_SCENE3D_RASTER_CPU_REFERENCE", "1")
            .env(DIR, directory.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "CPU reference: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
    }
    let path = child
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().to_path_buf());
    for (width, height) in [(384, 256), (1920, 1080), (3840, 2160)] {
        let output = RenderSpec::new(
            width,
            height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
        )
        .unwrap();
        for (kind, aces) in [
            ("lambert", false),
            ("lambert", true),
            ("pbr", false),
            ("pbr", true),
        ] {
            let project = lit_project(kind, aces);
            let mut runner = project.frame_runner(SkiaBackendKind::Metal).unwrap();
            runner.render_rgba8(FrameKey::new(0), output).unwrap();
            let mut times = Vec::new();
            let mut observed = std::collections::BTreeMap::new();
            for (index, frame) in [0, 7, 20, 59, 12, 7, 0, 59].into_iter().enumerate() {
                if index == 5 {
                    runner.invalidate_surface_generation().unwrap();
                }
                let started = Instant::now();
                let (pixels, evidence) = runner.render_rgba8(FrameKey::new(frame), output).unwrap();
                if index > 0 && index < 5 {
                    times.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                assert_eq!(
                    evidence.execution.scene3d_gpu_rasters,
                    u64::from(child.is_none())
                );
                let file = path.join(format!("{width}x{height}-{kind}-{aces}-{frame}.rgba"));
                if child.is_some() {
                    std::fs::write(file, pixels.data).unwrap();
                    continue;
                }
                let expected = std::fs::read(file).unwrap();
                assert_eq!(pixels.data.len(), expected.len());
                let max_error = pixels
                    .data
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                let changed = pixels
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0.iter())
                    .filter(|(a, b)| a != b)
                    .count();
                eprintln!(
                    "lit {width}x{height} {kind} aces={aces} frame={frame}: changed={changed}, max_rgba8_error={max_error}"
                );
                let code_tolerance = if kind == "pbr" && !aces { 2 } else { 1 };
                if max_error > code_tolerance {
                    for (index, (actual, expected)) in pixels
                        .data
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected.as_chunks::<4>().0.iter())
                        .enumerate()
                        .filter(|(_, (a, b))| {
                            a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 1)
                        })
                        .take(8)
                    {
                        eprintln!(
                            "worst ({},{}): GPU={actual:?}, CPU={expected:?}",
                            index % width as usize,
                            index / width as usize
                        );
                    }
                }
                // Untonemapped GGX highlights can span two final SDR code values after
                // conversion/resampling; Lambert/ACES retain a one-code limit. Visibility
                // and replay stay exact, with an independent raw F16 limit of 0.001.
                assert!(
                    max_error <= code_tolerance,
                    "{kind}, aces={aces}, frame={frame}: {max_error}"
                );
                if let Some(prior) = observed.insert(frame, pixels.data.clone()) {
                    assert_eq!(prior, pixels.data);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "lit timing {width}x{height} {kind} aces={aces} cpu_reference={}: median={:.3}ms including final readback",
                child.is_some(),
                times[times.len() / 2]
            );
        }
    }
}

#[test]
#[ignore = "requires real Metal and VideoToolbox; checks lit GPU lifetime and byte admission"]
fn lit_shared_delivery_keeps_random_frames_exact_with_bounded_memory() {
    verify_lit_shared_delivery(false);
    verify_lit_shared_delivery(true);
}

fn verify_lit_shared_delivery(advanced: bool) {
    let project = lit_project_features("pbr", true, advanced, advanced);
    let (width, height) = if advanced { (480, 320) } else { (384, 256) };
    let directory = tempfile::tempdir().unwrap();
    let frames = [0, 7, 20, 59, 12, 7, 0, 59, 12, 20, 0, 7].map(FrameKey::new);
    let output = RenderSpec::new(
        width,
        height,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut expected = None;
    for workers in [1, 4] {
        let mut sink = InspectSink::new_sized(
            &directory.path().join(format!("lit-{workers}.mp4")),
            width,
            height,
        );
        let limit = if advanced {
            128 * 1024 * 1024
        } else {
            64 * 1024 * 1024
        };
        let budget = valle_render::host::NativeRenderBudget::new(workers, Some(limit)).unwrap();
        let control = RenderControl::default().with_budget(budget.clone());
        let report = render_schedule(
            &project,
            &frames,
            output,
            &mut sink,
            &control,
            None,
            SkiaBackendKind::Metal,
        )
        .unwrap();
        assert_eq!(
            report.scene3d_gpu_rasters,
            frames.len() as u64,
            "advanced={advanced} workers={workers} memory={:?}",
            budget.memory_report()
        );
        assert_eq!(report.scene3d_cpu_fallbacks, 0);
        assert_eq!(report.delivery_readback_bytes, 0);
        assert_eq!(report.delivery, Some(FrameDelivery::SharedGpu));
        assert!(sink.finished);
        assert_eq!(sink.keys, frames.map(|key| key.index()));
        assert!(budget.memory_report().peak_bytes <= limit);
        assert_eq!(budget.memory_report().used_bytes, 0);
        if let Some(expected) = &expected {
            assert_eq!(&sink.pixels, expected);
        } else {
            expected = Some(sink.pixels);
        }
    }
}

#[test]
#[ignore = "requires real Metal; verifies direct GPU color/depth -> Ganesh DOF without intermediate readback"]
fn gpu_geometry_dof_matches_cpu_geometry_dof() {
    const DIR: &str = "VALLE_TEST_DOF_REFERENCE_DIR";
    let child = std::env::var_os(DIR);
    let directory = tempfile::tempdir().unwrap();
    if child.is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "gpu_geometry_dof_matches_cpu_geometry_dof",
                "--ignored",
                "--nocapture",
            ])
            .env("VALLE_SCENE3D_RASTER_CPU_REFERENCE", "1")
            .env(DIR, directory.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "CPU reference: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let path = child
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().to_path_buf());
    let output = RenderSpec::new(
        384,
        256,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    for kind in ["unlit", "lambert", "pbr"] {
        let project = lit_project_with_dof(kind, true, true);
        let mut runner = project.frame_runner(SkiaBackendKind::Metal).unwrap();
        for frame in [0, 7, 20, 59, 12, 7] {
            let (pixels, evidence) = runner.render_rgba8(FrameKey::new(frame), output).unwrap();
            assert_eq!(
                evidence.execution.scene3d_gpu_rasters,
                u64::from(child.is_none())
            );
            let file = path.join(format!("{kind}-{frame}.rgba"));
            if child.is_some() {
                std::fs::write(file, pixels.data).unwrap();
                continue;
            }
            let expected = std::fs::read(file).unwrap();
            let maximum = pixels
                .data
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            eprintln!("GPU geometry DOF {kind} frame={frame}: max_rgba8_error={maximum}");
            assert!(maximum <= 2, "{kind} frame={frame}: {maximum}");
        }
    }
}

#[test]
#[ignore = "requires real Metal; checks combined PCF, OIT and GPU depth of field at delivery sizes"]
fn shadows_blend_and_dof_reach_delivery_without_intermediate_readback() {
    const DIR: &str = "VALLE_TEST_ADVANCED_REFERENCE_DIR";
    let child = std::env::var_os(DIR);
    let directory = tempfile::tempdir().unwrap();
    if child.is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shadows_blend_and_dof_reach_delivery_without_intermediate_readback",
                "--ignored",
                "--nocapture",
            ])
            .env("VALLE_SCENE3D_RASTER_CPU_REFERENCE", "1")
            .env(DIR, directory.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "CPU reference: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let path = child
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().to_path_buf());
    let project = lit_project_features("pbr", true, true, true);
    for (width, height) in [(480, 320), (1920, 1080), (3840, 2160)] {
        let spec = RenderSpec::new(
            width,
            height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
        )
        .unwrap();
        let mut runner = project.frame_runner(SkiaBackendKind::Metal).unwrap();
        let mut previous = None;
        for frame in [0, 20, 59, 20] {
            let (pixels, evidence) = runner.render_rgba8(FrameKey::new(frame), spec).unwrap();
            assert_eq!(
                evidence.execution.scene3d_gpu_rasters,
                u64::from(child.is_none())
            );
            let file = path.join(format!("{width}-{height}-{frame}.rgba"));
            if child.is_some() {
                std::fs::write(file, pixels.data).unwrap();
                continue;
            }
            let expected = std::fs::read(file).unwrap();
            let maximum = pixels
                .data
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            eprintln!(
                "combined shadows/blend/DOF {width}x{height} frame={frame}: max_rgba8_error={maximum}"
            );
            if maximum > 1 {
                let errors = pixels
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0)
                    .enumerate()
                    .filter(|(_, (a, b))| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 1))
                    .map(|(i, (a, b))| ((i % width as usize, i / width as usize), a, b))
                    .take(12)
                    .collect::<Vec<_>>();
                eprintln!("sample_errors={errors:?}");
            }
            assert!(maximum <= 1, "{width}x{height} frame={frame}: {maximum}");
            if frame == 20
                && let Some(p) = previous.replace(pixels.data)
            {
                assert_eq!(p, previous.as_ref().unwrap().as_slice());
            }
        }
    }
}

#[test]
#[ignore = "requires real Metal and VideoToolbox"]
fn shared_delivery_cancellation_and_sink_failure_drain_blocked_preparers() {
    for case in 0..3 {
        let advanced = case == 2;
        let project = if advanced {
            lit_project_features("pbr", true, true, true)
        } else {
            project_with_gpu_raster(case == 1)
        };
        let output = if advanced {
            RenderSpec::new(
                480,
                320,
                RenderQuality::Preview,
                OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
            )
            .unwrap()
        } else {
            spec()
        };
        let dir = tempfile::tempdir().unwrap();
        let frames = (0..24).map(FrameKey::new).collect::<Vec<_>>();
        for failure in [false, true] {
            let mut sink = InspectSink::new_sized(
                &dir.path().join(format!("failure-{failure}.mp4")),
                output.width(),
                output.height(),
            );
            sink.fail_at = failure.then_some(1);
            let budget = valle_render::host::NativeRenderBudget::new(
                4,
                Some(if advanced {
                    128 * 1024 * 1024
                } else {
                    16 * 1024 * 1024
                }),
            )
            .unwrap();
            let control = RenderControl::default().with_budget(budget.clone());
            let cancel = control.clone();
            let progress: valle_render::host::ProgressCallback = Arc::new(move |done, _| {
                if !failure && done == 1 {
                    cancel.cancel();
                }
            });
            let error = render_schedule(
                &project,
                &frames,
                output,
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
            assert_eq!(budget.memory_report().used_bytes, 0);
        }
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
