//! Synchronous, object-safe video and audio source contracts. Mutable access advances decoder
//! state; fallible methods report I/O errors. Shared eager or lazy frames support repeated sampling
//! and parallel rendering without duplicate decoding.

use std::sync::Arc;

use crate::frame::{AudioBuffer, RgbaFrame};

/// Source metadata in display coordinates after rotation and sample-aspect normalization.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceMeta {
    /// Display width in pixels.
    pub width: u32,
    /// Display height in pixels.
    pub height: u32,
    /// Container rotation metadata in quarter-turn degrees.
    pub rotation_deg: i32,
    /// Nominal frame rate, averaged for variable-rate sources.
    pub nominal_fps: f64,
    /// Stream start timestamp in seconds for source-local time and seek conversion.
    pub start_time_s: f64,
    /// Optional source duration in seconds for looping and clamping.
    pub duration_s: Option<f64>,
}

/// Ready RGBA, lazy YUV, or a decoder-owned GPU surface. Consumers convert CPU pixels once or
/// import the GPU surface, sharing results when an input frame is reused.
pub enum SourceFrame {
    /// Ready RGBA supplied directly by a source or test adapter.
    Rgba(Arc<RgbaFrame>),
    /// Decoded YUV with explicit conversion parameters.
    #[cfg(feature = "libav")]
    LazyYuv(crate::codec::decode::LazyYuv),
    /// Decoder-owned platform surface. Native GPU renderers may import its borrowed handle
    /// directly; CPU consumers use the memoized transfer/conversion fallback in [`Self::rgba`].
    #[cfg(feature = "libav")]
    DecodedGpu(crate::codec::decode::DecodedGpuFrame),
}

impl SourceFrame {
    /// Return shared RGBA, performing and caching lazy conversion on first access.
    pub fn rgba(&self) -> anyhow::Result<Arc<RgbaFrame>> {
        match self {
            SourceFrame::Rgba(f) => Ok(f.clone()),
            #[cfg(feature = "libav")]
            SourceFrame::LazyYuv(l) => l.rgba(),
            #[cfg(feature = "libav")]
            SourceFrame::DecodedGpu(f) => f.rgba(),
        }
    }

    /// Return a directly importable decoded GPU surface. Non-square SAR frames return `None` so
    /// callers take the memoized RGBA path that normalizes them to square pixels.
    #[cfg(feature = "libav")]
    pub fn decoded_gpu(&self) -> Option<&crate::codec::decode::DecodedGpuFrame> {
        match self {
            SourceFrame::DecodedGpu(frame) if frame.supports_direct_import() => Some(frame),
            _ => None,
        }
    }
}

/// Video source returning the latest frame at or before source-local time. Sequential requests
/// advance decoding; backward requests seek. Send permits ownership transfer to a decode thread.
pub trait VideoSource: Send {
    /// Display dimensions, rotation, nominal frame rate, and stream start metadata.
    fn meta(&self) -> &SourceMeta;
    /// Return the latest frame at or before source-local time t.
    fn frame_at(&mut self, t: f64) -> anyhow::Result<Arc<RgbaFrame>>;
    /// Advance decoding while deferring conversion or GPU import to the consumer. The default
    /// implementation wraps eager RGBA.
    fn frame_at_lazy(&mut self, t: f64) -> anyhow::Result<Arc<SourceFrame>> {
        Ok(Arc::new(SourceFrame::Rgba(self.frame_at(t)?)))
    }
}

/// Audio source for the half-open interval [t, t+dt).
pub trait AudioSource: Send {
    /// Read a duration of audio beginning at source-local time t.
    fn samples(&mut self, t: f64, dt: f64) -> anyhow::Result<AudioBuffer>;

    /// Read an exact half-open interval on the source output sample grid. Use this after
    /// compilation has resolved sample identity; do not round-trip indices through seconds.
    fn samples_by_index(
        &mut self,
        start_sample: i64,
        end_sample: i64,
    ) -> anyhow::Result<AudioBuffer>;
}
