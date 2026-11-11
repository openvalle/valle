//! Storyboard selection, pagination and errors through the Motion, Timeline and Project CLI.
//! PNG rendering runs with VALLE_TEST_LAYERS=gpu; format errors need no renderer.

use std::path::{Path, PathBuf};
use std::process::Command;

use indoc::indoc;
use serde_json::{Value, json};

const VALLE: &str = env!("CARGO_BIN_EXE_valle");
const SCENE: &str = indoc! {r##"
    export const composition = { width: 64, height: 36, fps: 30, duration: 1 };
    export default function Scene() {
      return <View style={{ width: 32, height: 36, backgroundColor: "#202020",
        opacity: useCurrentTime().progress }} />;
    }
"##};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("valle-storyboard-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn invoke(dir: &Path, args: &[&str]) -> (bool, Value) {
    let out = Command::new(VALLE)
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)));
    (out.status.success(), value)
}

fn run(dir: &Path, args: &[&str]) -> Value {
    let (ok, value) = invoke(dir, args);
    assert!(ok, "{args:?}: {value}");
    value
}

fn png(path: &Path) -> ([u32; 2], Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    ([info.width, info.height], pixels)
}

fn pages(dir: &Path, value: &Value, counts: &[usize]) {
    let pages = value["storyboard"]["pages"].as_array().unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(pages.len(), counts.len());
    assert_eq!(files.len(), counts.len());
    let mut frames = Vec::new();
    for ((page, count), file) in pages.iter().zip(counts).zip(files) {
        assert_eq!(&page["file"], file);
        let cells = page["cells"].as_array().unwrap();
        assert_eq!(cells.len(), *count);
        let (size, pixels) = png(&dir.join(file.as_str().unwrap()));
        assert_eq!(json!(size), json!([page["width"], page["height"]]));
        // The page heading is real antialiased text, with intermediate coverage values.
        let top = cells[0]["y"].as_u64().unwrap() as usize;
        assert!(
            pixels[..top * size[0] as usize * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 32 && p[0] < 128 && p[0] == p[1] && p[1] == p[2])
        );
        for cell in cells {
            assert!(cell["time"].is_number());
            assert!(cell["timecode"].is_string());
            assert!(
                cell["x"].as_u64().unwrap() + cell["width"].as_u64().unwrap() <= size[0] as u64
            );
            assert!(
                cell["y"].as_u64().unwrap() + cell["height"].as_u64().unwrap() <= size[1] as u64
            );
            frames.push(cell["frame"].clone());
        }
    }
    assert_eq!(Value::from(frames), value["frames"]);
}

