//! `valle media inspect` through the CLI: silence, loudness and cuts of files written here, the
//! measures cached for the next run with other thresholds, and its usage errors. Needs FFmpeg,
//! but no GPU: the sheet, which labels its frames on the GPU, is left out.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const VALLE: &str = env!("CARGO_BIN_EXE_valle");

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("valle-inspect-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn inspect(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(VALLE)
        .args(["media", "inspect"])
        .args(args)
        .arg("--json")
        .env("VALLE_CACHE_DIR", dir.join("cache"))
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{out:?}"))
}

/// A mono 16-bit WAV at 48 kHz.
fn wav(path: &Path, samples: &[f64]) {
    let bytes = samples.len() as u32 * 2;
    let mut out = Vec::from(*b"RIFF");
    out.extend((36 + bytes).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16_u32.to_le_bytes());
    out.extend(1_u16.to_le_bytes());
    out.extend(1_u16.to_le_bytes());
    out.extend(48_000_u32.to_le_bytes());
    out.extend(96_000_u32.to_le_bytes());
    out.extend(2_u16.to_le_bytes());
    out.extend(16_u16.to_le_bytes());
    out.extend(b"data");
    out.extend(bytes.to_le_bytes());
    out.extend(
        samples
            .iter()
            .flat_map(|s| ((s * 32767.0).round() as i16).to_le_bytes()),
    );
    std::fs::write(path, out).unwrap();
}

fn tone(seconds: f64, amplitude: f64) -> impl Iterator<Item = f64> {
    (0..(seconds * 48_000.0) as usize)
        .map(move |i| amplitude * (i as f64 * std::f64::consts::TAU * 1000.0 / 48_000.0).sin())
}

#[test]
fn sound_has_its_silences_and_loudness_and_they_are_cached() {
    let dir = Folder::new("sound");
    let path = dir.0.join("voice.wav");
    // 2 s of tone at -20 dBFS, 0.4 s of nothing, 2 s of tone, 1 s of nothing, 2 s of tone.
    let samples: Vec<f64> = tone(2.0, 0.1)
        .chain(tone(0.4, 0.0))
        .chain(tone(2.0, 0.1))
        .chain(tone(1.0, 0.0))
        .chain(tone(2.0, 0.1))
        .collect();
    wav(&path, &samples);
    let file = path.to_str().unwrap();
    let first = inspect(&dir.0, &[file]);
    assert_eq!(first["status"], "ok", "{first}");
    assert_eq!(first["cached"], false);
    assert_eq!(first["audio"], true);
    assert!(first.get("cuts").is_none() && first.get("width").is_none());
    let silences = first["silences"].as_array().unwrap();
    assert_eq!(silences.len(), 1, "{first}");
    assert!((silences[0]["start"].as_f64().unwrap() - 4.4).abs() < 0.011);
    assert!((silences[0]["end"].as_f64().unwrap() - 5.4).abs() < 0.011);
    // A 1 kHz sine at -20 dBFS peak is -23 dBFS RMS. BS.1770 counts a mono file as one
    // channel and takes 0.691 off, and the 1 kHz shelf adds back about 0.7: -23.4 LUFS, as
    // FFmpeg's ebur128 measures the same file.
    let loudness = first["loudness"]["integrated"].as_f64().unwrap();
    assert!((loudness + 23.4).abs() < 0.1, "{loudness}");
    let peak = first["loudness"]["truePeak"].as_f64().unwrap();
    assert!((peak + 20.0).abs() < 0.2, "{peak}");
    assert_eq!(first["loudness"]["seconds"].as_array().unwrap().len(), 8);

    let again = inspect(&dir.0, &[file, "--silence-min", "0.3"]);
    assert_eq!(again["cached"], true);
    assert_eq!(again["silences"].as_array().unwrap().len(), 2, "{again}");
    assert_eq!(again["loudness"], first["loudness"]);
    // A file changed is measured again.
    wav(&path, &samples[..48_000]);
    let changed = inspect(&dir.0, &[file]);
    assert_eq!(changed["cached"], false);
    // No silence is no list, as an empty result field is left out.
    assert!(changed.get("silences").is_none(), "{changed}");
}

#[test]
fn usage_errors_name_what_is_wrong() {
    let dir = Folder::new("usage");
    let path = dir.0.join("short.wav");
    wav(&path, &[0.0; 4800]);
    let file = path.to_str().unwrap();
    for (args, message) in [
        (vec![file, "--silence-below", "3"], "--silence-below"),
        (vec![file, "--silence-min", "0"], "--silence-min"),
        (vec![file, "--sheet", "sheet.jpg"], "--sheet"),
        (vec![file, "--sheet", "sheet.png"], "no video"),
    ] {
        let value = inspect(&dir.0, &args);
        assert_eq!(value["status"], "error", "{args:?}");
        assert!(
            value["message"].as_str().unwrap().contains(message),
            "{value}"
        );
    }
    let missing = inspect(&dir.0, &["missing.mp4"]);
    assert_eq!(missing["code"], "video");
}
