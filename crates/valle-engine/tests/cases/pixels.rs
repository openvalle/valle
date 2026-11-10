//! Compare rendered RGBA pixels with reviewed Valle and browser baselines.

use std::path::Path;

use valle_engine::gpu::{Gpu, Renderer};

use crate::case::{Case, Layout};
use crate::scene::Sample;

#[derive(serde::Serialize)]
pub struct Comparison {
    pub size: [u32; 2],
    pub checks: Vec<Check>,
    pub device: String,
}

#[derive(serde::Serialize)]
pub struct Check {
    pub source: String,
    pub required: bool,
    pub pixels: Difference,
    pub text_edges: Difference,
    pub passed: bool,
    pub measurement_error: Option<String>,
}

#[derive(Default, serde::Serialize)]
pub struct Difference {
    pub changed: usize,
    pub max: u8,
}

impl Comparison {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| !c.required || c.passed)
    }
}

pub fn run(
    root: &Path,
    dir: &Path,
    case: &Case,
    actual: &Sample,
    gpu: &Gpu,
    out: &Path,
) -> Result<Comparison, String> {
    let size = actual.size();
    let number = actual.number();
    let actual = draw(gpu, actual)?;
    png(&out.join("actual.png"), size, &actual)?;
    let mut checks = Vec::new();
    if let Some(valle) = &case.valle {
        let name = dir
            .strip_prefix(root.join("tests/cases"))
            .map_err(|e| e.to_string())?;
        let path = root
            .join("tests/baselines/valle")
            .join(name)
            .join(format!("{number}.png"));
        let baseline = read_png(&path, size).map_err(|e| {
            format!(
                "{}: {e}; review the rendered frame before accepting a baseline",
                path.display()
            )
        })?;
        std::fs::copy(&path, out.join("valle.png")).map_err(|e| e.to_string())?;
        let (pixels, text_edges, diff) = compare(&actual, &baseline, &[])?;
        png(&out.join("valle-diff.png"), size, &diff)?;
        checks.push(Check {
            source: "valle".into(),
            required: true,
            passed: valle.accepts(pixels.changed, pixels.max),
            pixels,
            text_edges,
            measurement_error: None,
        });
    }
    if let Some(browser) = &case.browser {
        let name = dir
            .strip_prefix(root.join("tests/cases"))
            .map_err(|e| e.to_string())?;
        let baseline = root.join("tests/baselines/browser").join(name);
        for name in crate::browser::NAMES {
            let capture = crate::browser::load(root, &baseline, name)?;
            if capture.size != size {
                return Err(format!(
                    "{name} canvas {:?} differs from actual {size:?}",
                    capture.size
                ));
            }
            let path = baseline.join(format!("{name}.png"));
            let reference = read_png(&path, size)?;
            std::fs::copy(path, out.join(format!("{name}.png"))).map_err(|e| e.to_string())?;
            let edges = browser.text_edges.as_ref();
            let mask = if let Some(edges) = edges {
                let layout = case
                    .layout
                    .iter()
                    .find(|l| l.key == edges.key)
                    .ok_or("textEdges has no layout")?;
                text_edge_mask(&reference, size, layout)?
            } else {
                Vec::new()
            };
            let (pixels, text_edges, diff) = compare(&actual, &reference, &mask)?;
            png(&out.join(format!("{name}-diff.png")), size, &diff)?;
            let measurement_error = capture.measurements.check(case).err();
            let passed = browser.pixels.accepts(pixels.changed, pixels.max)
                && edges.is_none_or(|e| e.pixels.accepts(text_edges.changed, text_edges.max))
                && measurement_error.is_none();
            checks.push(Check {
                source: name.into(),
                required: name == browser.selected,
                pixels,
                text_edges,
                passed,
                measurement_error,
            });
        }
    }
    Ok(Comparison {
        size,
        checks,
        device: gpu.name(),
    })
}

pub fn read_png(path: &Path, size: [u32; 2]) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("PNG is too large")?];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if [info.width, info.height] != size {
        return Err(format!("{}: wrong PNG size", path.display()));
    }
    pixels.truncate(info.buffer_size());
    match info.color_type {
        png::ColorType::Rgba => Ok(pixels),
        png::ColorType::Rgb => Ok(pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()),
        _ => Err(format!("{}: expected RGB or RGBA PNG", path.display())),
    }
}

