//! Version-independent flac interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::frame::AudioBuffer;
use anyhow::Result;
use std::path::Path;
pub const FLAC_BITS_PER_SAMPLE: u8 = 24;
pub const FLAC_QUANTIZATION_POLICY: &str = "non-finite f32 rejected; finite f32 clamped to [-1,1], then signed 24-bit PCM round-to-nearest (ties away from zero)";
backend_type!(FlacPcm24Writer, flac);
impl FlacPcm24Writer {
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::flac::FlacPcm24Writer::create(path, sample_rate, channels)
                .map(Into::into),
            Version::V8 => backend::v8::flac::FlacPcm24Writer::create(path, sample_rate, channels)
                .map(Into::into),
            Version::V9 => backend::v9::flac::FlacPcm24Writer::create(path, sample_rate, channels)
                .map(Into::into),
        }
    }
    pub fn write(&mut self, buffer: &AudioBuffer) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.write(buffer),
            Inner::V8(inner) => inner.write(buffer),
            Inner::V9(inner) => inner.write(buffer),
        }
    }
    pub fn finish(self) -> Result<u64> {
        match self.0 {
            Inner::V7(inner) => inner.finish(),
            Inner::V8(inner) => inner.finish(),
            Inner::V9(inner) => inner.finish(),
        }
    }
}
pub fn validate_flac_pcm24(
    path: &Path,
    expected_sample_rate: u32,
    expected_channels: u16,
    expected_frames: u64,
) -> Result<()> {
    match backend::version()? {
        Version::V7 => backend::v7::flac::validate_flac_pcm24(
            path,
            expected_sample_rate,
            expected_channels,
            expected_frames,
        ),
        Version::V8 => backend::v8::flac::validate_flac_pcm24(
            path,
            expected_sample_rate,
            expected_channels,
            expected_frames,
        ),
        Version::V9 => backend::v9::flac::validate_flac_pcm24(
            path,
            expected_sample_rate,
            expected_channels,
            expected_frames,
        ),
    }
}
