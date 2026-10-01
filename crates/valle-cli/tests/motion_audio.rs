//! audio-analysis end-to-end: frozen WAV bytes drive random-access Motion frames.

use std::{path::Path, process::Command};

const SOURCE: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 8, fps: 30 });
export default function AudioBars(ctx) {
  return <Scene style={{width:320,height:180}}>
    <View key="bar" style={{position:"absolute",left:20,top:20,width:30,
      height:BEAT.level(ctx.seconds)*100,backgroundColor:"#ffffff"}} />
  </Scene>;
}
"##;

fn write_wav(path: &Path, silence_first: bool) {
    let rate = 48_000_u32;
    let frames = rate * 2;
    let bytes = frames * 2;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for index in 0..frames {
        let silent = (index < rate) == silence_first;
        let value = if silent {
            0
        } else {
            ((std::f64::consts::TAU * 440.0 * f64::from(index % rate) / f64::from(rate)).sin()
                * 24_000.0) as i16
        };
        wav.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

#[test]
fn synthetic_audio_changes_the_bar_and_reversed_signal_reverses_the_frames() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("audio.motion.tsx"), SOURCE).unwrap();
    write_wav(&dir.path().join("beat.wav"), true);
    write_wav(&dir.path().join("beat-reversed.wav"), false);
    let output = Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir.path())
        .args([
            "--json",
            "motion",
            "render",
            "audio.motion.tsx",
            "--asset",
            "beat=beat.wav",
            "--frames",
            "45,15",
            "--storyboard",
            "reverse.png",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let sheet = image::open(dir.path().join("reverse.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(sheet.dimensions(), (640, 180));
    let white_pixels = |left: u32| {
        (left..left + 320)
            .flat_map(|x| (0..180).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let pixel = sheet.get_pixel(x, y);
                pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240 && pixel[3] > 240
            })
            .count()
    };
    assert!(white_pixels(0) > 1000);
    assert_eq!(white_pixels(320), 0);

    let reversed = Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir.path())
        .args([
            "--json",
            "motion",
            "render",
            "audio.motion.tsx",
            "--asset",
            "beat=beat-reversed.wav",
            "--frames",
            "45,15",
            "--storyboard",
            "reversed-signal.png",
        ])
        .output()
        .unwrap();
    assert!(
        reversed.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&reversed.stdout),
        String::from_utf8_lossy(&reversed.stderr)
    );
    let other = image::open(dir.path().join("reversed-signal.png"))
        .unwrap()
        .to_rgba8();
    let count = |left: u32| {
        (left..left + 320)
            .flat_map(|x| (0..180).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let pixel = other.get_pixel(x, y);
                pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240 && pixel[3] > 240
            })
            .count()
    };
    assert_eq!(count(0), 0);
    assert!(count(320) > 1000);
}
