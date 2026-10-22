//! Motion delivery: lossless alpha, exact frame selection and real compile counts.
use serde_json::{Value, json};
use std::{
    io::BufReader,
    path::Path,
    process::{Command, Output},
};
use valle_media::{codec::decode_rgba_frames, frame::RgbaFrame};

const ALPHA: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/transparent-video-scene.motion.tsx"
);
const MULTI: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/multi-frame-motion-scene.motion.tsx"
);
const LAZY_EVALUATION: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/lazy-evaluation.motion.tsx"
);
const INSTANCE_TEMPLATE: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx"
);
const CIRCLE_INSTANCE_TEMPLATE: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/circle-instances.motion.tsx"
);
const PATH_INSTANCE_TEMPLATE: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/path-instances.motion.tsx"
);
const TEXT_OUTLINE: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/text-outline-drawing.motion.tsx"
);
const RANGE_SELECTOR: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/text-range-selector.motion.tsx"
);
const EXTRUDED_TEXT: &str =
    include_str!("../../valle-compiler/tests/fixtures/motion/composition/extruded-text.motion.tsx");
const CONTACT: &str = include_str!(
    "../../valle-compiler/tests/fixtures/motion/composition/contact-warning.motion.tsx"
);

const COST: &str = r##"export const composition = { width: 64, height: 36, fps: 4, duration: 1 };
export default function Cost() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 4);
  return (
    <Scene className="relative h-full w-full">
      <View key="card" className="absolute" style={{ left: 8, top: 8, width: 16 + frame * 8,
        height: 20, backgroundColor: "#ff8040", filter: "blur(2px)" }} />
    </Scene>
  );
}
"##;

const COST_TRANSITION: &str = r##"export const composition = { width: 64, height: 36, fps: 4, duration: 1 };
export default function Blur() { const time = useCurrentTime();
  return <Transition kind="linearBlur" progress={time.progress} style={{ width: 64, height: 36 }}>
    <View style={{ backgroundColor: "#2140ff" }} />
    <View style={{ backgroundColor: "#ff4a24" }} />
  </Transition>;
}
"##;

const COST_SHADER: &str = r##"export const controls = { assets: { effect: asset({ kind: "shader" }) } };
export const composition = { width: 64, height: 36, fps: 4, duration: 1 };
export default function Work() { const comp = useComposition();
  const size = { width: comp.width, height: comp.height };
  return <Scene style={size}><ShaderLayer source="asset://effect" style={size}>NESTED</ShaderLayer></Scene>;
}
"##;

const BUDGET_TEXT: &str = r##"export const composition = { width: 64, height: 36, fps: 4, duration: 1 };
export default function Title() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Text className="absolute" style={{ left: 4, top: 4, fontSize: 20, color: "#ffffff" }}>VALLE</Text>
  </Scene>;
}
"##;

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn success(result: Output) -> Value {
    assert!(
        result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn motion_check_reports_tangent_contact_without_failing_and_respects_strict_policy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("contact.motion.tsx"), CONTACT).unwrap();
    let output = run(
        dir.path(),
        &["motion", "check", "contact.motion.tsx", "--json"],
    );
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "ok");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("[warning:MorphContact]"), "{stderr}");
    assert!(stderr.contains("0.500000000000"), "{stderr}");

    std::fs::write(
        dir.path().join("contact.motion.tsx"),
        CONTACT.replace("contactPolicy: \"warn\"", "contactPolicy: \"error\""),
    )
    .unwrap();
    let strict = run(
        dir.path(),
        &["motion", "check", "contact.motion.tsx", "--json"],
    );
    assert!(!strict.status.success());
    let rejected: Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "motion_compile_failed");
}

fn png(path: &Path) -> RgbaFrame {
    let mut reader = png::Decoder::new(BufReader::new(std::fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    data.truncate(info.buffer_size());
    RgbaFrame {
        width: info.width,
        height: info.height,
        data,
    }
}

fn assert_same_pixels(first: &RgbaFrame, second: &RgbaFrame, label: &str) {
    assert_eq!((first.width, first.height), (second.width, second.height));
    if let Some((offset, (actual, expected))) = first
        .data
        .iter()
        .zip(&second.data)
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected)
    {
        let pixel = offset / 4;
        panic!(
            "{label}: pixel ({}, {}) channel {}: {actual} != {expected}",
            pixel % first.width as usize,
            pixel / first.width as usize,
            offset % 4,
        );
    }
}

fn assert_cell(sheet: &RgbaFrame, cell: &RgbaFrame, index: usize, columns: usize) {
    let x = (index % columns) * cell.width as usize;
    let y = (index / columns) * cell.height as usize;
    for row in 0..cell.height as usize {
        let start = ((y + row) * sheet.width as usize + x) * 4;
        let source = row * cell.width as usize * 4;
        let length = cell.width as usize * 4;
        assert_eq!(
            &sheet.data[start..start + length],
            &cell.data[source..source + length],
            "cell {index}, row {row}"
        );
    }
}

#[test]
fn text_outline_is_extruded_into_rotating_lit_scene_geometry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("extruded.motion.tsx"), EXTRUDED_TEXT).unwrap();
    let mut frames = Vec::new();
    for frame in [0, 15] {
        let name = format!("extruded-{frame}.png");
        let report = success(run(
            dir.path(),
            &[
                "motion",
                "render",
                "extruded.motion.tsx",
                "--frame",
                &frame.to_string(),
                "--backend",
                "raster",
                "-o",
                &name,
                "--json",
            ],
        ));
        assert_eq!(report["delivery"]["backend"], "raster");
        assert_eq!(report["compilations"], 1);
        frames.push(png(&dir.path().join(name)));
    }
    let mut widths = Vec::new();
    for image in &frames {
        let background = &image.data[..4];
        let mut center_ink = 0;
        let mut min_x = image.width as usize;
        let mut max_x = 0;
        let mut blue_levels = std::collections::BTreeMap::<u8, usize>::new();
        for y in 0..image.height as usize {
            for x in 0..image.width as usize {
                let at = (y * image.width as usize + x) * 4;
                let pixel = &image.data[at..at + 4];
                if pixel != background {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    *blue_levels.entry(pixel[2]).or_default() += 1;
                    if (270..370).contains(&x) && (130..230).contains(&y) {
                        center_ink += 1;
                    }
                }
            }
        }
        assert!(
            center_ink >= 1_000,
            "extruded-text center ink: {center_ink}"
        );
        let dominant = blue_levels
            .into_iter()
            .filter(|(_, count)| *count >= 100)
            .map(|(level, _)| level)
            .collect::<Vec<_>>();
        assert!(
            dominant
                .iter()
                .any(|a| dominant.iter().any(|b| a.abs_diff(*b) >= 20)),
            "extruded-text requires differently lit front and side faces: {dominant:?}"
        );
        widths.push(max_x - min_x + 1);
    }
    assert_ne!(
        widths[0], widths[1],
        "extruded-text rotation must change silhouette width"
    );
}

