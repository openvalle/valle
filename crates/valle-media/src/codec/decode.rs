//! Version-independent decode interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::codec::ffi::YuvMatrix;
use crate::frame::RgbaFrame;
use crate::transport::{
    SharedFrameBackend, SharedVideoFrameHandle, SourceFrame, SourceMeta, VideoFrameTransport,
    VideoSource,
};
use anyhow::Result;
use std::{path::Path, sync::Arc};
/// Container audio/video stream metadata without full decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct AvProbe {
    /// Optional video width, height, and duration in seconds.
    pub video: Option<(u32, u32, f64)>,
    /// Optional audio duration in seconds.
    pub audio: Option<f64>,
}

/// Probe detail needed for run-report summaries but intentionally kept out of the public
/// [`AvProbe`] contract.
pub(crate) struct AvProbeDetails {
    pub probe: AvProbe,
    pub video_pixel_format: Option<String>,
    pub audio_sample_rate_hz: Option<u32>,
    pub audio_channels: Option<u16>,
    pub audio_sample_format: Option<String>,
}

/// Exact container clock and presentation timestamps for a native video source.
/// Display metadata describes the normalized RGBA frames returned by LibavVideoSource.
pub struct VideoPresentationProbe {
    pub display: SourceMeta,
    pub stream: u32,
    pub time_base: (i32, i32),
    pub duration_ticks: i64,
    pub presentation: Vec<(i64, i64)>,
}

