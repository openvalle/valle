//! Authored WGSL rendered by the real GPU: resource bindings, coordinates, animated uniforms,
//! straight alpha and child/backdrop reads. Expected pixels come from these operations, not
//! from a capture of the implementation. The Web player renders the same fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

fn image(path: &Path) -> (usize, Vec<u8>) {
    let mut decoder =
        png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    bytes.truncate(info.buffer_size());
    (info.width as usize, bytes)
}

fn render(source: &Path, output: &Path, frames: &str) {
    let kind = if source.extension().is_some_and(|s| s == "json") {
        "timeline"
    } else {
        "motion"
    };
    let result = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args([kind, "render"])
        .arg(source)
        .args(["--frames", frames, "-o"])
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn wgsl_effects_and_transitions_render_their_declared_inputs() {
    if !std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|s| s == "gpu")
    {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = root.join("tests/resources/shaders");
    let out = std::env::temp_dir().join(format!("valle-wgsl-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    render(
        &fixtures.join("scene.motion.tsx"),
        &out.join("frames"),
        "0,1,2,3",
    );
    let (grid_w, grid) = image(&root.join("tests/resources/svg/pixels.png"));
    for frame in 0..4 {
        let (w, pixels) = image(&out.join(format!("frames/frame-{frame:05}.png")));
        let check = |x, y, expected: [u8; 4]| {
            let actual = &pixels[4 * (y * w + x)..4 * (y * w + x + 1)];
            assert!(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "frame {frame} ({x},{y}): {actual:?} != {expected:?}"
            );
        };
        check(16, 8, [255, 0, 0, 255]);
        check(16, 24, [0, 0, 255, 255]);
        // Pixel centers of the external 4×2 grid, still top-to-bottom after extraction.
        for y in 0..2 {
            for x in 0..4 {
                check(
                    64 + 16 * x + 7,
                    16 * y + 7,
                    grid[4 * (y * grid_w + x)..4 * (y * grid_w + x + 1)]
                        .try_into()
                        .unwrap(),
                );
            }
        }
        let progress = frame as f64 / 4.0;
        check(32, 48, [(255.0 * progress * 0.5).round() as u8, 62, 0, 255]);
        check(96, 48, [223, 191, 127, 255]);
        check(
            32,
            80,
            [
                (255.0 * (1.0 - progress)).round() as u8,
                0,
                (255.0 * progress).round() as u8,
                255,
            ],
        );
    }
    render(
        &fixtures.join("grade.timeline.json"),
        &out.join("grade.png"),
        "0",
    );
    let (w, pixels) = image(&out.join("grade.png"));
    let at = 4 * (16 * w + 32);
    assert!(
        pixels[at..at + 4]
            .iter()
            .zip([76u8, 96, 108, 255])
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    render(
        &fixtures.join("transitions.motion.tsx"),
        &out.join("transitions"),
        "0,1,2,3,4",
    );
    // Each built-in has the source/destination endpoint even with its default parameters.
    let (_, first) = image(&out.join("transitions/frame-00000.png"));
    let (_, last) = image(&out.join("transitions/frame-00004.png"));
    for cell in 0..13 {
        let x = (cell % 5) * 104 + 18;
        let y = (cell / 5) * 76 + 22;
        let at = 4 * (y * 520 + x);
        assert_eq!(
            &first[at..at + 4],
            &[249, 230, 46, 255],
            "transition {cell} start"
        );
        assert_eq!(
            &last[at..at + 4],
            &[255, 255, 255, 255],
            "transition {cell} end"
        );
    }
    for frame in 1..4 {
        let (_, pixels) = image(&out.join(format!("transitions/frame-{frame:05}.png")));
        let progress = frame as f64 / 4.0;
        let check = |cell: usize, x: usize, y: usize, expected: [u8; 4]| {
            let at = 4 * ((cell / 5 * 76 + 2 + y) * 520 + cell % 5 * 104 + 2 + x);
            let actual = &pixels[at..at + 4];
            assert!(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "transition {cell}, frame {frame}, ({x},{y}): {actual:?} != {expected:?}"
            );
        };
        // Both children are solid at (16,20): yellow and white. Fade interpolates their
        // encoded sRGB channels; multiply reaches yellow × white at its midpoint.
        for (cell, amount) in [(0, progress), (11, (2.0 * progress - 1.0).max(0.0))] {
            check(
                cell,
                16,
                20,
                [249.0, 230.0, 46.0, 255.0].map(|a| (a + (255.0 - a) * amount).round() as u8),
            );
        }
        // Wipes select unchanged child pixels on either side of a moving edge. Check both
        // directions and three positions, away from the antialiased edge itself.
        for cell in [1, 2] {
            for x in [12, 50, 88] {
                let to = if cell == 1 {
                    (x as f64 + 0.5) / 100.0 > 1.0 - progress
                } else {
                    (x as f64 + 0.5) / 100.0 < progress
                };
                let at = 4 * (62 * 520 + cell * 104 + 2 + x);
                let endpoint = if to { &last } else { &first };
                check(cell, x, 60, endpoint[at..at + 4].try_into().unwrap());
            }
        }
        // At these progress values the circle has revealed its center but not its corner.
        for (x, y, endpoint) in [(50, 36, &last), (1, 1, &first)] {
            let at = 4 * ((2 + y) * 520 + 3 * 104 + 2 + x);
            check(3, x, y, endpoint[at..at + 4].try_into().unwrap());
        }
    }
    std::fs::remove_dir_all(out).unwrap();
}
