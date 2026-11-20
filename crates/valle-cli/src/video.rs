//! Frames encoded into a video: H.264 or HEVC in a .mp4 or .mov, or ProRes 4444 with its alpha
//! in a .mov, by the machine's hardware encoder or FFmpeg's own; a timeline's sound beside them,
//! or alone in a .wav.

use std::collections::VecDeque;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use valle_engine::audio::Mixer;
use valle_engine::audio::loudness::Meter;
use valle_engine::gpu::Renderer;
use valle_engine::scene::Scene;
use valle_native::media::{self, AudioOptions, Codec, Encoding, Options, RATE, Textures};

use crate::Failure;
use crate::args::Args;
use crate::motion::{chosen, counted};

pub fn render(args: &Args, scene: &mut Scene, out: &Path) -> Result<Value, Failure> {
    let mov = (out.extension()).is_some_and(|e| e.eq_ignore_ascii_case("mov"));
    let codec = match args.codec {
        None if mov => Codec::ProRes4444,
        None => Codec::H264,
        Some(codec) => codec,
    };
    let failed = |e: String| Failure::new("video", e);
    let encoding = match args.encoder {
        Some(encoding) => encoding,
        // The hardware's for H.264 and HEVC where it has them; ProRes 4444 keeps its alpha
        // with FFmpeg's.
        None if codec == Codec::ProRes4444 => Encoding::Software,
        None => match media::has(codec, Encoding::Hardware).map_err(failed)? {
            true => Encoding::Hardware,
            false => Encoding::Software,
        },
    };
    let bitrate = args.bitrate;
    let c = scene.composition();
    let size = args.output_size([c.width, c.height])?;
    if codec != Codec::ProRes4444 && size.iter().any(|v| v % 2 != 0) {
        return Err(Failure::new(
            "usage",
            "H.264 and HEVC need even output dimensions",
        ));
    }
    let count = c.fps.frames_in(c.duration);
    let frames = chosen(args, count)?;
    // Sound goes with all the frames only, which it lines up with.
    let full = frames.iter().copied().eq(0..count);
    if !full && args.audio.configured() {
        return Err(Failure::new(
            "usage",
            "audio settings need a full-length video export",
        ));
    }
    let sound = (args.timeline && full && !args.audio.no_audio)
        .then(|| args.audio.resolve(if mov { "mov" } else { "mp4" }))
        .transpose()?;
    let mut mixer = sound
        .map(|audio| crate::timeline::mixer(args).map(|m| (m, audio.channels)))
        .transpose()?;
    let mut meter = Meter::default();
    let started = Instant::now();
    let (gpu, adapter) = crate::gpu()?;
    // On a Metal GPU, frames are drawn into the hardware encoder's own where it has them.
    let textures = Textures::new(gpu.device());
    let options = Options {
        size,
        fps: [c.fps.num(), c.fps.den()],
        codec,
        encoding,
        bitrate,
        sound,
        surfaces: textures.is_some(),
    };
    let mut renderer =
        Renderer::at_size(&gpu, [c.width, c.height], size).map_err(|e| Failure::new("gpu", e))?;
    let mut writer = media::writer(out, &options).map_err(failed)?;
    let gpu_failed = |e| Failure::new("gpu", e);
    // Frames the GPU is drawing into the encoder's, sent in order once it is done: one is drawn
    // while the next is laid out.
    let mut drawing = VecDeque::new();
    let mut direct = false;
    for (k, &i) in frames.iter().enumerate() {
        let at = c.fps.frame_start(i).map_err(|e| Failure::new("scene", e))?;
        let frame = {
            let _span = valle_engine::perf::span("eval");
            scene.frame(at).map_err(|e| Failure::new("scene", e))?
        };
        let (items, pictures) = (&frame.items, &frame.pictures);
        match (&textures, writer.surface().map_err(failed)?) {
            (Some(textures), Some(surface)) => {
                let _span = valle_engine::perf::span("draw");
                let planes = (textures.planes(gpu.device(), &surface)).map_err(failed)?;
                let views = [&planes.luma, &planes.chroma];
                let done = renderer
                    .planes(items, pictures, scene.faces(), scene.images(), views)
                    .map_err(gpu_failed)?;
                drawing.push_back((done, surface, planes));
                direct = true;
            }
            _ => {
                let pixels = {
                    let _span = valle_engine::perf::span("draw");
                    renderer
                        .render(items, pictures, scene.faces(), scene.images())
                        .map_err(gpu_failed)?
                };
                // Converted and encoded here, on this thread, as `draw` would hide it.
                let _span = valle_engine::perf::span("encoder");
                writer.push(&pixels).map_err(failed)?;
            }
        }
        if drawing.len() > 1
            && let Some((done, surface, planes)) = drawing.pop_front()
        {
            {
                let _span = valle_engine::perf::span("gpu-wait");
                gpu.wait(done).map_err(gpu_failed)?;
            }
            drop(planes);
            let _span = valle_engine::perf::span("encoder");
            writer.send(surface).map_err(failed)?;
        }
        if let Some((m, channels)) = &mut mixer {
            // The samples from this frame's start to the next's.
            let at = |f: i64| f * i64::from(RATE) * c.fps.den() / c.fps.num();
            let mut samples = vec![0.0; 2 * (at(i + 1) - at(i)) as usize];
            m.render(&mut samples);
            writer.sound(&samples).map_err(failed)?;
            measure_mix(&mut meter, &mut samples, *channels);
        }
        crate::progress(k + 1, frames.len());
    }
    for (done, surface, planes) in drawing {
        gpu.wait(done).map_err(gpu_failed)?;
        drop(planes);
        writer.send(surface).map_err(failed)?;
    }
    writer.finish().map_err(failed)?;
    valle_engine::perf::gauge("gpu_kb", renderer.bytes() / 1024);
    valle_engine::perf::gauge("frames", frames.len() as u64);
    valle_engine::perf::gauge("traces", scene.traces().0 as u64);
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
        "fps": c.fps.to_string(),
        "width": size[0],
        "height": size[1],
        "codec": format!("{codec:?}").to_lowercase(),
        "encoder": encoder,
        "adapter": adapter,
        // Whether the GPU drew the frames into the encoder's own memory, read back otherwise.
        "direct": direct,
        "seconds": (seconds * 1000.0).round() / 1000.0,
        // Silence has no loudness: a video without sound says none.
        "audio": options.sound.map(audio_info),
        "sound": mixer.is_some().then(|| loudness(&meter)).filter(|l| !l["lufs"].is_null()),
    }))
}

