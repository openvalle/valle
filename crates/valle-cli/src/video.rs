//! Frames encoded into a video: H.264 or HEVC in a .mp4 or .mov, or ProRes 4444 with its alpha
//! in a .mov, by the machine's hardware encoder or FFmpeg's own; a timeline's sound beside them,
//! or alone in a .wav.

use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use valle_engine::audio::Mixer;
use valle_engine::audio::loudness::Meter;
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::scene::Scene;
use valle_native::media::{self, Codec, Encoding, Options, RATE};

use crate::Failure;
use crate::args::Args;
use crate::motion::{chosen, counted, draw};

pub fn render(
    args: &Args,
    scene: &mut Scene,
    out: &Path,
    mut mixer: Option<&mut Mixer>,
) -> Result<Value, Failure> {
    let mov = (out.extension()).is_some_and(|e| e.eq_ignore_ascii_case("mov"));
    let codec = match args.option("codec") {
        None if mov => Codec::ProRes4444,
        None | Some("h264") => Codec::H264,
        Some("hevc") => Codec::Hevc,
        Some("prores") => Codec::ProRes4444,
        Some(other) => {
            return Err(Failure::new("usage", format!("no codec {other}"))
                .hint("--codec is h264, hevc or prores"));
        }
    };
    let failed = |e: String| Failure::new("video", e);
    let encoding = match args.option("encoder") {
        Some("hardware" | "vt") => Encoding::Hardware,
        Some("software" | "ffmpeg") => Encoding::Software,
        Some(other) => {
            return Err(Failure::new("usage", format!("no encoder {other}"))
                .hint("--encoder is vt (the hardware's) or ffmpeg (FFmpeg's own, on the CPU)"));
        }
        // The hardware's for H.264 and HEVC where it has them; ProRes 4444 keeps its alpha
        // with FFmpeg's.
        None if codec == Codec::ProRes4444 => Encoding::Software,
        None => match media::has(codec, Encoding::Hardware).map_err(failed)? {
            true => Encoding::Hardware,
            false => Encoding::Software,
        },
    };
    let bitrate = match args.option("bitrate") {
        None => None,
        Some(b) => Some(bits(b).ok_or_else(|| {
            Failure::new(
                "usage",
                format!("--bitrate is bits a second, such as 8M, not {b}"),
            )
        })?),
    };
    let c = scene.composition();
    let count = c.fps.frames_in(c.duration);
    let frames = chosen(args, count)?;
    // Sound goes with all the frames only, which it lines up with.
    if !frames.iter().copied().eq(0..count) {
        mixer = None;
    }
    let options = Options {
        size: [c.width, c.height],
        fps: [c.fps.num(), c.fps.den()],
        codec,
        encoding,
        bitrate,
        sound: mixer.is_some(),
    };
    let mut meter = Meter::default();
    let started = Instant::now();
    let mut writer = media::writer(out, &options).map_err(failed)?;
    let gpu = Gpu::new().map_err(|e| Failure::new("gpu", e))?;
    let mut renderer = Renderer::new(&gpu, c.width, c.height);
    for (k, &i) in frames.iter().enumerate() {
        let pixels = draw(scene, &mut renderer, c.fps.frame_start(i))?;
        writer.push(&pixels).map_err(failed)?;
        if let Some(m) = mixer.as_deref_mut() {
            // The samples from this frame's start to the next's.
            let at = |f: i64| f * i64::from(RATE) * c.fps.den() / c.fps.num();
            let mut samples = vec![0.0; 2 * (at(i + 1) - at(i)) as usize];
            m.render(&mut samples);
            meter.push(&samples);
            writer.sound(&samples).map_err(failed)?;
        }
        crate::progress(k + 1, frames.len());
    }
    writer.finish().map_err(failed)?;
    let seconds = started.elapsed().as_secs_f64();
    let encoder = match encoding {
        Encoding::Hardware => "hardware",
        Encoding::Software => "ffmpeg",
    };
    Ok(json!({
        "status": "ok",
        "summary": format!(
            "{} encoded into {} in {seconds:.1} s",
            counted(frames.len(), "frame"),
            out.display()
        ),
        "file": out.display().to_string(),
        "frames": frames.len(),
        "codec": format!("{codec:?}").to_lowercase(),
        "encoder": encoder,
        "seconds": seconds,
        "sound": mixer.is_some().then(|| loudness(&meter)),
    }))
}

/// A timeline's sound alone into a .wav: 32-bit float, stereo, at 48 kHz.
pub fn sound(mixer: &mut Mixer, out: &Path) -> Result<Value, Failure> {
    let mut samples = vec![0.0; 2 * mixer.length() as usize];
    mixer.render(&mut samples);
    let mut meter = Meter::default();
    meter.push(&samples);
    let data = samples.len() * 4;
    let mut wav = Vec::with_capacity(44 + data);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    // 16 bytes of format: IEEE float, 2 channels, the rate, bytes a second, a frame, bits.
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 8).to_le_bytes());
    wav.extend_from_slice(&8u16.to_le_bytes());
    wav.extend_from_slice(&32u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data as u32).to_le_bytes());
    for s in &samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(out, wav)
        .map_err(|e| Failure::new("write", format!("{}: {e}", out.display())))?;
    let seconds = mixer.length() as f64 / f64::from(RATE);
    Ok(json!({
        "status": "ok",
        "summary": format!("{seconds:.2} s of sound written to {}", out.display()),
        "file": out.display().to_string(),
        "sound": loudness(&meter),
    }))
}

/// What a sound measured: its integrated loudness (BS.1770, LUFS) and true peak (dBTP), which
/// its gains are set by.
fn loudness(meter: &Meter) -> Value {
    let l = meter.result();
    let finite = |x: f64| x.is_finite().then_some((x * 10.0).round() / 10.0);
    json!({
        "lufs": finite(l.integrated),
        "truePeak": finite(l.true_peak),
    })
}

/// `8M`, `800k` or a number of bits a second.
fn bits(text: &str) -> Option<u64> {
    let (number, scale) = match text.as_bytes().last()? {
        b'k' | b'K' => (&text[..text.len() - 1], 1e3),
        b'm' | b'M' => (&text[..text.len() - 1], 1e6),
        _ => (text, 1.0),
    };
    let n: f64 = number.parse().ok()?;
    (n > 0.0).then_some((n * scale) as u64)
}
