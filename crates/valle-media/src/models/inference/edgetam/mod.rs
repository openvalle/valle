//! Shared, stateful EdgeTAM video-object-segmentation adapter.
//!
//! This crate does not decode or encode media. Callers provide tightly packed straight-alpha
//! RGBA8 frames and receive tightly packed binary Gray8 masks plus soft Alpha8 mattes. The adapter
//! owns conversion to the model's
//! RGB/NCHW 1024-square canvas, prompt scaling, the bounded memory bank and pointer queue, mask
//! selection, resize-to-source, sigmoid/quantization, thresholding and strict frame/PTS ordering.

mod adapter;
mod artifact;
#[cfg(any(feature = "model-edgetam-onnx", test))]
mod contract;
#[cfg(feature = "model-edgetam-onnx")]
#[path = "onnx_impl.rs"]
mod onnx;

use anyhow::{Result, ensure};

pub use adapter::{EdgeTamAdapter, EdgeTamSession};
pub use artifact::EdgeTamConstants;
#[cfg(feature = "model-edgetam-onnx")]
pub use onnx::{OnnxBackend, OnnxEdgeTam, OnnxOptions};

pub const ADAPTER: &str = "edgetam-video-segmentation";
pub const CONTRACT_VERSION: u32 = 1;
pub const CANVAS_SIDE: usize = 1_024;
pub const LOW_SIDE: usize = 256;
pub const EMBEDDING_DIM: usize = 256;
pub const EMBEDDING_GRID: usize = 64;
pub const MEMORY_SLOTS: usize = 7;
pub const MEMORY_TOKENS_PER_FRAME: usize = 512;
pub const POINTER_TOKENS: usize = 64;
pub const KEY_VALUE_TOKENS: usize = MEMORY_SLOTS * MEMORY_TOKENS_PER_FRAME + POINTER_TOKENS;
pub const FULL_MEMORY_FROM_FRAME: u64 = 16;

pub(crate) const EMPTY_SLOT_BIAS: f32 = -30_000.0;
pub(crate) const NO_OBJECT_LOGIT: f32 = -1_024.0;
pub(crate) const STABILITY_DELTA: f32 = 0.05;
pub(crate) const STABILITY_THRESHOLD: f32 = 0.98;

/// A tightly packed, straight-alpha RGBA8 frame borrowed for one call.
///
/// RGB bytes are passed to EdgeTAM exactly as supplied. Alpha is intentionally ignored rather
/// than premultiplied or composited; callers that need compositing must do it before this boundary.
#[derive(Debug, Clone, Copy)]
pub struct Rgba8Frame<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> Rgba8Frame<'a> {
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self> {
        ensure!(
            width > 0 && height > 0,
            "RGBA8 frame dimensions must be positive"
        );
        let expected = usize::try_from(width)?
            .checked_mul(usize::try_from(height)?)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("RGBA8 frame dimensions overflow"))?;
        ensure!(
            pixels.len() == expected,
            "RGBA8 frame has {} bytes, {width}x{height} requires {expected}",
            pixels.len()
        );
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn pixels(self) -> &'a [u8] {
        self.pixels
    }
}

/// Opaque presentation timestamp ticks. All frames in one session must use the same time base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PresentationTimestamp(i64);

impl PresentationTimestamp {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PointLabel {
    Negative = 0,
    Positive = 1,
}

impl PointLabel {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PromptPoint {
    x: f32,
    y: f32,
    label: PointLabel,
}

impl PromptPoint {
    pub fn new(x: f32, y: f32, label: PointLabel) -> Result<Self> {
        ensure!(
            x.is_finite() && y.is_finite(),
            "prompt coordinates must be finite"
        );
        Ok(Self { x, y, label })
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }

    pub const fn label(self) -> PointLabel {
        self.label
    }
}

/// The release's fixed first-frame prompt protocol: exactly three labeled points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointPrompt {
    points: [PromptPoint; 3],
}

impl PointPrompt {
    pub fn new(points: [PromptPoint; 3]) -> Result<Self> {
        ensure!(
            points
                .iter()
                .any(|point| point.label == PointLabel::Positive),
            "EdgeTAM prompt requires at least one positive point"
        );
        Ok(Self { points })
    }

    pub const fn points(&self) -> &[PromptPoint; 3] {
        &self.points
    }
}

/// Owned, tightly packed binary Gray8 output. Every pixel is exactly 0 or 255.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray8Mask {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Owned, tightly packed soft Alpha8 output derived from the selected model logits.
///
/// This is deliberately separate from [`Gray8Mask`]: alpha preserves the model probability while
/// the binary mask is the thresholded cross-tool contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alpha8Matte {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Alpha8Matte {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

impl Gray8Mask {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AdapterOptions {
    /// Threshold applied after resizing selected logits to the source dimensions.
    pub mask_threshold: f32,
    /// Hard cap for one source/output frame. The default admits 8K UHD.
    pub max_source_pixels: usize,
}

impl Default for AdapterOptions {
    fn default() -> Self {
        Self {
            mask_threshold: 0.0,
            max_source_pixels: 33_554_432,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMetrics {
    pub frame_index: u64,
    pub preprocess_ms: f64,
    pub encode_ms: f64,
    pub memory_attention_ms: f64,
    pub decode_ms: f64,
    pub memory_encode_ms: f64,
    pub postprocess_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentationFrame {
    pub frame_index: u64,
    pub pts: PresentationTimestamp,
    pub mask: Gray8Mask,
    pub alpha: Alpha8Matte,
    pub metrics: FrameMetrics,
}

/// Current bounded state occupancy, exposed for diagnostics and memory assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateOccupancy {
    pub memory_frames: usize,
    pub pointer_frames: usize,
    pub max_memory_frames: usize,
    pub max_pointer_frames: usize,
}

/// Model-owned image encoder outputs.
#[derive(Debug)]
pub struct EncodedFrame {
    pub pixel_features: Vec<f32>,
    pub high_resolution_0: Vec<f32>,
    pub high_resolution_1: Vec<f32>,
}

/// Model-owned four-candidate decoder outputs.
#[derive(Debug)]
pub struct DecodedFrame {
    pub mask_logits: Vec<f32>,
    pub predicted_ious: Vec<f32>,
    pub object_pointers: Vec<f32>,
    pub object_score: f32,
}

/// Backend boundary for the five static EdgeTAM graphs.
///
/// The adapter validates every returned shape before adding it to session state.
pub trait EdgeTamBackend {
    fn encode(&mut self, image_nchw_0_255: &[f32]) -> Result<EncodedFrame>;

    fn memory_attention_warm(
        &mut self,
        current_tokens: &[f32],
        memory: &[f32],
        memory_positions: &[f32],
        attention_bias: &[f32],
    ) -> Result<Vec<f32>>;

    fn memory_attention_hot(&mut self, current_tokens: &[f32], memory: &[f32]) -> Result<Vec<f32>>;

    fn decode(
        &mut self,
        pixel_features: &[f32],
        high_resolution_0: &[f32],
        high_resolution_1: &[f32],
        coordinates: &[f32],
        labels: &[i32],
    ) -> Result<DecodedFrame>;

    fn encode_memory(&mut self, pixel_features: &[f32], memory_mask: &[f32]) -> Result<Vec<f32>>;
}