fn draw(gpu: &Gpu, sample: &Sample) -> Result<Vec<u8>, String> {
    let [width, height] = sample.size();
    let mut renderer = Renderer::new(gpu, width, height);
    renderer.render(
        &sample.frame.items,
        &sample.frame.pictures,
        sample.scene.faces(),
        sample.scene.images(),
    )
}

// This opt-in mask is for text over a flat background. Derive it only from the reviewed
// browser image, never from the current render. Permit a one-pixel antialias fringe outside
// the content box, clipped to the padding box so the border can never enter the mask.
fn text_edge_mask(
    reference: &[u8],
    [width, height]: [u32; 2],
    layout: &Layout,
) -> Result<Vec<bool>, String> {
    let width = width as usize;
    let height = height as usize;
    if reference.len() != width * height * 4 {
        return Err("wrong mask image size".into());
    }
    let c = &layout.content;
    let p = &layout.padding;
    let left = (c.x - 1.0).max(p.x).max(0.0).ceil() as usize;
    let top = (c.y - 1.0).max(p.y).max(0.0).ceil() as usize;
    let right = (c.x + c.width + 1.0)
        .min(p.x + p.width)
        .min(width as f32)
        .floor()
        .max(0.0) as usize;
    let bottom = (c.y + c.height + 1.0)
        .min(p.y + p.height)
        .min(height as f32)
        .floor()
        .max(0.0) as usize;
    let pixels = reference.as_chunks::<4>().0;
    let mut mask = vec![false; width * height];
    for y in top..bottom {
        for x in left..right {
            let center = pixels[y * width + x];
            mask[y * width + x] =
                (y.saturating_sub(1).max(top)..=(y + 1).min(bottom - 1)).any(|ny| {
                    (x.saturating_sub(1).max(left)..=(x + 1).min(right - 1))
                        .any(|nx| pixels[ny * width + nx] != center)
                });
        }
    }
    Ok(mask)
}

pub fn compare(
    actual: &[u8],
    reference: &[u8],
    edges: &[bool],
) -> Result<(Difference, Difference, Vec<u8>), String> {
    if actual.len() != reference.len()
        || actual.is_empty()
        || !actual.len().is_multiple_of(4)
        || (!edges.is_empty() && edges.len() != actual.len() / 4)
    {
        return Err(
            "pixel buffers must have the same nonzero RGBA length and a matching mask".into(),
        );
    }
    let mut pixels = Difference::default();
    let mut text_edges = Difference::default();
    let mut diff = Vec::with_capacity(actual.len());
    for (i, (a, b)) in actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(reference.as_chunks::<4>().0)
        .enumerate()
    {
        let delta = a.iter().zip(b).map(|(&a, &b)| a.abs_diff(b)).max().unwrap();
        let edge = edges.get(i) == Some(&true);
        let stats = if edge { &mut text_edges } else { &mut pixels };
        stats.max = stats.max.max(delta);
        if delta == 0 {
            diff.extend_from_slice(&[255, 255, 255, 255]);
        } else {
            stats.changed += 1;
            // Amber: text-edge comparison. Magenta: the rest of the image.
            diff.extend_from_slice(if edge {
                &[240, 160, 0, 255]
            } else {
                &[220, 0, 100, 255]
            });
        }
    }
    Ok((pixels, text_edges, diff))
}

pub fn png(path: &Path, [width, height]: [u32; 2], pixels: &[u8]) -> Result<(), String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::create(path).map_err(|e| bad(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| bad(&e))?;
    writer.write_image_data(pixels).map_err(|e| bad(&e))?;
    writer.finish().map_err(|e| bad(&e))
}

#[test]
fn pixel_comparison_detects_color_alpha_and_one_pixel_shifts() {
    let green = [0, 170, 0, 255];
    let clear = [0, 0, 0, 0];
    let original = [green, clear].concat();
    assert_eq!(compare(&original, &original, &[]).unwrap().0.changed, 0);
    for actual in [[0, 169, 0, 255], [0, 170, 0, 254]] {
        let (pixels, _, diff) = compare(&[actual, clear].concat(), &original, &[]).unwrap();
        assert_eq!((pixels.changed, pixels.max), (1, 1));
        assert_eq!(&diff[..4], &[220, 0, 100, 255]);
    }
    assert_eq!(
        compare(&[clear, green].concat(), &original, &[])
            .unwrap()
            .0
            .changed,
        2
    );
    assert!(compare(&original, &green, &[]).is_err());
}

