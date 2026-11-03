//! Frames encoded into a video: H.264 or HEVC in a .mp4 or .mov, or ProRes 4444 with its alpha
//! in a .mov, by the machine's hardware encoder or FFmpeg's own.

use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::scene::Scene;
use valle_native::media::{self, Codec, Encoding, Options};

use crate::Failure;
use crate::args::Args;
use crate::motion::{chosen, counted, draw};

pub fn render(args: &Args, scene: &mut Scene, out: &Path) -> Result<Value, Failure> {
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
    let options = Options {
        size: [c.width, c.height],
        fps: [c.fps.num(), c.fps.den()],
        codec,
        encoding,
        bitrate,
    };
    let count = c.fps.frames_in(c.duration);
    let frames = chosen(args, count)?;
    let started = Instant::now();
    let mut writer = media::writer(out, &options).map_err(failed)?;
    let gpu = Gpu::new().map_err(|e| Failure::new("gpu", e))?;
    let mut renderer = Renderer::new(&gpu, c.width, c.height);
    for &i in &frames {
        let pixels = draw(scene, &mut renderer, c.fps.frame_start(i))?;
        writer.push(&pixels).map_err(failed)?;
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
    }))
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
