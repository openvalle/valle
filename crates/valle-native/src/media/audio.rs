//! Settings for an exported audio track. The mixer supplies 48 kHz interleaved stereo.

use std::borrow::Cow;

use super::{RATE, Version, v7, v8, v9, version};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    Aac,
    Opus,
    PcmS16,
    PcmF32,
}

impl AudioCodec {
    pub fn name(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Opus => "opus",
            Self::PcmS16 => "pcm-s16le",
            Self::PcmF32 => "pcm-f32le",
        }
    }

    pub fn pcm_bits(self) -> Option<u16> {
        match self {
            Self::PcmS16 => Some(16),
            Self::PcmF32 => Some(32),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AudioOptions {
    pub codec: AudioCodec,
    pub rate: u32,
    pub channels: u8,
    /// Target bits per second for compressed codecs.
    pub bitrate: Option<u32>,
}

impl AudioOptions {
    pub fn for_container(ext: &str) -> Self {
        let codec = match ext {
            "wav" => AudioCodec::PcmF32,
            "mov" => AudioCodec::PcmS16,
            _ => AudioCodec::Aac,
        };
        Self {
            codec,
            rate: RATE,
            channels: 2,
            bitrate: codec.pcm_bits().is_none().then_some(256_000),
        }
    }

    /// Keep both input and converted blocks small when upsampling PCM.
    pub fn input_block(self) -> usize {
        (4096u64 * u64::from(RATE) / u64::from(self.rate)).clamp(1, 4096) as usize
    }

    pub fn validate(&self, ext: &str) -> Result<(), String> {
        use AudioCodec::*;
        if !matches!(
            (ext, self.codec),
            ("mp4", Aac | Opus) | ("mov", Aac | PcmS16) | ("wav", PcmS16 | PcmF32)
        ) {
            return Err(format!(
                "{} audio cannot be exported to .{ext}",
                self.codec.name()
            ));
        }
        if !matches!(self.channels, 1 | 2) || self.rate == 0 || self.rate > i32::MAX as u32 {
            return Err("audio needs 1 or 2 channels and a positive 32-bit sample rate".into());
        }
        if self.codec == Opus && self.rate != RATE {
            return Err("an Opus track in MP4 has a 48000 Hz clock; select 48000 Hz or AAC".into());
        }
        if self.codec.pcm_bits().is_some() && self.bitrate.is_some() {
            return Err("PCM bitrate is set by its bit depth, sample rate and channels; omit --audio-bitrate".into());
        }
        if self.bitrate == Some(0) {
            return Err("audio bitrate must be positive".into());
        }
        Ok(())
    }
}

/// Changes the mix rate only when needed. The default export borrows the original block.
pub struct OutputPcm {
    resampler: Option<Box<dyn Resample>>,
    channels: u8,
}

pub(super) trait Resample {
    fn convert(&mut self, samples: Option<&[f32]>) -> Result<Vec<f32>, String>;
}

impl OutputPcm {
    pub fn new(options: AudioOptions) -> Result<Self, String> {
        let resampler: Option<Box<dyn Resample>> = if options.rate == RATE {
            None
        } else {
            Some(match version()? {
                Version::V7 => Box::new(v7::sound::OutputRate::new(options.rate)?),
                Version::V8 => Box::new(v8::sound::OutputRate::new(options.rate)?),
                Version::V9 => Box::new(v9::sound::OutputRate::new(options.rate)?),
            })
        };
        Ok(Self {
            resampler,
            channels: options.channels,
        })
    }

    /// Pass None at end of stream to drain any delayed samples.
    pub fn convert<'a>(&mut self, samples: Option<&'a [f32]>) -> Result<Cow<'a, [f32]>, String> {
        if samples.is_some_and(|s| !s.len().is_multiple_of(2)) {
            return Err("stereo sound needs a left and right sample for each time".into());
        }
        let stereo = match &mut self.resampler {
            Some(r) => Cow::Owned(r.convert(samples)?),
            None => Cow::Borrowed(samples.unwrap_or_default()),
        };
        Ok(if self.channels == 1 {
            Cow::Owned(
                stereo
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| (s[0] + s[1]) * 0.5)
                    .collect(),
            )
        } else {
            stereo
        })
    }
}

/// Signed 16-bit PCM, clipping values outside its range.
pub fn pcm_i16(value: f32) -> i16 {
    (value * 32768.0).round_ties_even() as i16
}