#[test]
fn text_tolerance_cannot_hide_solid_changes_missing_text_or_displacement() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let case = Case::read(&root.join("tests/cases/css/text/preserved-spaces/case.json")).unwrap();
    let browser = case.browser.as_ref().unwrap();
    let edges = browser.text_edges.as_ref().unwrap();
    let layout = &case.layout[0];
    let size = [360, 140];
    let reference = read_png(
        &root.join("tests/baselines/browser/css/text/preserved-spaces/chrome.png"),
        size,
    )
    .unwrap();
    let mask = text_edge_mask(&reference, size, layout).unwrap();
    let accepts = |actual: &[u8]| {
        let (pixels, text, _) = compare(actual, &reference, &mask).unwrap();
        browser.pixels.accepts(pixels.changed, pixels.max)
            && edges.pixels.accepts(text.changed, text.max)
    };
    assert!(accepts(&reference));
    let edge = mask.iter().position(|&edge| edge).unwrap();
    let mut minor = reference.clone();
    minor[edge * 4] = minor[edge * 4].saturating_sub(1);
    assert!(
        accepts(&minor),
        "a one-level glyph-edge difference is allowed"
    );
    for (x, y) in [(10, 10), (20, 24), (25, 30), (320, 100)] {
        let mut changed = reference.clone();
        changed[(y * 360 + x) * 4] -= 1;
        assert!(
            !accepts(&changed),
            "solid pixel ({x}, {y}) must remain exact"
        );
    }
    let background = [244, 247, 250, 255];
    for shift in [None, Some((1, 0)), Some((0, 1))] {
        let mut changed = reference.clone();
        for y in 33..103 {
            for x in 29..331 {
                let color = if let Some((dx, dy)) = shift {
                    let i = ((y - dy) * 360 + x - dx) * 4;
                    &reference[i..i + 4]
                } else {
                    &background
                };
                let i = (y * 360 + x) * 4;
                changed[i..i + 4].copy_from_slice(color);
            }
        }
        assert!(
            !accepts(&changed),
            "missing text or shifted glyphs {shift:?} must fail"
        );
    }
}

#[test]
fn valle_pixel_allowances_reject_more_changed_pixels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, size) in [
        ("tailwind/text", [320, 200]),
        ("tailwind/paragraphs", [320, 320]),
    ] {
        let case = Case::read(&root.join("tests/cases").join(name).join("case.json")).unwrap();
        let valle = case.valle.as_ref().unwrap();
        let limit = valle.max_pixels.unwrap();
        let path = root.join("tests/baselines/valle").join(name).join("0.png");
        let baseline = read_png(&path, size).unwrap();
        let accepts = |actual: &[u8]| {
            let (pixels, _, _) = compare(actual, &baseline, &[]).unwrap();
            valle.accepts(pixels.changed, pixels.max)
        };
        // Pixels changed one level each, at the most it allows and one more.
        let changed = |n: usize| {
            let mut changed = baseline.clone();
            for i in 0..n {
                changed[i * 4] ^= 1;
            }
            changed
        };
        assert!(accepts(&baseline));
        assert!(
            accepts(&changed(limit)),
            "{name}: {limit} pixels are allowed"
        );
        assert!(
            !accepts(&changed(limit + 1)),
            "{name}: a change across more than {limit} pixels must fail"
        );
        let mut far = baseline.clone();
        let step = valle.max_difference + 1;
        far[0] = if far[0] >= step {
            far[0] - step
        } else {
            far[0] + step
        };
        assert!(
            !accepts(&far),
            "{name}: one pixel past the largest difference must fail"
        );
    }
}