#[test]
fn text_outline_draws_on_and_matches_real_text_ink() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("outline.motion.tsx"), TEXT_OUTLINE).unwrap();
    let render = |source: &str, frame: &str, output: &str| {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frame",
                frame,
                "-o",
                output,
                "--backend",
                "raster",
                "--json",
            ],
        ));
        png(&dir.path().join(output))
    };
    let bright = |frame: &RgbaFrame| {
        frame
            .data
            .chunks_exact(4)
            .filter(|pixel| pixel[..3].iter().copied().max().unwrap() > 80)
            .count()
    };
    let counts = ["0", "6", "30", "54", "59"]
        .iter()
        .map(|frame| {
            let output = format!("outline-{frame}.png");
            bright(&render("outline.motion.tsx", frame, &output))
        })
        .collect::<Vec<_>>();
    assert!(counts[0] < 50, "{counts:?}");
    assert!(counts[1] < counts[2] && counts[2] < counts[3], "{counts:?}");

    let control = r##"export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Control() {
  return <Scene className="relative h-full w-full" style={{backgroundColor:"#101010"}}>
    <Text className="absolute" style={{left:60,top:80,fontSize:150,fontWeight:800,color:"transparent",
      WebkitTextStrokeWidth:3,WebkitTextStrokeColor:"#ffffff"}}>VALLE</Text>
  </Scene>;
}"##;
    std::fs::write(dir.path().join("control.motion.tsx"), control).unwrap();
    let control = render("control.motion.tsx", "0", "control.png");
    let control_count = bright(&control);
    assert!(
        (counts[4] as f64 - control_count as f64).abs() / control_count as f64 <= 0.1,
        "outline {} vs Text stroke {control_count}",
        counts[4],
    );

    let filled_outline = TEXT_OUTLINE.replace(
        "fill=\"none\" stroke=\"#ffffff\" strokeWidth={3} trimEnd={time.progress}",
        "fill=\"#ffffff\"",
    );
    std::fs::write(dir.path().join("filled-outline.motion.tsx"), filled_outline).unwrap();
    let filled_text = r##"export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Control() {
  return <Scene className="relative h-full w-full" style={{backgroundColor:"#101010"}}>
    <Text className="absolute" style={{left:60,top:80,fontSize:150,fontWeight:800,color:"#ffffff"}}>VALLE</Text>
  </Scene>;
}"##;
    std::fs::write(dir.path().join("filled-text.motion.tsx"), filled_text).unwrap();
    let path = render("filled-outline.motion.tsx", "0", "filled-outline.png");
    let text = render("filled-text.motion.tsx", "0", "filled-text.png");
    assert_same_pixels(&path, &text, "filled glyph outline and Text");
}

#[test]
fn range_selector_feathers_one_letter_with_unit_local_blur() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("selector.motion.tsx"), RANGE_SELECTOR).unwrap();
    let no_blur = RANGE_SELECTOR.replace(
        "blur: 8 * (1 - rangeSelector(unit, { start: 0, end: time.progress, softness: 0.15, shape: \"ramp\" }))",
        "blur: 0",
    );
    let full = no_blur.replace(
        "opacity: rangeSelector(unit, { start: 0, end: time.progress, softness: 0.15, shape: \"ramp\" })",
        "opacity: 1",
    );
    std::fs::write(dir.path().join("no-blur.motion.tsx"), no_blur).unwrap();
    std::fs::write(dir.path().join("full.motion.tsx"), full).unwrap();
    let render = |source: &str, output: &str| {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frame",
                "30",
                "-o",
                output,
                "--backend",
                "raster",
                "--json",
            ],
        ));
        png(&dir.path().join(output))
    };
    let selected = render("selector.motion.tsx", "selected.png");
    let sharp = render("no-blur.motion.tsx", "sharp.png");
    let reference = render("full.motion.tsx", "reference.png");
    let red = |frame: &RgbaFrame, x: usize, y: usize| frame.data[(y * 640 + x) * 4];
    let brightest = |frame: &RgbaFrame, from: usize, to: usize| {
        (from..to)
            .flat_map(|x| (130..250).map(move |y| red(frame, x, y)))
            .max()
            .unwrap()
    };
    assert!(brightest(&selected, 40, 201) > 200);
    assert!(brightest(&selected, 460, 601) < 60);

    let mut fonts = valle_motion::Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let viewport = valle_motion::Viewport::new((640, 360));
    let prefix = |text: &str| {
        valle_motion::measure_text(
            &valle_motion::TextMeasure {
                text,
                font_size: 90.0,
                font_weight: Some(800.0),
                ..Default::default()
            },
            &fonts,
            viewport,
        )
        .unwrap()
        .width
    };
    // Measure the four-letter prefix and fifth-letter endpoint with exactly the authored font.
    let front_left = (40.0 + prefix("SELE")).floor() as usize;
    let front_right = (40.0 + prefix("SELEC")).ceil() as usize;
    let peak = brightest(&selected, front_left, front_right);
    let reference_peak = brightest(&reference, front_left, front_right);
    let relative_opacity = f64::from(peak.saturating_sub(16)) / f64::from(reference_peak - 16);
    assert!(
        (0.2..0.8).contains(&relative_opacity),
        "front opacity {relative_opacity}, x={front_left}..{front_right}"
    );

    // At the C's leading edge, count the horizontal midtone transition at a shared scanline.
    // The same per-letter alpha without blur must remain a sharp edge.
    let band = |frame: &RgbaFrame, left: usize| {
        (left.saturating_sub(12)..left + 14)
            .filter(|&x| (40..=160).contains(&red(frame, x, 190)))
            .count()
    };
    let blurred_band = band(&selected, front_left);
    let sharp_band = band(&sharp, front_left);
    assert!(
        blurred_band >= 4 && sharp_band <= 2,
        "front edge: blurred={blurred_band}, sharp={sharp_band}, x={front_left}"
    );

    let revealed_left = (40.0 + prefix("SEL")).floor() as usize;
    let revealed_band = (revealed_left.saturating_sub(3)..revealed_left + 5)
        .filter(|&x| (40..=220).contains(&red(&selected, x, 190)))
        .count();
    assert!(
        revealed_band <= 2,
        "revealed edge band {revealed_band}, x={revealed_left}"
    );
}

