//! Opt-in hardware acceptance. Run outside the sandbox with
//! `cargo test -p valle-cli --test motion_gpu -- --ignored --test-threads=1`.
#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{io::BufReader, path::Path, process::Command};

#[cfg(target_os = "macos")]
const GPU: &str = "metal";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const GPU: &str = "vulkan";

fn render(dir: &Path, name: &str, source: &str, backend: &str) -> Vec<u8> {
    render_size(dir, name, source, backend, (640, 360))
}

fn render_size(dir: &Path, name: &str, source: &str, backend: &str, size: (u32, u32)) -> Vec<u8> {
    let gpu_stage = match name {
        name if name.contains("bloom") => Some("gpu-bloom"),
        "node-glow" => Some("gpu-glow"),
        name if name.starts_with("radial-") => Some("gpu-radial-blur"),
        name if name.starts_with("film-") => Some("gpu-film-grain"),
        name if name.starts_with("lens-") => Some("gpu-lens-distortion"),
        name if name.starts_with("scene3d-dof-") && GPU == "metal" => Some("gpu-scene3d-dof"),
        _ => None,
    };
    let input = dir.join(format!("{name}.motion.tsx"));
    std::fs::write(&input, source).unwrap();
    let output = dir.join(format!("{name}-{backend}.png"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    command.args([
        "motion",
        "render",
        input.to_str().unwrap(),
        "--frame",
        "30",
        "--backend",
        backend,
        "-o",
        output.to_str().unwrap(),
        "--json",
    ]);
    command.env_remove("VALLE_BLOOM_CPU_REFERENCE");
    command.env_remove("VALLE_GLOW_CPU_REFERENCE");
    command.env_remove("VALLE_RADIAL_BLUR_CPU_REFERENCE");
    command.env_remove("VALLE_FILM_GRAIN_CPU_REFERENCE");
    command.env_remove("VALLE_LENS_DISTORTION_CPU_REFERENCE");
    command.env_remove("VALLE_SCENE3D_DOF_CPU_REFERENCE");
    if gpu_stage.is_some() && backend == GPU {
        command.env("VALLE_TRACE_F16_STAGES", "1");
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{name} {backend}: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["delivery"]["backend"], backend);
    if backend == GPU {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(!trace.contains("Shader compilation error"), "{trace}");
        if let Some(stage) = gpu_stage {
            assert!(
                trace.contains(&format!("[valle f16] {stage}")),
                "{name}: {trace}"
            );
            assert!(!trace.contains("gpu-to-cpu"), "{name}: {trace}");
        }
    }
    let mut reader = png::Decoder::new(BufReader::new(std::fs::File::open(output).unwrap()))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), size);
    assert_eq!(info.color_type, png::ColorType::Rgba);
    pixels.truncate(info.buffer_size());
    pixels
}

#[test]
#[ignore = "requires a hardware GPU; run with --ignored"]
fn fullhd_bloom_stays_on_gpu_and_matches_raster() {
    let dir = tempfile::tempdir().unwrap();
    let source = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/scene-bloom.motion.tsx"
    )
    .replace("width: 640, height: 360", "width: 1920, height: 1080");
    let raster = render_size(dir.path(), "bloom-fullhd", &source, "raster", (1920, 1080));
    let gpu = render_size(dir.path(), "bloom-fullhd", &source, GPU, (1920, 1080));
    let worst = raster
        .iter()
        .zip(&gpu)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        worst <= 2,
        "scene-bloom 1080p Raster/GPU max channel diff {worst}"
    );
}