#[test]
fn css_and_svg_allowances_reject_missing_or_displaced_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "css/background/gradients",
        "css/border/styles-radius",
        "css/shadow/blur",
        "css/text/alignment",
        "css/text/mixed-runs",
        "css/text/overflow-wrap",
        "css/text/spacing-indent",
        "css/text/ellipsis",
        "css/text/word-wrap",
        "css/text/wrapping",
        "css/compositing/stacking-opacity",
        "css/filter/order",
        "css/filter/blur-overflow",
        "css/shadow/rounded-clip",
        "css/compositing/backdrop-blend/controls",
        "css/compositing/backdrop-blend/basic",
        "css/compositing/backdrop-blend/clips",
        "css/compositing/backdrop-blend/nested",
        "css/compositing/backdrop-blend/planes",
        "css/compositing/backdrop-blend/overflow",
        "svg/basic-shapes/inline",
        "svg/path-commands/inline",
        "svg/stroke-caps-joins/inline",
        "svg/markers/inline",
        "svg/image-sizing/file",
        "svg/gradient-sizing/file",
        "svg/gradient-coordinates/inline",
        "svg/radial-focus/inline",
        "svg/group-paint-opacity/inline",
    ] {
        let case = Case::read(&root.join("tests/cases").join(name).join("case.json")).unwrap();
        let browser = case.browser.as_ref().unwrap();
        let dir = root.join("tests/baselines/browser").join(name);
        let capture = crate::browser::load(&root, &dir, &browser.selected).unwrap();
        let reference =
            read_png(&dir.join(format!("{}.png", browser.selected)), capture.size).unwrap();
        let mask = browser.text_edges.as_ref().map_or_else(Vec::new, |edges| {
            let layout = case.layout.iter().find(|l| l.key == edges.key).unwrap();
            text_edge_mask(&reference, capture.size, layout).unwrap()
        });
        let [width, height] = capture.size.map(|n| n as usize);
        for shift in [None, Some((1, 0)), Some((0, 1))] {
            let mut changed = vec![255; reference.len()];
            if let Some((dx, dy)) = shift {
                for y in dy..height {
                    let from = (y - dy) * width * 4;
                    let to = (y * width + dx) * 4;
                    let len = (width - dx) * 4;
                    changed[to..to + len].copy_from_slice(&reference[from..from + len]);
                }
            }
            let (pixels, text, _) = compare(&changed, &reference, &mask).unwrap();
            let accepted = browser.pixels.accepts(pixels.changed, pixels.max)
                && browser
                    .text_edges
                    .as_ref()
                    .is_none_or(|edges| edges.pixels.accepts(text.changed, text.max));
            assert!(
                !accepted,
                "{name}: missing content or one-pixel displacement {shift:?} must fail"
            );
        }
    }
}

#[test]
fn combination_allowances_reject_missing_effects() {
    if !std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|layer| layer == "gpu")
    {
        return;
    }
    crate::with_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let gpu = valle_native::gpu::adapter(None)
            .and_then(|c| Gpu::with(c.adapter))
            .unwrap();
        for (name, from, to) in [
            (
                "css/filter/blur-overflow",
                "overflow: \"hidden\"",
                "overflow: \"visible\"",
            ),
            (
                "css/shadow/rounded-clip",
                "overflow: \"hidden\"",
                "overflow: \"visible\"",
            ),
            (
                "css/filter/order",
                "invert(1) brightness(0.5)",
                "brightness(0.5) invert(1)",
            ),
            (
                "css/mask/composite",
                "maskComposite: \"subtract\"",
                "maskComposite: \"add\"",
            ),
            (
                "css/compositing/stacking-opacity",
                "zIndex: 100",
                "zIndex: 0",
            ),
            (
                "css/compositing/backdrop-blend/basic",
                "mixBlendMode: \"multiply\"",
                "mixBlendMode: \"normal\"",
            ),
            (
                "css/compositing/backdrop-blend/basic",
                "backdropFilter: \"invert(1)\"",
                "backdropFilter: \"invert(0)\"",
            ),
            (
                "css/compositing/scene-blend-isolation",
                "transformStyle: \"preserve-3d\"",
                "transformStyle: \"flat\"",
            ),
            (
                "svg/group-paint-opacity/inline",
                "opacity=\"0.5\"",
                "opacity=\"1\"",
            ),
        ] {
            let dir = root.join("tests/cases").join(name);
            let source = std::fs::read_to_string(dir.join("scene.tsx")).unwrap();
            let changed = source.replacen(from, to, 1);
            assert_ne!(source, changed, "mutation must change {name}");
            let mut scene = valle_engine::scene::Scene::load(
                "probe.tsx",
                &|path| (path == "probe.tsx").then(|| changed.as_bytes().to_vec()),
                &valle_engine::front::Inputs::default(),
                std::rc::Rc::new(|_| Err("these geometric probes do not use fonts".into())),
            )
            .unwrap();
            let at = valle_engine::time::Tick::ZERO;
            let frame = scene.frame(at).unwrap();
            let sample = Sample { at, scene, frame };
            let actual = draw(&gpu, &sample).unwrap();
            let case = Case::read(&dir.join("case.json")).unwrap();
            let browser = case.browser.unwrap();
            let baseline = read_png(
                &root
                    .join("tests/baselines/browser")
                    .join(name)
                    .join(format!("{}.png", browser.selected)),
                sample.size(),
            )
            .unwrap();
            let (pixels, _, _) = compare(&actual, &baseline, &[]).unwrap();
            assert!(
                !browser.pixels.accepts(pixels.changed, pixels.max),
                "{name}: changing {from} to {to} must fail the reviewed allowance"
            );
        }
    });
}