#[test]
fn check_reports_frame_activation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lazy.motion.tsx"), LAZY_EVALUATION).unwrap();
    let report = success(run(
        dir.path(),
        &[
            "motion",
            "check",
            "lazy.motion.tsx",
            "--frame",
            "1",
            "--json",
        ],
    ));
    assert_eq!(report["frame"], 1);
    assert_eq!(report["nodes"], 2);
    assert_eq!(report["templates"], 1);
    assert_eq!(report["instanceRows"], 2000);
    assert_eq!(report["activeNodes"], 2);
    assert_eq!(report["layoutNodes"], 2);
    assert!(
        report["evaluatedExpressions"].as_u64().unwrap() * 5
            < report["fullEvaluationExpressions"].as_u64().unwrap()
    );
}

/// `motion check --cost` plans every output frame without rendering it: a row per frame, the
/// frames with the most planned pixels first with their largest passes, and four times the pixels
/// when the delivery size doubles.
#[test]
fn check_cost_plans_every_frame_and_scales_with_the_output_size() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cost.motion.tsx"), COST).unwrap();
    let check = |size: Option<&str>| {
        let mut args = vec!["motion", "check", "cost.motion.tsx", "--cost", "--json"];
        if let Some(size) = size {
            args.extend(["--output-size", size]);
        }
        success(run(dir.path(), &args))
    };
    let report = check(None);
    let cost = &report["cost"];
    assert_eq!(
        (cost["width"].as_u64(), cost["height"].as_u64()),
        (Some(64), Some(36))
    );
    let frames = cost["frames"].as_array().unwrap();
    let index = |frame: &Value| frame["frame"].as_u64().unwrap();
    let pixels = |frame: &Value| frame["pixels"].as_u64().unwrap();
    assert_eq!(frames.iter().map(index).collect::<Vec<_>>(), [0, 1, 2, 3]);
    // The card widens every frame, and so does the work its blur plans.
    assert!(
        frames
            .windows(2)
            .all(|pair| pixels(&pair[0]) < pixels(&pair[1])),
        "{frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame["peakBytes"].as_u64().unwrap() > 0)
    );
    let hot = cost["hotFrames"].as_array().unwrap();
    assert_eq!(hot.iter().map(index).collect::<Vec<_>>(), [3, 2, 1, 0]);
    assert_eq!(hot[0]["seconds"], 0.75);
    assert_eq!(pixels(&hot[0]), pixels(&frames[3]));
    assert!(
        hot[0]["passes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pass| pass["kind"] == "blur"
                && pass["path"].as_str().unwrap().ends_with(".filters[0]")),
        "{}",
        hot[0]
    );
    let kinds = hot[0]["kinds"].as_array().unwrap();
    assert_eq!(
        kinds
            .iter()
            .map(|kind| kind["pixels"].as_u64().unwrap())
            .sum::<u64>(),
        pixels(&hot[0])
    );
    assert!(kinds.iter().any(|kind| kind["kind"] == "blur"), "{kinds:?}");
    assert_eq!(cost["failures"], json!([]));

    let doubled = check(Some("128x72"));
    assert_eq!(doubled["cost"]["width"], 128);
    let (once, twice) = (pixels(&frames[3]), pixels(&doubled["cost"]["frames"][3]));
    assert!(twice > 3 * once && twice < 5 * once, "{once} -> {twice}");
}

#[test]
fn budgeted_frames_reserve_the_font_they_load() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("title.motion.tsx"), BUDGET_TEXT).unwrap();
    // The first frame loads a font larger than all of this scene's pixels into its admission.
    for (output, budget) in [("plain", None), ("budgeted", Some("64MiB"))] {
        let pattern = format!("{output}/%05d.png");
        let mut args = vec![
            "motion",
            "render",
            "title.motion.tsx",
            "--backend",
            "raster",
            "--frames",
            "0,1,2,3",
            "-o",
            pattern.as_str(),
            "--json",
        ];
        if let Some(budget) = budget {
            args.extend(["--memory-budget", budget]);
        }
        success(run(dir.path(), &args));
    }
    for frame in 0..4 {
        let name = format!("{frame:05}.png");
        assert_eq!(
            png(&dir.path().join("plain").join(&name)),
            png(&dir.path().join("budgeted").join(&name))
        );
    }
}

#[test]
fn a_budget_only_needs_to_hold_the_frame_plans() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cost.motion.tsx"), COST).unwrap();
    // 96 MiB holds each of these 1080p frames, though not beside every cache at its ceiling.
    for (output, budget) in [("plain", None), ("budgeted", Some("96MiB"))] {
        let pattern = format!("{output}/%05d.png");
        let mut args = vec![
            "motion",
            "render",
            "cost.motion.tsx",
            "--backend",
            "raster",
            "--frames",
            "0,1,2,3",
            "--output-size",
            "1920x1080",
            "-o",
            pattern.as_str(),
            "--json",
        ];
        if let Some(budget) = budget {
            args.extend(["--memory-budget", budget]);
        }
        success(run(dir.path(), &args));
    }
    for frame in 0..4 {
        let name = format!("{frame:05}.png");
        assert_eq!(
            png(&dir.path().join("plain").join(&name)),
            png(&dir.path().join("budgeted").join(&name))
        );
    }
}

