//! Version-independent audio interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::{frame::AudioBuffer, transport::AudioSource};
use anyhow::Result;
use std::path::Path;
backend_type!(LibavAudioSource, audio);
impl LibavAudioSource {
    pub fn open(path: &Path, out_rate: u32, out_channels: u16) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::audio::LibavAudioSource::open(path, out_rate, out_channels)
                .map(Into::into),
            Version::V8 => backend::v8::audio::LibavAudioSource::open(path, out_rate, out_channels)
                .map(Into::into),
            Version::V9 => backend::v9::audio::LibavAudioSource::open(path, out_rate, out_channels)
                .map(Into::into),
        }
    }
    pub fn buf_high_water(&self) -> usize {
        match &self.0 {
            Inner::V7(inner) => inner.buf_high_water(),
            Inner::V8(inner) => inner.buf_high_water(),
            Inner::V9(inner) => inner.buf_high_water(),
        }
    }
}
backend_type!(LibavAudioStream, audio);
impl LibavAudioStream {
    pub fn open(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::audio::LibavAudioStream::open(path, sample_rate, channels)
                .map(Into::into),
            Version::V8 => backend::v8::audio::LibavAudioStream::open(path, sample_rate, channels)
                .map(Into::into),
            Version::V9 => backend::v9::audio::LibavAudioStream::open(path, sample_rate, channels)
                .map(Into::into),
        }
    }
    pub fn sample_rate(&self) -> u32 {
        match &self.0 {
            Inner::V7(inner) => inner.sample_rate(),
            Inner::V8(inner) => inner.sample_rate(),
            Inner::V9(inner) => inner.sample_rate(),
        }
    }
    pub fn channels(&self) -> u16 {
        match &self.0 {
            Inner::V7(inner) => inner.channels(),
            Inner::V8(inner) => inner.channels(),
            Inner::V9(inner) => inner.channels(),
        }
    }
    pub fn frame_count_hint(&self) -> Option<u64> {
        match &self.0 {
            Inner::V7(inner) => inner.frame_count_hint(),
            Inner::V8(inner) => inner.frame_count_hint(),
            Inner::V9(inner) => inner.frame_count_hint(),
        }
    }
    pub fn total_frames(&self) -> Option<u64> {
        match &self.0 {
            Inner::V7(inner) => inner.total_frames(),
            Inner::V8(inner) => inner.total_frames(),
            Inner::V9(inner) => inner.total_frames(),
        }
    }
    pub fn position(&self) -> u64 {
        match &self.0 {
            Inner::V7(inner) => inner.position(),
            Inner::V8(inner) => inner.position(),
            Inner::V9(inner) => inner.position(),
        }
    }
    pub fn read(&mut self, maximum_frames: usize) -> Result<AudioBuffer> {
        match &mut self.0 {
            Inner::V7(inner) => inner.read(maximum_frames),
            Inner::V8(inner) => inner.read(maximum_frames),
            Inner::V9(inner) => inner.read(maximum_frames),
        }
    }
}
pub fn decode_audio_mono_f32(path: &Path, sample_rate: u32) -> Result<Vec<f32>> {
    match backend::version()? {
        Version::V7 => backend::v7::audio::decode_audio_mono_f32(path, sample_rate),
        Version::V8 => backend::v8::audio::decode_audio_mono_f32(path, sample_rate),
        Version::V9 => backend::v9::audio::decode_audio_mono_f32(path, sample_rate),
    }
}
impl AudioSource for LibavAudioSource {
    fn samples(&mut self, t: f64, dt: f64) -> Result<AudioBuffer> {
        match &mut self.0 {
            Inner::V7(inner) => inner.samples(t, dt),
            Inner::V8(inner) => inner.samples(t, dt),
            Inner::V9(inner) => inner.samples(t, dt),
        }
    }
    fn samples_by_index(&mut self, s0: i64, s1: i64) -> Result<AudioBuffer> {
        match &mut self.0 {
            Inner::V7(inner) => inner.samples_by_index(s0, s1),
            Inner::V8(inner) => inner.samples_by_index(s0, s1),
            Inner::V9(inner) => inner.samples_by_index(s0, s1),
        }
    }
}