#[test]
fn ellipsis_allowance_rejects_clipped_text_without_the_marker() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let case = Case::read(&root.join("tests/cases/css/text/ellipsis/case.json")).unwrap();
    let browser = case.browser.as_ref().unwrap();
    let dir = root.join("tests/baselines/browser/css/text/ellipsis");
    let capture = crate::browser::load(&root, &dir, &browser.selected).unwrap();
    let reference = read_png(&dir.join(format!("{}.png", browser.selected)), capture.size).unwrap();
    let find = |key| &case.layout.iter().find(|l| l.key == key).unwrap().border;
    let clip = find("clip");
    let ellipsis = find("ellipsis");
    assert_eq!([clip.width, clip.height], [ellipsis.width, ellipsis.height]);
    let width = capture.size[0] as usize;
    let mut changed = reference.clone();
    // Replace only the ellipsis box with its authored text-overflow: clip control.
    for y in 0..clip.height as usize {
        let from = ((clip.y as usize + y) * width + clip.x as usize) * 4;
        let to = ((ellipsis.y as usize + y) * width + ellipsis.x as usize) * 4;
        let len = clip.width as usize * 4;
        changed[to..to + len].copy_from_slice(&reference[from..from + len]);
    }
    let (pixels, _, _) = compare(&changed, &reference, &[]).unwrap();
    assert!(
        !browser.pixels.accepts(pixels.changed, pixels.max),
        "glyph antialiasing tolerance must not hide a missing overflow marker"
    );
}

#[test]
fn motion_baselines_reject_wrong_interpolation_echo_and_exposure() {
    if !std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|layer| layer == "gpu")
    {
        return;
    }
    crate::with_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let gpu = valle_native::gpu::adapter(None)
            .and_then(|c| Gpu::with(c.adapter))
            .unwrap();
        for (name, at, number, from, to) in [
            ("interpolate", "0.5", 2, "[64, 160]", "[64, 156]"),
            ("echo", "1", 4, "delay={0.25}", "delay={0.125}"),
            ("echo", "1", 4, "decay={0.5}", "decay={0.75}"),
            ("shutter", "0.2", 6, "angle={180}", "angle={0}"),
            ("shutter", "0.2", 6, "fps: 30", "fps: 60"),
        ] {
            let source =
                std::fs::read_to_string(root.join(format!("tests/cases/motion/{name}/scene.tsx")))
                    .unwrap();
            let changed = source.replacen(from, to, 1);
            assert_ne!(source, changed, "mutation must change {name}");
            let mut scene = valle_engine::scene::Scene::load(
                "probe.tsx",
                &|path| (path == "probe.tsx").then(|| changed.as_bytes().to_vec()),
                &valle_engine::front::Inputs::default(),
                std::rc::Rc::new(|_| Err("these geometric probes do not use fonts".into())),
            )
            .unwrap();
            let at_tick = valle_engine::time::Tick::parse_seconds(at).unwrap();
            let frame = scene.frame(at_tick).unwrap();
            let sample = Sample {
                at: at_tick,
                scene,
                frame,
            };
            let actual = draw(&gpu, &sample).unwrap();
            let baseline = read_png(
                &root.join(format!("tests/baselines/valle/motion/{name}/{number}.png")),
                sample.size(),
            )
            .unwrap();
            let (difference, _, _) = compare(&actual, &baseline, &[]).unwrap();
            assert!(
                difference.max > 1,
                "{name}: changing {from} to {to} must fail the baseline check"
            );
        }
    });
}