/// Frames beyond a renderer limit fail the cost check before anything renders, each with its
/// frame, time and the pass doing the most of the exceeded work.
#[test]
fn check_cost_lists_frames_beyond_the_shader_limits() {
    let dir = tempfile::tempdir().unwrap();
    let check = |source: &str, size: &str| {
        run(
            dir.path(),
            &[
                "motion",
                "check",
                source,
                "--asset",
                "effect=effect.shader.json",
                "--cost",
                "--output-size",
                size,
                "--json",
            ],
        )
    };
    // Built-in transitions are bounded by construction and never count toward the Shader budget,
    // even when a 72-sample linear blur covers a whole 4K frame.
    std::fs::write(dir.path().join("blur.motion.tsx"), COST_TRANSITION).unwrap();
    for size in ["1920x1080", "3840x2160"] {
        let report = success(run(
            dir.path(),
            &[
                "motion",
                "check",
                "blur.motion.tsx",
                "--cost",
                "--output-size",
                size,
                "--json",
            ],
        ));
        assert_eq!(report["cost"]["failures"], json!([]), "{size}");
    }

    std::fs::write(
        dir.path().join("effect.shader.json"),
        json!({
            "name": "workload", "entry": "effect.vsksl", "inputs": [], "uniforms": [],
            "output": {"colorSpace": "linear-srgb", "alphaMode": "straight", "allowTransparent": true},
            "budget": "local"
        })
        .to_string(),
    )
    .unwrap();
    // The budget is an average per output pixel, so a full-frame layer within it at 1080p stays
    // within it at 4K.
    std::fs::write(
        dir.path().join("effect.vsksl"),
        "float4 valle_main(float2 uv) { float4 value = float4(uv, 0.0, 1.0); \
         for (int i = 0; i < 32; ++i) { value = value * 0.999 + float4(0.0001); } return value; }",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("work.motion.tsx"),
        COST_SHADER.replace("NESTED", ""),
    )
    .unwrap();
    for size in ["1920x1080", "3840x2160"] {
        let report = success(check("work.motion.tsx", size));
        assert_eq!(report["cost"]["failures"], json!([]), "{size}");
    }

    // A layer sampling its content 64 times inside another that does too averages 128 samples per
    // pixel, twice the budget, at any output size from 1080p up.
    std::fs::write(
        dir.path().join("effect.vsksl"),
        "float4 valle_main(float2 uv) { float4 sum = float4(0.0); \
         for (int i = 0; i < 64; ++i) { sum += sampleContent(uv) / 64.0; } return sum; }",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("nested.motion.tsx"),
        COST_SHADER.replace(
            "NESTED",
            r#"<ShaderLayer source="asset://effect" style={size} />"#,
        ),
    )
    .unwrap();
    assert_eq!(
        success(check("nested.motion.tsx", "64x36"))["cost"]["failures"],
        json!([])
    );
    for size in ["1920x1080", "3840x2160"] {
        let output = check("nested.motion.tsx", size);
        assert!(!output.status.success(), "{size}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["error"]["code"], "frame_plan_failed", "{report}");
        let failures = report["cost"]["failures"].as_array().unwrap();
        assert_eq!(failures.len(), 4, "{size}: {report}");
        let last = failures.last().unwrap();
        assert_eq!(
            (last["frame"].as_u64(), last["seconds"].as_f64()),
            (Some(3), Some(0.75))
        );
        let error = last["error"].as_str().unwrap();
        assert!(
            error.contains("Shader work") && error.contains("the largest share is visual[0]."),
            "{error}"
        );
    }
}

#[test]
fn instance_range_matches_expanded_raster_at_multiple_samples() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lazy.motion.tsx"), LAZY_EVALUATION).unwrap();
    std::fs::write(
        dir.path().join("lazy-expanded.motion.tsx"),
        LAZY_EVALUATION.replace("className=\"absolute\"", "className=\"absolute \""),
    )
    .unwrap();
    for (source, output) in [
        ("lazy.motion.tsx", "inst/%05d.png"),
        ("lazy-expanded.motion.tsx", "expanded/%05d.png"),
    ] {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frames",
                "0,1,30",
                "--backend",
                "raster",
                "-o",
                output,
                "--json",
            ],
        ));
    }
    for frame in [0, 1, 30] {
        let name = format!("{frame:05}.png");
        assert_same_pixels(
            &png(&dir.path().join("inst").join(&name)),
            &png(&dir.path().join("expanded").join(&name)),
            &format!("lazy-evaluation frame {frame}"),
        );
    }
}

#[test]
fn check_and_raster_pixels_match_expanded_map() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("grid.motion.tsx"), INSTANCE_TEMPLATE).unwrap();
    std::fs::write(
        dir.path().join("grid-large.motion.tsx"),
        INSTANCE_TEMPLATE.replace("const COLS = 8;", "const COLS = 64;"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("grid-expanded.motion.tsx"),
        INSTANCE_TEMPLATE.replace("className=\"absolute\"", "className=\"absolute \""),
    )
    .unwrap();
    let fractional = INSTANCE_TEMPLATE.replace(
        "left: cell.x, top: cell.y, width: 4, height: 4,",
        "left: cell.x + 0.25, top: cell.y + 0.25, width: 4.6, height: 4.6,",
    );
    std::fs::write(dir.path().join("grid-fractional.motion.tsx"), &fractional).unwrap();
    std::fs::write(
        dir.path().join("grid-fractional-expanded.motion.tsx"),
        fractional.replace("className=\"absolute\"", "className=\"absolute \""),
    )
    .unwrap();
    let small = success(run(
        dir.path(),
        &["motion", "check", "grid.motion.tsx", "--json"],
    ));
    let large = success(run(
        dir.path(),
        &["motion", "check", "grid-large.motion.tsx", "--json"],
    ));
    for (report, rows) in [(&small, 64), (&large, 4096)] {
        assert_eq!(report["templates"], 1);
        assert_eq!(report["instanceRows"], rows);
        // Shared expressions are interned regardless of the number of instance rows.
        assert_eq!(report["expressions"], 19);
        assert!(report["evaluatedExpressions"].as_u64().unwrap() > 0);
        assert!(report["timings"]["templateCompile"].as_f64().unwrap() > 0.0);
        assert!(report["timings"]["instanceData"].as_f64().unwrap() > 0.0);
    }
    for (source, output) in [
        ("grid.motion.tsx", "inst/%05d.png"),
        ("grid-expanded.motion.tsx", "expanded/%05d.png"),
        ("grid-fractional.motion.tsx", "fractional/%05d.png"),
        (
            "grid-fractional-expanded.motion.tsx",
            "fractional-expanded/%05d.png",
        ),
    ] {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frames",
                "0,15,30",
                "--backend",
                "raster",
                "-o",
                output,
                "--json",
            ],
        ));
    }
    for frame in [0, 15, 30] {
        let name = format!("{frame:05}.png");
        assert_same_pixels(
            &png(&dir.path().join("inst").join(&name)),
            &png(&dir.path().join("expanded").join(&name)),
            &format!("instances frame {frame}"),
        );
        assert_same_pixels(
            &png(&dir.path().join("fractional").join(&name)),
            &png(&dir.path().join("fractional-expanded").join(&name)),
            &format!("fractional frame {frame}"),
        );
    }
}

