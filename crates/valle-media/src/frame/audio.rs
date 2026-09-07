//! Interleaved f32 audio samples and deterministic channel/mixing primitives. Mix by linear
//! summation and hard clipping to [-1,1], without implicit normalization. Mono-to-stereo duplicates
//! samples; stereo-to-mono averages channels.

/// Equal left/right gains for stereo-to-mono averaging.
const STEREO_TO_MONO_GAIN: f32 = 0.5;

/// Interleaved f32 audio with samples.len() equal to frames times channels.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// Frame-major interleaved samples, conventionally in [-1,1].
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioBuffer {
    /// Create a silent buffer.
    pub fn silence(sample_rate: u32, channels: u16, frames: usize) -> Self {
        AudioBuffer {
            samples: vec![0.0; frames * channels as usize],
            sample_rate,
            channels,
        }
    }

    /// Sample-frame count, with one sample per channel in each frame.
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// Mix matching rates and channel counts by summation and hard clipping. Pad the shorter buffer
    /// with silence; incompatible formats return None.
    pub fn try_mix(&self, other: &AudioBuffer) -> Option<AudioBuffer> {
        if self.sample_rate != other.sample_rate || self.channels != other.channels {
            return None;
        }
        let n = self.samples.len().max(other.samples.len());
        let mut out = vec![0.0f32; n];
        for (i, o) in out.iter_mut().enumerate() {
            let a = self.samples.get(i).copied().unwrap_or(0.0);
            let b = other.samples.get(i).copied().unwrap_or(0.0);
            *o = (a + b).clamp(-1.0, 1.0);
        }
        Some(AudioBuffer {
            samples: out,
            sample_rate: self.sample_rate,
            channels: self.channels,
        })
    }

    /// Duplicate mono into stereo and average stereo into mono. Other channel-count conversions
    /// deterministically repeat the source-channel average.
    pub fn to_channels(&self, target: u16) -> AudioBuffer {
        if target == 0 || self.channels == 0 || target == self.channels {
            return self.clone();
        }
        let frames = self.frames();
        let from = self.channels as usize;
        let to = target as usize;
        let mut out = Vec::with_capacity(frames * to);
        for f in 0..frames {
            let base = f * from;
            let src = &self.samples[base..base + from];
            match (from, to) {
                (1, 2) => {
                    out.push(src[0]);
                    out.push(src[0]);
                }
                (2, 1) => out.push(STEREO_TO_MONO_GAIN * (src[0] + src[1])),
                _ => {
                    let mono = src.iter().sum::<f32>() / from as f32;
                    for _ in 0..to {
                        out.push(mono);
                    }
                }
            }
        }
        AudioBuffer {
            samples: out,
            sample_rate: self.sample_rate,
            channels: target,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_and_frames() {
        let b = AudioBuffer::silence(48_000, 2, 100);
        assert_eq!(b.frames(), 100);
        assert!(b.samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn mix_is_linear_sum_with_hard_clamp() {
        let a = AudioBuffer {
            samples: vec![0.6, 0.6],
            sample_rate: 48_000,
            channels: 2,
        };
        let b = AudioBuffer {
            samples: vec![0.6, -0.9],
            sample_rate: 48_000,
            channels: 2,
        };
        let m = a.try_mix(&b).unwrap();
        assert_eq!(m.samples[0], 1.0); // Clip the summed sample without normalization.
        assert!((m.samples[1] - (-0.3)).abs() < 1e-6);
    }

    #[test]
    fn mix_rejects_mismatched_format() {
        let a = AudioBuffer::silence(48_000, 2, 4);
        let b = AudioBuffer::silence(44_100, 2, 4);
        assert!(a.try_mix(&b).is_none());
    }

    #[test]
    fn pan_law_mono_to_stereo_and_back() {
        let mono = AudioBuffer {
            samples: vec![0.5, -0.25],
            sample_rate: 48_000,
            channels: 1,
        };
        let st = mono.to_channels(2);
        assert_eq!(st.channels, 2);
        assert_eq!(st.samples, vec![0.5, 0.5, -0.25, -0.25]); // Duplicate at unity gain.

        let stereo = AudioBuffer {
            samples: vec![1.0, 0.0, 0.4, 0.6],
            sample_rate: 48_000,
            channels: 2,
        };
        let m = stereo.to_channels(1);
        assert_eq!(m.samples, vec![0.5, 0.5]); // 0.5(L+R)
    }
}
