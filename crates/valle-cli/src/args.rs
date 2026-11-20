//! A command's arguments: its file, its output, and the values bound to the scene.

use serde_json::{Map, Value};
use valle_engine::front::Inputs;
use valle_native::media::{AudioCodec, AudioOptions, Codec, Decoding, Encoding};
use valle_timeline::time::Rate;

use crate::Failure;

/// What `crate::cli` read for a command on a scene or a timeline.
#[derive(Default)]
pub struct Args {
    /// The scene, or the document, the command works on.
    pub file: String,
    /// Whether `file` is a timeline, not a scene.
    pub timeline: bool,
    pub out: Option<String>,
    pub inputs: Inputs,
    /// Explicit frames, otherwise all (or a storyboard's sample).
    pub frames: Option<Frames>,
    pub storyboard: bool,
    pub samples: Option<u32>,
    pub step: Option<u32>,
    pub per_page: Option<u32>,
    /// Output pixels, if not the canvas's.
    pub output_size: Option<[u32; 2]>,
    /// The output frame rate, without changing the scene's duration.
    pub fps: Option<Rate>,
    pub codec: Option<Codec>,
    pub encoder: Option<Encoding>,
    pub decoder: Option<Decoding>,
    /// A video's bits a second.
    pub bitrate: Option<u64>,
    pub audio: AudioFlags,
}

#[derive(clap::Args, Default)]
pub struct AudioFlags {
    /// Omit the audio track and skip mixing
    #[arg(long, conflicts_with_all = ["audio_codec", "audio_sample_rate", "audio_channels", "audio_bitrate"])]
    pub no_audio: bool,
    /// Audio encoding: aac, opus, pcm-s16le or pcm-f32le, depending on the container
    #[arg(long, value_parser = audio_codec)]
    audio_codec: Option<AudioCodec>,
    /// Audio samples per second (default 48000)
    #[arg(long, value_name = "HZ", value_parser = clap::value_parser!(u32).range(1..=i32::MAX as i64))]
    audio_sample_rate: Option<u32>,
    /// 1 for mono, 2 for stereo (default)
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    audio_channels: Option<u8>,
    /// Compressed audio's target bits per second, such as 128k
    #[arg(long, value_name = "BITS", value_parser = audio_bits)]
    audio_bitrate: Option<u32>,
}

fn audio_codec(s: &str) -> Result<AudioCodec, String> {
    [
        AudioCodec::Aac,
        AudioCodec::Opus,
        AudioCodec::PcmS16,
        AudioCodec::PcmF32,
    ]
    .into_iter()
    .find(|c| c.name() == s)
    .ok_or_else(|| "choose aac, opus, pcm-s16le or pcm-f32le".into())
}

fn audio_bits(s: &str) -> Result<u32, String> {
    u32::try_from(crate::cli::bits(s)?)
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "audio bitrate must be a positive 32-bit integer in bits per second".into())
}

impl AudioFlags {
    pub fn configured(&self) -> bool {
        self.audio_codec.is_some()
            || self.audio_sample_rate.is_some()
            || self.audio_channels.is_some()
            || self.audio_bitrate.is_some()
    }

    pub fn resolve(&self, ext: &str) -> Result<AudioOptions, Failure> {
        let defaults = AudioOptions::for_container(ext);
        let codec = self.audio_codec.unwrap_or(defaults.codec);
        let rate = self.audio_sample_rate.unwrap_or(defaults.rate);
        let channels = self.audio_channels.unwrap_or(defaults.channels);
        // AAC-LC fits at most 6144 bits per channel in a 1024-sample frame.
        let default_bitrate = match codec {
            AudioCodec::Aac => Some(256_000.min(rate.saturating_mul(6 * u32::from(channels)))),
            AudioCodec::Opus => Some(256_000),
            _ => None,
        };
        let settings = AudioOptions {
            codec,
            rate,
            channels,
            bitrate: self.audio_bitrate.or(default_bitrate),
        };
        settings
            .validate(ext)
            .map_err(|e| Failure::new("usage", e))?;
        Ok(settings)
    }
}