#[test]
fn circle_template_matches_expanded_arc_edges() {
    let dir = tempfile::tempdir().unwrap();
    let varied = CIRCLE_INSTANCE_TEMPLATE
        .replace(
            "20.3 + (i % COLS) * (600 / COLS)",
            "8.13 + ((i * 73) % 600) + (i % 7) * 0.07",
        )
        .replace(
            "20.6 + Math.floor(i / COLS) * (320 / COLS)",
            "8.17 + ((i * 47) % 340) + (i % 5) * 0.09",
        )
        .replace("3.5 + (i % 3) * 0.25", "1.37 + (i % 23) * 1.11");
    std::fs::write(dir.path().join("circle.motion.tsx"), &varied).unwrap();
    std::fs::write(
        dir.path().join("circle-expanded.motion.tsx"),
        varied.replace("<Circle ", "<circle "),
    )
    .unwrap();
    for (source, output) in [
        ("circle.motion.tsx", "template/%05d.png"),
        ("circle-expanded.motion.tsx", "expanded/%05d.png"),
    ] {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frames",
                "0,15,30",
                "--backend",
                "raster",
                "-o",
                output,
                "--json",
            ],
        ));
    }
    for frame in [0, 15, 30] {
        let name = format!("{frame:05}.png");
        assert_same_pixels(
            &png(&dir.path().join("template").join(&name)),
            &png(&dir.path().join("expanded").join(&name)),
            &format!("instances Circle frame {frame}"),
        );
    }
}

#[test]
fn path_template_matches_expanded_fractional_edges() {
    let dir = tempfile::tempdir().unwrap();
    let varied = PATH_INSTANCE_TEMPLATE
        .replace(
            "20.3 + (i % COLS) * (600 / COLS)",
            "8.13 + ((i * 73) % 600) + (i % 7) * 0.07",
        )
        .replace(
            "20.6 + Math.floor(i / COLS) * (320 / COLS)",
            "8.17 + ((i * 47) % 340) + (i % 5) * 0.09",
        );
    std::fs::write(dir.path().join("path.motion.tsx"), &varied).unwrap();
    std::fs::write(
        dir.path().join("path-expanded.motion.tsx"),
        varied.replace("<Path ", "<path "),
    )
    .unwrap();
    let report = success(run(
        dir.path(),
        &["motion", "check", "path.motion.tsx", "--json"],
    ));
    assert_eq!(report["templates"], 1);
    assert_eq!(report["instanceRows"], 64);
    for (source, output) in [
        ("path.motion.tsx", "template/%05d.png"),
        ("path-expanded.motion.tsx", "expanded/%05d.png"),
    ] {
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                source,
                "--frames",
                "0,15,30",
                "--backend",
                "raster",
                "-o",
                output,
                "--json",
            ],
        ));
    }
    for frame in [0, 15, 30] {
        let name = format!("{frame:05}.png");
        assert_same_pixels(
            &png(&dir.path().join("template").join(&name)),
            &png(&dir.path().join("expanded").join(&name)),
            &format!("instances Path frame {frame}"),
        );
    }
}

#[test]
fn transparent_mov_and_full_png_sequence_round_trip_exactly() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.motion.tsx"), ALPHA).unwrap();
    let report = success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "alpha.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "alpha.mov",
            "--codec",
            "qtrle",
            "--json",
        ],
    ));
    assert_eq!(report["compilations"], 1);
    assert_eq!(report["delivery"]["frames"], 30);
    assert_eq!(report["delivery"]["codec"], "qtrle");
    let decoded = decode_rgba_frames(&dir.path().join("alpha.mov"), None).unwrap();
    assert_eq!(decoded.len(), 30);
    let first = &decoded[0];
    assert_eq!((first.width, first.height), (640, 360));
    assert!(first.data.chunks_exact(4).any(|pixel| pixel[3] <= 5));
    assert!(first.data.chunks_exact(4).any(|pixel| pixel[3] >= 250));
    // Antialiasing must survive too; a binary opaque/transparent mask is insufficient.
    assert!(
        first
            .data
            .chunks_exact(4)
            .any(|pixel| pixel[3] > 5 && pixel[3] < 250)
    );
    let sequence = success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "alpha.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "frames/%05d.png",
            "--json",
        ],
    ));
    assert_eq!(sequence["compilations"], 1);
    assert_eq!(sequence["delivery"]["frames"], 30);
    assert_eq!(
        std::fs::read_dir(dir.path().join("frames"))
            .unwrap()
            .count(),
        30
    );
    for (i, frame) in decoded.iter().enumerate() {
        assert_eq!(*frame, png(&dir.path().join(format!("frames/{i:05}.png"))));
    }
}