fn audio_info(o: AudioOptions) -> Value {
    json!({ "codec": o.codec.name(), "sampleRate": o.rate, "channels": o.channels,
        "bitrate": o.bitrate.map(u64::from).or_else(|| o.codec.pcm_bits().map(|b| u64::from(b) * u64::from(o.channels) * u64::from(o.rate))) })
}

/// Stream PCM into a WAV, then patch the header with the number of bytes actually converted.
pub fn sound(mixer: &mut Mixer, out: &Path, options: AudioOptions) -> Result<Value, Failure> {
    let bits = options
        .codec
        .pcm_bits()
        .ok_or_else(|| Failure::new("usage", "WAV needs PCM"))?;
    let stride = u16::from(options.channels) * (bits / 8);
    let length = mixer.length() as u64;
    let frames = (u128::from(length) * u128::from(options.rate)).div_ceil(u128::from(RATE));
    if frames * u128::from(stride) + 36 > u128::from(u32::MAX) {
        return Err(Failure::new(
            "write",
            "audio exceeds the 4 GiB RIFF WAV size limit",
        ));
    }
    let bytes_per_second = options
        .rate
        .checked_mul(u32::from(stride))
        .ok_or_else(|| Failure::new("write", "WAV byte rate exceeds 32 bits"))?;
    let mut pcm = media::OutputPcm::new(options).map_err(|e| Failure::new("sound", e))?;
    let failed = |e: std::io::Error| Failure::new("write", format!("{}: {e}", out.display()));
    let mut file = BufWriter::new(std::fs::File::create(out).map_err(failed)?);
    let header = |data: u32| {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&(if bits == 32 { 3u16 } else { 1u16 }).to_le_bytes());
        wav.extend_from_slice(&u16::from(options.channels).to_le_bytes());
        wav.extend_from_slice(&options.rate.to_le_bytes());
        wav.extend_from_slice(&bytes_per_second.to_le_bytes());
        wav.extend_from_slice(&stride.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        wav
    };
    file.write_all(&header(0)).map_err(failed)?;
    let mut meter = Meter::default();
    let mut data = 0u32;
    let mut remaining = length;
    let block_size = options.input_block();
    let mut block = vec![0.0; block_size * 2];
    let mut bytes = Vec::new();
    loop {
        let n = remaining.min(block_size as u64) as usize;
        let input = &mut block[..2 * n];
        if n > 0 {
            mixer.render(input);
        }
        let samples = pcm
            .convert((n > 0).then_some(input))
            .map_err(|e| Failure::new("sound", e))?;
        bytes.clear();
        for &sample in samples.iter() {
            if bits == 32 {
                bytes.extend_from_slice(&sample.to_le_bytes());
            } else {
                bytes.extend_from_slice(&media::pcm_i16(sample).to_le_bytes());
            }
        }
        measure_mix(&mut meter, input, options.channels);
        file.write_all(&bytes).map_err(failed)?;
        data += bytes.len() as u32;
        if n == 0 {
            break;
        }
        remaining -= n as u64;
    }
    file.seek(SeekFrom::Start(0)).map_err(failed)?;
    file.write_all(&header(data)).map_err(failed)?;
    file.flush().map_err(failed)?;
    let seconds = length as f64 / f64::from(RATE);
    Ok(json!({
        "status":"ok", "summary":format!("{seconds:.2} s of sound written to {}", out.display()),
        "file":out.display().to_string(), "audio":audio_info(options), "sound":loudness(&meter),
    }))
}

/// BS.1770 measures the channels delivered. The meter runs on 48 kHz stereo; for mono
/// its second channel is silent. Reuse the mixer block once the writer has consumed it.
fn measure_mix(meter: &mut Meter, samples: &mut [f32], channels: u8) {
    if channels == 1 {
        for pair in samples.as_chunks_mut::<2>().0 {
            *pair = [(pair[0] + pair[1]) * 0.5, 0.0];
        }
    }
    meter.push(samples);
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