backend_type!(LibavVideoSource, decode);
impl LibavVideoSource {
    pub fn open(path: &Path) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::decode::LibavVideoSource::open(path).map(Into::into),
            Version::V8 => backend::v8::decode::LibavVideoSource::open(path).map(Into::into),
            Version::V9 => backend::v9::decode::LibavVideoSource::open(path).map(Into::into),
        }
    }
    pub fn open_with_threads(path: &Path, threads: usize) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::decode::LibavVideoSource::open_with_threads(path, threads)
                .map(Into::into),
            Version::V8 => backend::v8::decode::LibavVideoSource::open_with_threads(path, threads)
                .map(Into::into),
            Version::V9 => backend::v9::decode::LibavVideoSource::open_with_threads(path, threads)
                .map(Into::into),
        }
    }
    pub fn open_with_transport(path: &Path, transport: VideoFrameTransport) -> Result<Self> {
        match backend::version()? {
            Version::V7 => {
                backend::v7::decode::LibavVideoSource::open_with_transport(path, transport)
                    .map(Into::into)
            }
            Version::V8 => {
                backend::v8::decode::LibavVideoSource::open_with_transport(path, transport)
                    .map(Into::into)
            }
            Version::V9 => {
                backend::v9::decode::LibavVideoSource::open_with_transport(path, transport)
                    .map(Into::into)
            }
        }
    }
}
backend_type!(LazyYuv, decode);
impl LazyYuv {
    pub fn rgba(&self) -> Result<Arc<RgbaFrame>> {
        match &self.0 {
            Inner::V7(inner) => inner.rgba(),
            Inner::V8(inner) => inner.rgba(),
            Inner::V9(inner) => inner.rgba(),
        }
    }
}
backend_type!(DecodedGpuFrame, decode);
impl DecodedGpuFrame {
    pub fn backend(&self) -> SharedFrameBackend {
        match &self.0 {
            Inner::V7(inner) => inner.backend(),
            Inner::V8(inner) => inner.backend(),
            Inner::V9(inner) => inner.backend(),
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        match &self.0 {
            Inner::V7(inner) => inner.dimensions(),
            Inner::V8(inner) => inner.dimensions(),
            Inner::V9(inner) => inner.dimensions(),
        }
    }
    pub fn coded_dimensions(&self) -> (u32, u32) {
        match &self.0 {
            Inner::V7(inner) => inner.coded_dimensions(),
            Inner::V8(inner) => inner.coded_dimensions(),
            Inner::V9(inner) => inner.coded_dimensions(),
        }
    }
    pub fn supports_direct_import(&self) -> bool {
        match &self.0 {
            Inner::V7(inner) => inner.supports_direct_import(),
            Inner::V8(inner) => inner.supports_direct_import(),
            Inner::V9(inner) => inner.supports_direct_import(),
        }
    }
    pub fn rotation_deg(&self) -> u32 {
        match &self.0 {
            Inner::V7(inner) => inner.rotation_deg(),
            Inner::V8(inner) => inner.rotation_deg(),
            Inner::V9(inner) => inner.rotation_deg(),
        }
    }
    pub fn is_full_range(&self) -> bool {
        match &self.0 {
            Inner::V7(inner) => inner.is_full_range(),
            Inner::V8(inner) => inner.is_full_range(),
            Inner::V9(inner) => inner.is_full_range(),
        }
    }
    pub fn matrix(&self) -> YuvMatrix {
        match &self.0 {
            Inner::V7(inner) => inner.matrix(),
            Inner::V8(inner) => inner.matrix(),
            Inner::V9(inner) => inner.matrix(),
        }
    }
    pub fn handle(&self) -> SharedVideoFrameHandle {
        match &self.0 {
            Inner::V7(inner) => inner.handle(),
            Inner::V8(inner) => inner.handle(),
            Inner::V9(inner) => inner.handle(),
        }
    }
    pub fn rgba(&self) -> Result<Arc<RgbaFrame>> {
        match &self.0 {
            Inner::V7(inner) => inner.rgba(),
            Inner::V8(inner) => inner.rgba(),
            Inner::V9(inner) => inner.rgba(),
        }
    }
}
pub fn decode_rgba_frames(path: &Path, max: Option<usize>) -> Result<Vec<RgbaFrame>> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::decode_rgba_frames(path, max),
        Version::V8 => backend::v8::decode::decode_rgba_frames(path, max),
        Version::V9 => backend::v9::decode::decode_rgba_frames(path, max),
    }
}
pub fn probe_dimensions(path: &Path) -> Result<(u32, u32)> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::probe_dimensions(path),
        Version::V8 => backend::v8::decode::probe_dimensions(path),
        Version::V9 => backend::v9::decode::probe_dimensions(path),
    }
}
pub fn probe_av(path: &Path) -> Result<AvProbe> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::probe_av(path),
        Version::V8 => backend::v8::decode::probe_av(path),
        Version::V9 => backend::v9::decode::probe_av(path),
    }
}
#[cfg(any(
    test,
    feature = "tool-enhance",
    feature = "tool-separate",
    all(feature = "tool-transcribe", not(target_os = "windows"))
))]
pub(crate) fn probe_av_details(path: &Path) -> Result<AvProbeDetails> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::probe_av_details(path),
        Version::V8 => backend::v8::decode::probe_av_details(path),
        Version::V9 => backend::v9::decode::probe_av_details(path),
    }
}
pub fn probe_video_presentation(path: &Path) -> Result<VideoPresentationProbe> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::probe_video_presentation(path),
        Version::V8 => backend::v8::decode::probe_video_presentation(path),
        Version::V9 => backend::v9::decode::probe_video_presentation(path),
    }
}
impl VideoSource for LibavVideoSource {
    fn meta(&self) -> &SourceMeta {
        match &self.0 {
            Inner::V7(inner) => inner.meta(),
            Inner::V8(inner) => inner.meta(),
            Inner::V9(inner) => inner.meta(),
        }
    }
    fn frame_at(&mut self, t: f64) -> Result<Arc<RgbaFrame>> {
        match &mut self.0 {
            Inner::V7(inner) => inner.frame_at(t),
            Inner::V8(inner) => inner.frame_at(t),
            Inner::V9(inner) => inner.frame_at(t),
        }
    }
    fn frame_at_lazy(&mut self, t: f64) -> Result<Arc<SourceFrame>> {
        match &mut self.0 {
            Inner::V7(inner) => inner.frame_at_lazy(t),
            Inner::V8(inner) => inner.frame_at_lazy(t),
            Inner::V9(inner) => inner.frame_at_lazy(t),
        }
    }
}

#[cfg(feature = "tool-segment")]
pub(crate) fn validate_qtrle(path: &Path) -> Result<()> {
    match backend::version()? {
        Version::V7 => backend::v7::decode::validate_qtrle(path),
        Version::V8 => backend::v8::decode::validate_qtrle(path),
        Version::V9 => backend::v9::decode::validate_qtrle(path),
    }
}