#[derive(Clone)]
pub enum Frames {
    List(Vec<i64>),
    /// From the first to the last, both drawn.
    Range(i64, i64),
}

impl Args {
    /// Reject incompatible output options before loading a scene or opening an encoder.
    pub fn validate_output(&self) -> Result<(), Failure> {
        if self.storyboard
            && self
                .out
                .as_ref()
                .is_some_and(|out| !out.to_ascii_lowercase().ends_with(".png"))
        {
            return Err(Failure::new("usage", "a storyboard is a .png"));
        }
        if self.fps.is_some()
            && self
                .out
                .as_ref()
                .is_some_and(|out| out.to_ascii_lowercase().ends_with(".wav"))
        {
            return Err(Failure::new(
                "usage",
                "--fps applies to images and video, not .wav",
            ));
        }
        if self.audio.configured() || self.audio.no_audio {
            if !self.timeline && self.audio.configured() {
                return Err(Failure::new(
                    "usage",
                    "a Motion scene has no audio track to configure",
                ));
            }
            if let Some(out) = &self.out {
                let ext = std::path::Path::new(out)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(ext.as_str(), "mp4" | "mov" | "wav") || self.storyboard {
                    return Err(Failure::new(
                        "usage",
                        "audio options apply to .mp4, .mov or .wav",
                    ));
                }
                if ext == "wav" && self.audio.no_audio {
                    return Err(Failure::new(
                        "usage",
                        "--no-audio cannot produce a WAV file",
                    ));
                }
                self.audio.resolve(&ext)?;
            }
        }
        Ok(())
    }

    /// The render options given, by their option's name: what a render record keeps.
    pub fn options(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some([w, h]) = self.output_size {
            out.push(("output-size", format!("{w}x{h}")));
        }
        if let Some(fps) = self.fps {
            out.push(("fps", fps.to_string()));
        }
        if let Some(codec) = self.codec {
            let name = match codec {
                Codec::H264 => "h264",
                Codec::Hevc => "hevc",
                Codec::ProRes4444 => "prores",
            };
            out.push(("codec", name.into()));
        }
        let coder = |hardware: bool| if hardware { "vt" } else { "ffmpeg" }.to_string();
        if let Some(e) = self.encoder {
            out.push(("encoder", coder(e == Encoding::Hardware)));
        }
        if let Some(d) = self.decoder {
            out.push(("decoder", coder(d == Decoding::Hardware)));
        }
        if let Some(bits) = self.bitrate {
            out.push(("bitrate", bits.to_string()));
        }
        if self.audio.no_audio {
            out.push(("no-audio", true.to_string()));
        }
        if let Some(c) = self.audio.audio_codec {
            out.push(("audio-codec", c.name().into()));
        }
        if let Some(v) = self.audio.audio_sample_rate {
            out.push(("audio-sample-rate", v.to_string()));
        }
        if let Some(v) = self.audio.audio_channels {
            out.push(("audio-channels", v.to_string()));
        }
        if let Some(v) = self.audio.audio_bitrate {
            out.push(("audio-bitrate", v.to_string()));
        }
        out
    }

    /// Output pixels, leaving the scene's logical canvas unchanged.
    pub fn output_size(&self, canvas: [u32; 2]) -> Result<[u32; 2], Failure> {
        let size = self.output_size.unwrap_or(canvas);
        valle_engine::gpu::output_size(size).map_err(|e| Failure::new("usage", e))?;
        Ok(size)
    }
}

/// JSON written out, or `@path` for a file of it.
pub fn json(text: &str, what: &str) -> Result<Value, Failure> {
    let text = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| Failure::new("not_found", format!("{what} {path}: {e}")))?,
        None => text.to_string(),
    };
    serde_json::from_str(&text).map_err(|e| Failure::new("usage", format!("{what}: {e}")))
}

pub fn object(text: &str, what: &str) -> Result<Map<String, Value>, Failure> {
    match json(text, what)? {
        Value::Object(o) => Ok(o),
        _ => Err(Failure::new(
            "usage",
            format!("{what} is a JSON object of values by name"),
        )),
    }
}
