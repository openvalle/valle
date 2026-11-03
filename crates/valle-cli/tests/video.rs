//! `valle motion render` into a video, read back: each encoder's frames show what a PNG of the
//! frame shows, the hardware's on macOS drawn by the GPU into its own memory.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

/// Smooth gradients and flat colors, which a video keeps closely, moving with time.
const PICTURE: &str = r##"export const composition = { width: 256, height: 144, fps: 30, duration: 1 };
export default function Scene() {
  const { progress } = useCurrentTime();
  return (
    <View style={{ width: 256, height: 144, display: "flex", background: "linear-gradient(90deg, #1e3a8a, #f59e0b)" }}>
      <View style={{ width: 96, height: 96, margin: 24, marginLeft: 24 + progress * 64, background: "#e11d48", borderRadius: 48 }} />
      <View style={{ width: 64, height: 144, background: "linear-gradient(180deg, #ffffff, #10b981)" }} />
    </View>
  );
}
"##;

/// The video at `clip`, filling the frame.
const SHOWN: &str = r##"export const composition = { width: 256, height: 144, fps: 30, duration: 1 };
export const controls = { resources: { clip: resource({ kind: "video", default: "clip.mp4" }) } };
export default function Scene() {
  return <Video src="resource://clip" style={{ width: 256, height: 144 }} />;
}
"##;

fn valle(args: &[&str], dir: &Path) -> Value {
    let out = Command::new(VALLE)
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .output()
        .expect("valle runs");
    let value: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(value["status"], "ok", "valle {args:?}: {value}");
    value
}

fn pixels(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(path).expect("a PNG"),
    ));
    let mut reader = decoder.read_info().expect("a PNG");
    let mut data = vec![0; reader.output_buffer_size().expect("a size")];
    let info = reader.next_frame(&mut data).expect("a frame");
    data.truncate(info.buffer_size());
    data
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("valle-video-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    std::fs::write(dir.join("picture.motion.tsx"), PICTURE).expect("written");
    std::fs::write(dir.join("shown.motion.tsx"), SHOWN).expect("written");
    dir
}

/// Encodes the picture with `encoder`, then compares frame 15 of the video, decoded by FFmpeg's
/// own decoder, with the PNG: the most and the mean any channel differs by.
fn round_trip(encoder: &str) -> (Value, u8, f64) {
    let dir = folder(encoder);
    let rendered = valle(
        &["motion", "render", "picture.motion.tsx", "-o", "clip.mp4"]
            .into_iter()
            .chain(["--encoder", encoder, "--bitrate", "40M"])
            .collect::<Vec<_>>(),
        &dir,
    );
    valle(
        &[
            "motion",
            "render",
            "picture.motion.tsx",
            "--frame",
            "15",
            "-o",
            "a.png",
        ],
        &dir,
    );
    let decoder = ["--decoder", "ffmpeg"];
    let shown = [
        "motion",
        "render",
        "shown.motion.tsx",
        "--frame",
        "15",
        "-o",
        "b.png",
    ];
    valle(&[&shown[..], &decoder[..]].concat(), &dir);
    let (a, b) = (pixels(&dir.join("a.png")), pixels(&dir.join("b.png")));
    assert_eq!(a.len(), b.len());
    let (mut most, mut sum, mut n) = (0, 0u64, 0u64);
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        for k in 0..3 {
            let d = pa[k].abs_diff(pb[k]);
            most = most.max(d);
            sum += u64::from(d);
            n += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    (rendered, most, sum as f64 / n as f64)
}

#[test]
fn ffmpeg_encodes_what_the_frame_shows() {
    let (rendered, most, mean) = round_trip("ffmpeg");
    assert_eq!(rendered["direct"], false, "{rendered}");
    assert!(mean < 2.0 && most < 96, "most {most}, mean {mean:.2}");
}

#[test]
#[cfg(target_os = "macos")]
fn the_hardware_encodes_what_the_gpu_drew_into_it() {
    let (rendered, most, mean) = round_trip("vt");
    assert_eq!(rendered["direct"], true, "{rendered}");
    assert!(mean < 2.0 && most < 96, "most {most}, mean {mean:.2}");
}