#[test]
#[ignore = "requires a hardware GPU; run with --ignored"]
fn shared_f16_filters_render_on_real_gpu() {
    let dir = tempfile::tempdir().unwrap();
    let glow =
        include_str!("../../valle-compiler/tests/fixtures/motion/composition/node-glow.motion.tsx");
    let no_glow = glow.replace(", filter: \"glow(24px 1.5 #22d3ee)\"", "");
    let radial = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/radial-blur.motion.tsx"
    );
    let clipped_radial = radial.replace("left: 280, top: 160", "left: 600, top: 160");
    let translucent_radial = radial.replace(
        "backgroundColor: \"#ffffff\", filter: \"radial-blur(40px 20px 20px)\"",
        "backgroundColor: \"#22d3ee\", opacity: 0.65, filter: \"radial-blur(39.5px 20.25px 23.5px)\"",
    );
    assert_ne!(clipped_radial, radial);
    assert_ne!(translucent_radial, radial);
    let lens = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/lens-distortion.motion.tsx"
    );
    let clipped_lens = lens.replace("left: 200, top: 100", "left: 520, top: 100");
    let fractional_lens = lens.replace("lens-distortion(-0.3 0)", "lens-distortion(-0.275 0.125)");
    let transparent_lens = lens.replace("backgroundColor: \"#303030\", filter", "filter");
    assert_ne!(clipped_lens, lens);
    assert_ne!(fractional_lens, lens);
    assert_ne!(transparent_lens, lens);
    let film = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/film-grain.motion.tsx"
    );
    let clipped_film = film.replace("left: 200, top: 100", "left: 580, top: 260");
    let colored_film = film.replace(
        "backgroundColor: \"#808080\", filter: \"film-grain(7 0.12 2px)\"",
        "backgroundColor: \"#22d3ee\", opacity: 0.45, filter: \"film-grain(4294967295 0.4 2.5px)\"",
    );
    assert_ne!(clipped_film, film);
    assert_ne!(colored_film, film);
    for (name, source) in [
        ("f01-base", no_glow.as_str()),
        ("node-glow", glow),
        (
            "scene-bloom",
            include_str!(
                "../../valle-compiler/tests/fixtures/motion/composition/scene-bloom.motion.tsx"
            ),
        ),
        (
            "node-bloom",
            include_str!(
                "../../valle-compiler/tests/fixtures/motion/composition/node-bloom.motion.tsx"
            ),
        ),
        ("radial-blur", radial),
        ("radial-clipped", clipped_radial.as_str()),
        ("radial-translucent", translucent_radial.as_str()),
        ("film-grain", film),
        ("film-clipped", clipped_film.as_str()),
        ("film-colored", colored_film.as_str()),
        ("lens-distortion", lens),
        ("lens-clipped", clipped_lens.as_str()),
        ("lens-fractional", fractional_lens.as_str()),
        ("lens-transparent", transparent_lens.as_str()),
    ] {
        let raster = render(dir.path(), name, source, "raster");
        let gpu = render(dir.path(), name, source, GPU);
        if name == "node-glow" {
            let pixel = |x: usize, y: usize| &gpu[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(380, 180)[1] >= 40, "glow halo is missing");
            assert!(pixel(480, 180)[1] <= 22, "glow extends past its radius");
        }
        if name == "radial-blur" {
            let pixel = |x: usize, y: usize| &gpu[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(370, 180)[0] >= 80, "radial blur is missing");
            assert!(
                pixel(380, 180)[0] <= 20,
                "radial blur extends past its amount"
            );
        }
        if name == "film-grain" {
            let pixel = |x: usize, y: usize| &gpu[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert_ne!(
                pixel(300, 180)[0],
                pixel(320, 180)[0],
                "film grain is missing"
            );
        }
        if name == "lens-distortion" {
            let pixel = |x: usize, y: usize| &gpu[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(248, 180)[0] >= 200, "lens distortion is missing");
            assert!(
                pixel(220, 180)[0] <= 60,
                "lens distortion exceeds its frame"
            );
        }
        let worst = raster
            .iter()
            .zip(&gpu)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 2, "{name}: Raster/GPU max channel diff {worst}");
    }
}

#[test]
#[ignore = "requires a hardware GPU; run with --ignored"]
fn fullhd_sphere_matches_raster_on_real_gpu() {
    let dir = tempfile::tempdir().unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-motion/tests/fixtures/scene3d/sphere.glb");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-compiler/tests/fixtures/motion/composition/sphere-scene3d.motion.tsx");
    let mut renders = Vec::new();
    for backend in ["raster", GPU] {
        let output = dir.path().join(format!("sphere-{backend}.png"));
        let result = Command::new(env!("CARGO_BIN_EXE_valle"))
            .args(["--json", "motion", "render"])
            .arg(&source)
            .arg("--asset")
            .arg(format!("model={}", model.display()))
            .args(["--backend", backend, "--frame", "0", "-o"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{backend}: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["delivery"]["backend"], backend);
        assert!(report["delivery"]["timing"]["renderMs"].as_f64().unwrap() > 0.0);
        let mut reader = png::Decoder::new(BufReader::new(std::fs::File::open(output).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (1920, 1080));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        let center = (540 * 1920 + 960) * 4;
        assert_ne!(&pixels[center..center + 4], &[0, 0, 0, 255]);
        renders.push(pixels);
    }
    let worst = renders[0].iter().zip(&renders[1]).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 2, "Scene3D CPU fallback upload changed pixels: {worst}");
}