#[test]
fn non_png_storyboards_fail_before_loading_sources_or_creating_outputs() {
    let dir = folder("format");
    run(&dir, &["project", "create", "."]);
    std::fs::write(dir.join("bad.motion.tsx"), "not TSX").unwrap();
    std::fs::write(dir.join("main.timeline.json"), "not JSON").unwrap();
    for (group, file) in [
        ("motion", Some("bad.motion.tsx")),
        ("timeline", Some("main.timeline.json")),
        ("project", None),
    ] {
        for out in ["bad.mp4", "bad.mov", "bad.wav", "directory"] {
            let mut args = vec![group, "render"];
            args.extend(file);
            args.extend(["--storyboard", "-o", out]);
            let (ok, value) = invoke(&dir, &args);
            assert!(!ok, "{args:?}: {value}");
            assert_eq!(value["code"], "usage", "{value}");
            assert_eq!(value["message"], "a storyboard is a .png", "{value}");
            assert!(!dir.join(out).exists());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn storyboard_cli_samples_pages_and_preserves_explicit_frames() {
    if !std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|l| l == "gpu")
    {
        return;
    }
    let dir = folder("render");
    run(&dir, &["project", "create", "."]);
    std::fs::write(dir.join("scene.motion.tsx"), SCENE).unwrap();
    let doc = json!({"canvas":{"width":64,"height":36,"fps":"30000/1001"},
        "tracks":{"visual":[{"items":[{"id":"a","kind":"solid","color":"red","duration":1}]}]}});
    std::fs::write(dir.join("main.timeline.json"), doc.to_string()).unwrap();
    for args in [
        vec![
            "motion",
            "render",
            "scene.motion.tsx",
            "--storyboard",
            "-o",
            "motion.png",
        ],
        vec![
            "timeline",
            "render",
            "main.timeline.json",
            "--storyboard",
            "-o",
            "timeline.png",
        ],
        vec!["project", "render", "--storyboard"],
    ] {
        let value = run(&dir, &args);
        let selected = value["frames"].as_array().unwrap();
        assert_eq!(selected.len(), 12);
        assert_eq!(selected[0], 0);
        assert_eq!(selected[11], 29);
        pages(&dir, &value, &[12]);
    }
    let sampled = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--storyboard",
            "--samples",
            "20",
            "--per-page",
            "8",
            "-o",
            "sheets/片段.test.png",
        ],
    );
    pages(&dir, &sampled, &[8, 8, 4]);
    assert_eq!(
        sampled["files"],
        json!([
            "sheets/片段.test-001.png",
            "sheets/片段.test-002.png",
            "sheets/片段.test-003.png"
        ])
    );
    assert!(!dir.join("sheets/片段.test.png").exists());

    let explicit = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--storyboard",
            "--frames",
            "29,0,29",
            "--per-page",
            "2",
            "-o",
            "ordered.png",
        ],
    );
    pages(&dir, &explicit, &[2, 1]);
    assert_eq!(explicit["frames"], json!([29, 0, 29]));
    let (size, pixels) = png(&dir.join("ordered-001.png"));
    let red = |cell: &Value| {
        let x = cell["x"].as_u64().unwrap() as usize;
        let y = cell["y"].as_u64().unwrap() as usize;
        pixels[(y * size[0] as usize + x) * 4]
    };
    let cells = &explicit["storyboard"]["pages"][0]["cells"];
    assert!(red(&cells[0]) < 50, "late frame shows the dark box");
    assert_eq!(
        red(&cells[1]),
        64,
        "frame zero shows the transparent checkerboard"
    );

    let stepped = run(
        &dir,
        &[
            "timeline",
            "render",
            "main.timeline.json",
            "--storyboard",
            "--step",
            "10",
            "--per-page",
            "2",
            "-o",
            "stepped.png",
        ],
    );
    pages(&dir, &stepped, &[2, 1]);
    assert_eq!(stepped["frames"], json!([0, 10, 20]));
    let time = stepped["storyboard"]["pages"][0]["cells"][1]["time"]
        .as_f64()
        .unwrap();
    assert!((time - 10010.0 / 30000.0).abs() < 1e-9);

    std::fs::write(
        dir.join("short.motion.tsx"),
        SCENE.replace("duration: 1", "duration: 0.2"),
    )
    .unwrap();
    let short = run(
        &dir,
        &[
            "motion",
            "render",
            "short.motion.tsx",
            "--storyboard",
            "--samples",
            "60",
            "-o",
            "short.png",
        ],
    );
    assert_eq!(short["frames"], json!([0, 1, 2, 3, 4, 5]));
    pages(&dir, &short, &[6]);
    assert_eq!(short["files"], json!(["short.png"]));
    let one = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--storyboard",
            "--frame",
            "7",
            "-o",
            "one.png",
        ],
    );
    assert_eq!(one["frames"], json!([7]));
    pages(&dir, &one, &[1]);
    let range = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--storyboard",
            "--frames",
            "0..14",
            "-o",
            "range.png",
        ],
    );
    pages(&dir, &range, &[12, 3]);

    // Automatic project output chooses a new base even when only numbered pages exist.
    let args = [
        "project",
        "render",
        "--storyboard",
        "--samples",
        "20",
        "--per-page",
        "8",
    ];
    let first = run(&dir, &args);
    let second = run(&dir, &args);
    pages(&dir, &first, &[8, 8, 4]);
    pages(&dir, &second, &[8, 8, 4]);
    for file in first["files"].as_array().unwrap() {
        assert!(!second["files"].as_array().unwrap().contains(file));
    }
    let plain = run(
        &dir,
        &["motion", "render", "scene.motion.tsx", "-o", "plain.png"],
    );
    assert_eq!(plain["frames"], json!([0]));
    assert!(plain.get("storyboard").is_none());
    let sequence = run(
        &dir,
        &["motion", "render", "short.motion.tsx", "-o", "sequence"],
    );
    assert_eq!(sequence["frames"], json!([0, 1, 2, 3, 4, 5]));
    std::fs::remove_dir_all(dir).unwrap();
}
