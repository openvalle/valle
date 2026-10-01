//! Opt-in hardware acceptance. Run outside the sandbox with
//! `VALLE_TEST_NATIVE_BACKEND=metal cargo test -p valle-cli --test motion_metal`.
#![cfg(target_os = "macos")]

use std::{io::BufReader, path::Path, process::Command};

fn render(dir: &Path, name: &str, source: &str, backend: &str) -> Vec<u8> {
    render_size(dir, name, source, backend, (640, 360))
}

fn render_size(dir: &Path, name: &str, source: &str, backend: &str, size: (u32, u32)) -> Vec<u8> {
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
    if (name.starts_with("bloom")
        || name == "node-glow"
        || name.starts_with("radial-filter")
        || name.starts_with("film-filter")
        || name.starts_with("lens-filter"))
        && backend == "metal"
    {
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
    if backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(!trace.contains("Shader compilation error"), "{trace}");
    }
    if name.starts_with("bloom") && backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(trace.contains("[valle f16] gpu-bloom"), "{trace}");
        assert!(!trace.contains("gpu-to-cpu"), "{trace}");
    }
    if name == "node-glow" && backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(trace.contains("[valle f16] gpu-glow"), "{trace}");
        assert!(!trace.contains("gpu-to-cpu"), "{trace}");
    }
    if name.starts_with("radial-filter") && backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(trace.contains("[valle f16] gpu-radial-blur"), "{trace}");
        assert!(!trace.contains("gpu-to-cpu"), "{trace}");
    }
    if name.starts_with("film-filter") && backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(trace.contains("[valle f16] gpu-film-grain"), "{trace}");
        assert!(!trace.contains("gpu-to-cpu"), "{trace}");
    }
    if name.starts_with("lens-filter") && backend == "metal" {
        let trace = String::from_utf8_lossy(&result.stderr);
        assert!(trace.contains("[valle f16] gpu-lens-distortion"), "{trace}");
        assert!(!trace.contains("gpu-to-cpu"), "{trace}");
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
fn fullhd_bloom_stays_on_metal_and_matches_raster() {
    if std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() != Ok("metal") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = include_str!(
        "../../valle-compiler/tests/fixtures/motion/composition/scene-bloom.motion.tsx"
    )
    .replace("width: 640, height: 360", "width: 1920, height: 1080");
    let raster = render_size(dir.path(), "bloom-fullhd", &source, "raster", (1920, 1080));
    let metal = render_size(dir.path(), "bloom-fullhd", &source, "metal", (1920, 1080));
    let worst = raster
        .iter()
        .zip(&metal)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        worst <= 2,
        "scene-bloom 1080p Raster/Metal max channel diff {worst}"
    );
}

#[test]
fn shared_f16_filters_render_on_real_metal() {
    if std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() != Ok("metal") {
        return;
    }
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
        let metal = render(dir.path(), name, source, "metal");
        if name == "node-glow" {
            let pixel = |x: usize, y: usize| &metal[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(380, 180)[1] >= 40, "glow halo is missing");
            assert!(pixel(480, 180)[1] <= 22, "glow extends past its radius");
        }
        if name == "radial-blur" {
            let pixel = |x: usize, y: usize| &metal[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(370, 180)[0] >= 80, "radial blur is missing");
            assert!(
                pixel(380, 180)[0] <= 20,
                "radial blur extends past its amount"
            );
        }
        if name == "film-grain" {
            let pixel = |x: usize, y: usize| &metal[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert_ne!(
                pixel(300, 180)[0],
                pixel(320, 180)[0],
                "film grain is missing"
            );
        }
        if name == "lens-distortion" {
            let pixel = |x: usize, y: usize| &metal[(y * 640 + x) * 4..(y * 640 + x) * 4 + 4];
            assert!(pixel(248, 180)[0] >= 200, "lens distortion is missing");
            assert!(
                pixel(220, 180)[0] <= 60,
                "lens distortion exceeds its frame"
            );
        }
        let worst = raster
            .iter()
            .zip(&metal)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 2, "{name}: Raster/Metal max channel diff {worst}");
    }
}

#[test]
fn fullhd_sphere_matches_raster_on_real_metal() {
    if std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() != Ok("metal") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-motion/tests/fixtures/scene3d/sphere.glb");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../valle-compiler/tests/fixtures/motion/composition/sphere-scene3d.motion.tsx");
    let mut renders = Vec::new();
    for backend in ["raster", "metal"] {
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
    assert_eq!(renders[0], renders[1]);
}