#[test]
fn prores_4444_mov_preserves_png_alpha() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("alpha.motion.tsx"),
        ALPHA.replace("duration: 1", "duration: 0.2"),
    )
    .unwrap();
    let report = success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "alpha.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "alpha.mov",
            "--codec",
            "prores4444",
            "--json",
        ],
    ));
    assert_eq!(report["delivery"]["codec"], "prores4444");
    assert_eq!(report["delivery"]["frames"], 6);
    let decoded = decode_rgba_frames(&dir.path().join("alpha.mov"), None).unwrap();
    assert_eq!(decoded.len(), 6);
    success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "alpha.motion.tsx",
            "--backend",
            "raster",
            "-o",
            "frames/%05d.png",
            "--json",
        ],
    ));
    let mut found_transparent = false;
    let mut found_partial = false;
    let mut found_opaque = false;
    for (index, frame) in decoded.iter().enumerate() {
        let expected = png(&dir.path().join(format!("frames/{index:05}.png")));
        assert_eq!(
            (frame.width, frame.height),
            (expected.width, expected.height)
        );
        for (actual, source) in frame
            .data
            .chunks_exact(4)
            .zip(expected.data.chunks_exact(4))
        {
            assert!(
                actual[3].abs_diff(source[3]) <= 2,
                "alpha {actual:?} != {source:?}"
            );
            found_transparent |= source[3] <= 5;
            found_partial |= source[3] > 5 && source[3] < 250;
            found_opaque |= source[3] >= 250;
        }
    }
    assert!(found_transparent && found_partial && found_opaque);
}

#[test]
fn sheet_cells_and_sparse_outputs_equal_independent_frames_with_one_compile() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.motion.tsx"), MULTI).unwrap();
    let output = run(
        dir.path(),
        &[
            "--events",
            "motion",
            "render",
            "main.motion.tsx",
            "--backend",
            "raster",
            "--frames",
            "0,30,59",
            "--storyboard",
            "sheet.png",
            "-o",
            "frames/%05d.png",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let events = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let compilations = events
        .iter()
        .filter(|event| event["type"] == "motion.compilation")
        .collect::<Vec<_>>();
    assert_eq!(compilations.len(), 1);
    assert_eq!(compilations[0]["data"]["entry"], "main.motion.tsx");
    let report = &events.last().unwrap()["data"];
    assert_eq!(report["compilations"], 1);
    assert_eq!(report["delivery"]["frameKeys"], json!([0, 30, 59]));
    assert_eq!(report["delivery"]["frames"], 3);
    let sheet = png(&dir.path().join("sheet.png"));
    assert_eq!((sheet.width, sheet.height), (1920, 360));
    for (index, frame) in [0, 30, 59].into_iter().enumerate() {
        let filename = format!("single-{frame}.png");
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                "main.motion.tsx",
                "--backend",
                "raster",
                "--frame",
                &frame.to_string(),
                "-o",
                &filename,
            ],
        ));
        let individual = png(&dir.path().join(&filename));
        assert_eq!(
            individual,
            png(&dir.path().join(format!("frames/{frame:05}.png")))
        );
        assert_cell(&sheet, &individual, index, 3);
    }
    // Exact requested order and duplicate frames survive sheet delivery and worker scheduling.
    let report = success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "main.motion.tsx",
            "--backend",
            "raster",
            "--threads",
            "2",
            "--frames",
            "59,0,30,0,59",
            "--storyboard",
            "reverse.png",
            "--json",
        ],
    ));
    assert_eq!(report["compilations"], 1);
    assert_eq!(report["delivery"]["frameKeys"], json!([59, 0, 30, 0, 59]));
    let reverse = png(&dir.path().join("reverse.png"));
    assert_eq!((reverse.width, reverse.height), (2560, 720));
    for (i, frame) in [59, 0, 30, 0, 59].into_iter().enumerate() {
        assert_cell(
            &reverse,
            &png(&dir.path().join(format!("single-{frame}.png"))),
            i,
            4,
        );
    }
    assert_eq!(reverse.pixel(2000, 600), Some([0, 0, 0, 0]));
}

#[test]
fn bare_filename_import_check_matches_dot_relative_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.motion.ts"), "export const SIZE = 64;").unwrap();
    std::fs::write(dir.path().join("module-entry.motion.tsx"), r##"
import { SIZE } from "./lib";
export const composition = { width: 640, height: 360, fps: 30, duration: 1 };
export default function Main() { return <Scene className="relative h-full w-full" style={{backgroundColor:"#101010"}}>
<View key="box" className="absolute" style={{left:20,top:20,width:SIZE,height:SIZE,backgroundColor:"#ffffff"}} /></Scene>; }
"##).unwrap();
    let mut bare = success(run(
        dir.path(),
        &["motion", "check", "module-entry.motion.tsx", "--json"],
    ));
    let mut relative = success(run(
        dir.path(),
        &["motion", "check", "./module-entry.motion.tsx", "--json"],
    ));
    assert_eq!(bare["compilations"], 1);
    // The entry spelling must not change check semantics; measured durations can differ.
    bare.as_object_mut().unwrap().remove("timings");
    relative.as_object_mut().unwrap().remove("timings");
    assert_eq!(bare, relative);
}

#[test]
fn default_storyboard_scales_cells_and_samples_first_and_last_frames_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.motion.tsx"), MULTI).unwrap();
    let report = success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "main.motion.tsx",
            "--backend",
            "raster",
            "--storyboard",
            "sheet.png",
            "--output-size",
            "160x90",
            "--json",
        ],
    ));
    assert_eq!(report["compilations"], 1);
    assert_eq!(report["delivery"]["frames"], 12);
    let keys = report["delivery"]["frameKeys"].as_array().unwrap();
    assert_eq!(keys.first().unwrap(), 0);
    assert_eq!(keys.last().unwrap(), 59);
    let sheet = png(&dir.path().join("sheet.png"));
    assert_eq!((sheet.width, sheet.height), (640, 270));
    for (index, frame) in [(0, 0), (11, 59)] {
        let file = format!("single-{frame}.png");
        success(run(
            dir.path(),
            &[
                "motion",
                "render",
                "main.motion.tsx",
                "--backend",
                "raster",
                "--frame",
                &frame.to_string(),
                "--output-size",
                "160x90",
                "-o",
                &file,
            ],
        ));
        assert_cell(&sheet, &png(&dir.path().join(file)), index, 4);
    }
}

