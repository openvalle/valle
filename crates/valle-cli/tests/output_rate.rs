//! Output rate changes sampling, including hooks and audio, without rewriting the sources.
//! Native rendering runs with VALLE_TEST_LAYERS=gpu.

use std::path::{Path, PathBuf};
use std::process::Command;

use indoc::indoc;
use serde_json::{Value, json};
use valle_native::media::{self, Decoding, RATE};
use valle_timeline::time::{Rate, Tick};

const SCENE: &str = indoc! {r##"
    export const composition = { width: 128, height: 64, fps: 4, duration: 2 };
    export default function Scene() {
      const t = useCurrentTime().seconds;
      return <View style={{ width: 128, height: 64, background: "black" }}>
        <View style={{ position: "absolute", top: 0, left: t * 32,
          width: 8, height: 16, background: "red" }} />
        <View style={{ position: "absolute", top: 32, left: 0,
          width: useComposition().fps, height: 16, background: "lime" }} />
      </View>;
    }
"##};

fn enabled() -> bool {
    std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|l| l == "gpu")
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("valle-output-rate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn invoke(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)))
}

fn run(dir: &Path, args: &[&str]) -> Value {
    let v = invoke(dir, args);
    assert_eq!(v["status"], "ok", "{args:?}: {v}");
    v
}

fn png(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    pixels
}

#[test]
fn images_use_output_frame_numbers_and_actual_composition_rate() {
    if !enabled() {
        return;
    }
    let dir = folder("images");
    std::fs::write(dir.join("scene.motion.tsx"), SCENE).unwrap();
    let old = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--frame",
            "2",
            "-o",
            "old.png",
        ],
    );
    let new = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--fps",
            "8",
            "--frame",
            "4",
            "-o",
            "new.png",
        ],
    );
    assert_eq!(old["fps"], "4");
    assert_eq!(new["fps"], "8");
    let (old, new) = (png(&dir.join("old.png")), png(&dir.join("new.png")));
    // Both are at 0.5 seconds; the moving red box has identical pixels.
    assert_eq!(&old[..128 * 24 * 4], &new[..128 * 24 * 4]);
    let at = |x: usize, y: usize| (y * 128 + x) * 4;
    assert_eq!(&new[at(17, 8)..at(17, 8) + 4], [255, 0, 0, 255]);
    assert_eq!(&new[at(33, 8)..at(33, 8) + 4], [0, 0, 0, 255]);
    // useComposition().fps sees 8, while the authored file still says 4.
    assert_eq!(&old[at(6, 40)..at(6, 40) + 4], [0, 0, 0, 255]);
    assert_eq!(&new[at(6, 40)..at(6, 40) + 4], [0, 255, 0, 255]);

    let motion = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--fps",
            "8",
            "--encoder",
            "ffmpeg",
            "-o",
            "motion.mp4",
        ],
    );
    assert_eq!(motion["fps"], "8");
    assert_eq!(motion["frames"], 16);
    let info = media::probe(&dir.join("motion.mp4")).unwrap();
    assert_eq!(info.fps, Some([8, 1]));
    assert!(!info.audio);

    let seq = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--fps",
            "3",
            "-o",
            "frames",
        ],
    );
    assert_eq!(seq["frames"], json!([0, 1, 2, 3, 4, 5]));
    assert_eq!(seq["files"].as_array().unwrap().len(), 6);
    let board = run(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--fps",
            "30000/1001",
            "--storyboard",
            "--frames",
            "0,30,59",
            "-o",
            "sheet.png",
        ],
    );
    assert_eq!(board["fps"], "30000/1001");
    let cell = &board["storyboard"]["pages"][0]["cells"][1];
    assert_eq!(cell["frame"], 30);
    assert_eq!(cell["time"], 1.001);
    assert_eq!(cell["timecode"], "00:00:01;00");
    let bad = invoke(
        &dir,
        &[
            "motion",
            "render",
            "scene.motion.tsx",
            "--fps",
            "8",
            "--frame",
            "16",
            "-o",
            "bad.png",
        ],
    );
    assert_eq!(bad["status"], "error");
    assert!(
        bad["message"]
            .as_str()
            .unwrap()
            .contains("past the last, 15")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("scene.motion.tsx")).unwrap(),
        SCENE
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Two tones at known times, so a changed video rate cannot silently speed up the soundtrack.
fn sound(path: &Path) {
    let samples: Vec<i16> = (0..RATE * 2)
        .map(|i| {
            let t = f64::from(i) / f64::from(RATE);
            if (0.5..0.75).contains(&t) || (1.5..1.75).contains(&t) {
                ((t * 440.0 * std::f64::consts::TAU).sin() * 8192.0) as i16
            } else {
                0
            }
        })
        .collect();
    let len = samples.len() as u32 * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(len + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    for value in [16u32, 1 | (1 << 16), RATE, RATE * 2, 2 | (16 << 16)] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn video(path: &Path, fps: Rate) {
    let info = media::probe(path).unwrap();
    assert_eq!(info.fps, Some([fps.num() as u32, fps.den() as u32]));
    assert!(info.audio);
    let frames = fps.frames_in(Tick::parse_seconds("2").unwrap());
    let expected = fps.frame_start(frames).unwrap().0;
    assert!((info.duration.unwrap() - expected).abs() < 1000);
    let mut reader = media::reader(path, Some(Decoding::Software)).unwrap();
    let pixels = reader.frame(Tick::parse_seconds("1").unwrap().0).unwrap();
    // The box has moved 32 pixels at one second, independently of the export rate.
    let p = &pixels[(8 * 128 + 35) * 4..(8 * 128 + 35) * 4 + 3];
    assert!(p[0] > 180 && p[1] < 80 && p[2] < 80, "{p:?}");
    let audio = media::sound(path).unwrap().unwrap();
    let rms = |start: f64| {
        let a = (start * f64::from(RATE)) as usize * 2;
        let b = a + (RATE / 10) as usize * 2;
        let sum: f64 = audio[a..b].iter().map(|&s| f64::from(s).powi(2)).sum();
        (sum / (b - a) as f64).sqrt()
    };
    for start in [0.1, 1.1] {
        assert!(rms(start) < 0.01, "silence at {start}");
    }
    for start in [0.55, 1.55] {
        assert!(rms(start) > 0.1, "tone at {start}");
    }
}

#[test]
fn timeline_and_project_keep_duration_sound_and_authored_sources() {
    if !enabled() {
        return;
    }
    let dir = folder("video");
    run(&dir, &["project", "create", "."]);
    std::fs::write(dir.join("scene.motion.tsx"), SCENE).unwrap();
    sound(&dir.join("tone.wav"));
    let doc = json!({"canvas":{"width":128,"height":64,"fps":4},
        "resources":{"scene":"scene.motion.tsx","tone":"tone.wav"},
        "tracks":{"visual":[{"items":[{"id":"a","kind":"motion","component":"scene","duration":2}]}],
        "audio":[{"clips":[{"src":"tone","start":0,"duration":2}]}]}}).to_string();
    std::fs::write(dir.join("main.timeline.json"), &doc).unwrap();
    for (fps, encoder) in [("8", "ffmpeg"), ("30000/1001", "ffmpeg"), ("60", "vt")] {
        if encoder == "vt" && !cfg!(target_os = "macos") {
            continue;
        }
        let rate = Rate::parse(fps).unwrap();
        let value = run(
            &dir,
            &[
                "timeline",
                "render",
                "main.timeline.json",
                "--fps",
                fps,
                "--encoder",
                encoder,
                "-o",
                "video.mp4",
            ],
        );
        assert_eq!(value["fps"], fps);
        assert_eq!(
            value["frames"],
            rate.frames_in(Tick::parse_seconds("2").unwrap())
        );
        video(&dir.join("video.mp4"), rate);
    }
    let result = run(
        &dir,
        &["project", "render", "--fps", "6", "--encoder", "ffmpeg"],
    );
    assert_eq!(result["fps"], "6");
    assert_eq!(result["frames"], 12);
    video(
        Path::new(result["file"].as_str().unwrap()),
        Rate::new(6, 1).unwrap(),
    );
    let snapshot = result["snapshot"].as_u64().unwrap();
    let records: Value = serde_json::from_slice(
        &std::fs::read(dir.join(format!(".valle/history/{snapshot}/renders.json"))).unwrap(),
    )
    .unwrap();
    assert!(
        records[0]["options"]
            .as_array()
            .unwrap()
            .contains(&json!(["fps", "6"]))
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("main.timeline.json")).unwrap(),
        doc
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("scene.motion.tsx")).unwrap(),
        SCENE
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn audio_only_output_rejects_a_frame_rate_before_loading() {
    let dir = folder("wav");
    let result = invoke(
        &dir,
        &[
            "timeline",
            "render",
            "missing.timeline.json",
            "--fps",
            "60",
            "-o",
            "audio.wav",
        ],
    );
    assert_eq!(result["code"], "usage");
    assert!(result["message"].as_str().unwrap().contains("--fps"));
    assert!(!dir.join("audio.wav").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