#[test]
#[ignore = "requires a hardware GPU; run with --ignored"]
fn scene3d_dof_preserves_cpu_depth_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    // Near/far overlap, a focus-plane surface, translucent color, and clipped geometry.
    // All geometry is procedural, so this also exercises deferred Scene3D resource binding.
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 32 });
const PLANE = extrude(path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z"), { depth: 0.01 });
export default function DepthOfField() {
  return <Scene>
    <Scene3D key="depth" style={{ width: 640, height: 360 }}
      camera={{ position: [0,0,5], target: [0,0,0], fov: 45,
        depthOfField: { focusDistance: 5, maxBlurRadius: RADIUS } }}>
      <Mesh key="far" geometry={BALL} position={[0.3,0,-2]} material={{ type: "unlit", color: "#3366EE" }} />
      <Mesh key="focus" geometry={PLANE} position={[1.8,0,0]} scale={[0.45,0.7,1]} material={{ type: "unlit", color: "#33DD66" }} />
      <Mesh key="near" geometry={BALL} position={[-0.7,0,2]} scale={[0.55,0.55,0.55]} material={{ type: "unlit", color: "#FF553399" }} />
      <Mesh key="edge" geometry={BALL} position={[-3.6,0,0]} material={{ type: "unlit", color: "#FFFFFF" }} />
    </Scene3D>
  </Scene>;
}
"##;
    let sharp = render(
        dir.path(),
        "scene3d-sharp",
        &source.replace("RADIUS", "0"),
        "raster",
    );
    for radius in [0.5, 0.75, 6.0, 12.5, 16.0] {
        let source = source.replace("RADIUS", &radius.to_string());
        let name = format!("scene3d-dof-{radius}");
        let raster = render(dir.path(), &name, &source, "raster");
        let gpu = render(dir.path(), &name, &source, GPU);
        if radius >= 6.0 {
            assert_ne!(raster, sharp, "CPU depth of field did not change the image");
            assert_ne!(
                gpu, sharp,
                "GPU depth of field did not change the image"
            );
        }
        let worst = raster
            .iter()
            .zip(&gpu)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 2, "{name}: Raster/GPU max channel diff {worst}");
    }
}

#[test]
#[ignore = "requires a hardware GPU; run with --ignored"]
fn blur_shadow_and_transparent_delivery_match_raster() {
    let dir = tempfile::tempdir().unwrap();
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Filters() {
  return <Scene style={{ width: 640, height: 360 }}>
    <View style={{ position: "absolute", left: 70, top: 110, width: 120, height: 120,
      backgroundColor: "#22d3ee", filter: "blur(12px)", opacity: 0.65 }} />
    <View style={{ position: "absolute", left: 370, top: 110, width: 120, height: 120,
      backgroundColor: "#fb923c", boxShadow: "12px 8px 24px #22d3ee80" }} />
  </Scene>;
}
"##;
    let raster = render(dir.path(), "blur-shadow", source, "raster");
    let gpu = render(dir.path(), "blur-shadow", source, GPU);
    assert!(gpu.chunks_exact(4).any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    assert!(gpu.chunks_exact(4).any(|pixel| pixel[3] == 0));
    // Compare visible premultiplied colors: straight RGB at zero alpha is undefined.
    for (cpu, gpu) in raster.chunks_exact(4).zip(gpu.chunks_exact(4)) {
        assert!(cpu[3].abs_diff(gpu[3]) <= 2, "alpha: {cpu:?} {gpu:?}");
        for channel in 0..3 {
            let a = f32::from(cpu[channel]) * f32::from(cpu[3]) / 255.0;
            let b = f32::from(gpu[channel]) * f32::from(gpu[3]) / 255.0;
            // Two alpha codes plus one straight-color code can differ by three visible codes.
            assert!((a - b).abs() <= 3.0, "blur/shadow: {cpu:?} {gpu:?}");
        }
    }
}