#[test]
fn image_delivery_preflights_all_frames_and_outputs_without_partial_publication() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.motion.tsx"), MULTI).unwrap();
    for frames in ["0,60", "0,0"] {
        let output = run(
            dir.path(),
            &[
                "motion",
                "render",
                "main.motion.tsx",
                "--backend",
                "raster",
                "--frames",
                frames,
                "-o",
                "bad/%05d.png",
                "--json",
            ],
        );
        assert!(!output.status.success());
        assert!(!dir.path().join("bad/00000.png").exists());
    }
    std::fs::create_dir_all(dir.path().join("frames")).unwrap();
    std::fs::write(dir.path().join("frames/00030.png"), b"keep").unwrap();
    let output = run(
        dir.path(),
        &[
            "motion",
            "render",
            "main.motion.tsx",
            "--backend",
            "raster",
            "--frames",
            "0,30",
            "-o",
            "frames/%05d.png",
            "--storyboard",
            "sheet.png",
            "--json",
        ],
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(dir.path().join("frames/00030.png")).unwrap(),
        b"keep"
    );
    assert!(!dir.path().join("frames/00000.png").exists());
    assert!(!dir.path().join("sheet.png").exists());
    assert_eq!(
        std::fs::read_dir(dir.path().join("frames"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn video_codec_and_selection_mismatches_fail_before_reading_source() {
    for args in [
        vec!["-o", "bad.mov", "--codec", "h264"],
        vec!["-o", "bad.mp4", "--codec", "qtrle"],
        vec!["-o", "bad.mp4", "--codec", "prores4444"],
        vec!["-o", "bad.mov", "--codec", "hevc"],
        vec!["-o", "bad.mov", "--hardware-encode"],
        vec!["-o", "bad.mp4", "--bitrate", "1000000"],
        vec!["-o", "bad.mp4", "--codec", "hevc", "--encode-threads", "2"],
        vec!["-o", "bad.png"],
        vec!["-o", "%d-%d.png"],
        vec!["-o", "bad.mp4", "--frames", "0,1"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut command = vec!["motion", "render", "missing.motion.tsx", "--json"];
        command.extend(args);
        let output = run(dir.path(), &command);
        assert!(!output.status.success(), "{command:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            !report["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot read Motion")
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

/// An MP4 `colr` box of BT.709 primaries, the IEC 61966-2-1 (sRGB) transfer, the BT.709 matrix
/// and limited range.
fn tagged_srgb(mp4: &[u8]) -> bool {
    let colr = b"colrnclx\x00\x01\x00\x0d\x00\x01\x00";
    mp4.windows(colr.len()).any(|window| window == colr)
}

/// MP4 stores the sRGB values of the PNG frame and tags them as sRGB, so players show the same
/// colors. Dark colors expose any other transfer: BT.709 encoding writes #102030 as #071121.
#[test]
fn mp4_carries_the_png_colors_tagged_as_srgb() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bands.motion.tsx"),
        r##"export const composition = { width: 192, height: 64, fps: 30, duration: 0.2 };
export default function Bands() {
  return <Scene className="relative h-full w-full">
    <View className="absolute" style={{ left: 0, top: 0, width: 64, height: 64, backgroundColor: "#102030" }} />
    <View className="absolute" style={{ left: 64, top: 0, width: 64, height: 64, backgroundColor: "#808080" }} />
    <View className="absolute" style={{ left: 128, top: 0, width: 64, height: 64, backgroundColor: "#3c78c8" }} />
  </Scene>;
}
"##,
    )
    .unwrap();
    success(run(
        dir.path(),
        &[
            "motion",
            "render",
            "bands.motion.tsx",
            "--frame",
            "0",
            "-o",
            "frame.png",
            "--json",
        ],
    ));
    let expected = png(&dir.path().join("frame.png"));
    let mut encodings = vec![("software.mp4", vec![])];
    if valle_media::codec::hw_h264_available() {
        encodings.push(("hardware.mp4", vec!["--hardware-encode"]));
        encodings.push((
            "hardware-raster.mp4",
            vec!["--hardware-encode", "--backend", "raster"],
        ));
    }
    for (file, flags) in encodings {
        let mut command = vec!["motion", "render", "bands.motion.tsx", "-o", file, "--json"];
        command.extend(flags);
        success(run(dir.path(), &command));
        assert!(
            tagged_srgb(&std::fs::read(dir.path().join(file)).unwrap()),
            "{file}"
        );
        let decoded = decode_rgba_frames(&dir.path().join(file), None).unwrap();
        for x in [32, 96, 160] {
            let offset = (32 * 192 + x) * 4;
            let actual = &decoded[0].data[offset..][..3];
            let source = &expected.data[offset..][..3];
            assert!(
                actual.iter().zip(source).all(|(a, b)| a.abs_diff(*b) <= 3),
                "{file} at x {x}: {actual:?} != {source:?}"
            );
        }
    }
}

#[test]
fn hevc_mp4_is_tagged_hvc1_for_motion_and_timeline() {
    use valle_media::codec::{Mp4VideoCodec, Muxer, VideoFrameTransport, VideoTransfer};
    // Skip where the platform cannot open a hardware HEVC encoder.
    let probe = tempfile::tempdir().unwrap();
    if let Err(error) = Muxer::open_ext_rational_with_transport(
        &probe.path().join("probe.mp4"),
        320,
        180,
        30,
        1,
        Mp4VideoCodec::Hevc,
        VideoTransfer::Srgb,
        None,
        true,
        None,
        VideoFrameTransport::Cpu,
    ) {
        eprintln!("skip HEVC delivery test: {error}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("scene.motion.tsx"),
        "export const composition = { width: 320, height: 180, fps: 30, duration: 0.5 }; export default function Test(){return <Scene><View style={{position:'absolute',left:0,top:0,width:320,height:180,backgroundColor:'#0000ff'}} /></Scene>;}",
    )
    .unwrap();
    let timeline = json!({
        "canvas": {"width": 320, "height": 180, "fps": 30},
        "tracks": {"visual":[{"clips":[{"kind":"solid", "color":"#ff0000", "start":0, "duration":0.5}]}]}
    });
    std::fs::write(dir.path().join("timeline.json"), timeline.to_string()).unwrap();
    let motion = success(run(
        dir.path(),
        &[
            "--json",
            "motion",
            "render",
            "scene.motion.tsx",
            "-o",
            "motion.mp4",
            "--codec",
            "hevc",
            "--bitrate",
            "2000000",
        ],
    ));
    let timeline = success(run(
        dir.path(),
        &[
            "--json",
            "timeline",
            "render",
            "timeline.json",
            "-o",
            "timeline.mp4",
            "--codec",
            "hevc",
        ],
    ));
    for (report, file, color) in [
        (motion, "motion.mp4", [0, 0, 255]),
        (timeline, "timeline.mp4", [255, 0, 0]),
    ] {
        assert_eq!(report["delivery"]["codec"], "hevc", "{report}");
        let bytes = std::fs::read(dir.path().join(file)).unwrap();
        let has = |tag: &[u8]| bytes.windows(4).any(|w| w == tag);
        assert!(has(b"hvc1") && has(b"hvcC") && !has(b"hev1"), "{file}");
        assert!(tagged_srgb(&bytes), "{file}");
        let decoded = decode_rgba_frames(&dir.path().join(file), None).unwrap();
        assert_eq!(decoded.len(), 15, "{file}");
        // A flat primary survives the encoder's conversion to BT.709 YUV within a few codes.
        let center = &decoded[7].data[(90 * 320 + 160) * 4..][..3];
        assert!(
            center
                .iter()
                .zip(color)
                .all(|(&code, expected)| code.abs_diff(expected) <= 4),
            "{file}: {center:?}"
        );
    }
}

#[test]
fn timeline_transparent_video_and_frames_share_delivery_and_fractional_clock() {
    let dir = tempfile::tempdir().unwrap();
    let timeline = json!({
        "canvas": {"width": 65, "height": 33, "fps": "30000/1001", "background":"#00000000"},
        "tracks": {"visual":[{"clips":[{"kind":"solid", "color":"#4488cc80", "start":0, "duration":0.1}]}]}
    });
    std::fs::write(dir.path().join("timeline.json"), timeline.to_string()).unwrap();
    let report = success(run(
        dir.path(),
        &[
            "timeline",
            "render",
            "timeline.json",
            "-o",
            "alpha.mov",
            "--json",
        ],
    ));
    assert_eq!(report["compilations"], 0);
    assert_eq!(report["delivery"]["frames"], 3);
    let decoded = decode_rgba_frames(&dir.path().join("alpha.mov"), None).unwrap();
    assert_eq!(decoded.len(), 3);
    assert_eq!((decoded[0].width, decoded[0].height), (65, 33));
    let clock = valle_media::codec::decode::probe_video_presentation(&dir.path().join("alpha.mov"))
        .unwrap();
    assert_eq!(clock.presentation.len(), 3);
    for pair in clock.presentation.windows(2) {
        let seconds = (pair[1].0 - pair[0].0) as f64 * f64::from(clock.time_base.0)
            / f64::from(clock.time_base.1);
        assert!((seconds - 1001.0 / 30000.0).abs() < 1e-9);
    }
    success(run(
        dir.path(),
        &["timeline", "render", "timeline.json", "-o", "frames/%d.png"],
    ));
    for (i, frame) in decoded.iter().enumerate() {
        assert_eq!(*frame, png(&dir.path().join(format!("frames/{i}.png"))));
        assert!(
            frame
                .data
                .chunks_exact(4)
                .all(|pixel| (i16::from(pixel[3]) - 128).abs() <= 1)
        );
    }
    let prores = success(run(
        dir.path(),
        &[
            "timeline",
            "render",
            "timeline.json",
            "-o",
            "prores.mov",
            "--codec",
            "prores4444",
            "--json",
        ],
    ));
    assert_eq!(prores["delivery"]["codec"], "prores4444");
    assert_eq!(prores["delivery"]["frames"], 3);
    let prores_frames = decode_rgba_frames(&dir.path().join("prores.mov"), None).unwrap();
    assert_eq!(prores_frames.len(), 3);
    for (index, frame) in prores_frames.iter().enumerate() {
        let expected = png(&dir.path().join(format!("frames/{index}.png")));
        assert_eq!((frame.width, frame.height), (65, 33));
        for (actual, source) in frame
            .data
            .chunks_exact(4)
            .zip(expected.data.chunks_exact(4))
        {
            assert!(actual[3].abs_diff(source[3]) <= 2);
        }
    }
    let rejected = run(
        dir.path(),
        &["timeline", "render", "timeline.json", "-o", "opaque.mp4"],
    );
    assert!(!rejected.status.success());
    assert!(!dir.path().join("opaque.mp4").exists());
}

#[test]
fn qtrle_rejects_audio_instead_of_dropping_it_and_audio_only_mp4_reports_aac() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer =
        valle_media::codec::FloatWavWriter::create(&dir.path().join("audio.wav"), 48_000, 1)
            .unwrap();
    writer
        .write(&valle_media::frame::AudioBuffer {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.125; 4800],
        })
        .unwrap();
    writer.finish().unwrap();
    let mut timeline = json!({
        "canvas": {"width":64,"height":32,"fps":30},
        "resources": {"sound":"audio.wav"},
        "tracks": {
            "visual":[{"clips":[{"kind":"solid","color":"#ffffff80","start":0,"duration":0.1}]}],
            "audio":[{"clips":[{"src":"sound","start":0,"duration":0.1}]}]
        }
    });
    std::fs::write(dir.path().join("timeline.json"), timeline.to_string()).unwrap();
    let rejected = run(
        dir.path(),
        &[
            "timeline",
            "render",
            "timeline.json",
            "-o",
            "alpha.mov",
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("video only")
    );
    assert!(!dir.path().join("alpha.mov").exists());
    timeline["tracks"]["visual"] = json!([]);
    std::fs::write(dir.path().join("timeline.json"), timeline.to_string()).unwrap();
    let report = success(run(
        dir.path(),
        &[
            "timeline",
            "render",
            "timeline.json",
            "-o",
            "audio.mp4",
            "--json",
        ],
    ));
    assert_eq!(report["delivery"]["audioOnly"], true);
    assert_eq!(report["delivery"]["codec"], "aac");
    let probe = valle_media::codec::probe_av(&dir.path().join("audio.mp4")).unwrap();
    assert!(probe.audio.is_some());
    assert!(probe.video.is_none());
}
