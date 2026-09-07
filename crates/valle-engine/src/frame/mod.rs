//! Engine-owned mechanical frame geometry shared by preparation and hosts.

mod resolved;
pub(crate) use resolved::anchor_fraction;
pub use resolved::{
    Anchor, ClipAnimationApplication, CompositeBlendMode, LayerBackdrop, LayerInset, MaskShape,
    RasterFit, ResolvedCamera, ResolvedCameraTarget, ResolvedClipAnimation, ResolvedMask,
    ResolvedTransform, SourceCrop,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::resource::{
    AuthorSrgbStraight, ContentDigest, MediaDescriptor, OutputSpec, SemanticAssetKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderQuality {
    Preview,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RenderSpecWire")]
pub struct RenderSpec {
    width: u32,
    height: u32,
    quality: RenderQuality,
    output: OutputSpec,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderSpecWire {
    width: u32,
    height: u32,
    quality: RenderQuality,
    output: OutputSpec,
}

impl TryFrom<RenderSpecWire> for RenderSpec {
    type Error = RenderSpecError;

    fn try_from(value: RenderSpecWire) -> Result<Self, Self::Error> {
        Self::new(value.width, value.height, value.quality, value.output)
    }
}

impl RenderSpec {
    pub fn new(
        width: u32,
        height: u32,
        quality: RenderQuality,
        output: OutputSpec,
    ) -> Result<Self, RenderSpecError> {
        if width == 0 || height == 0 {
            return Err(RenderSpecError::EmptyOutput);
        }
        Ok(Self {
            width,
            height,
            quality,
            output,
        })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn quality(self) -> RenderQuality {
        self.quality
    }

    pub const fn output(self) -> OutputSpec {
        self.output
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RenderSpecError {
    #[error("render output width and height must be positive")]
    EmptyOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasMapping {
    pub scale: f64,
    pub offset_x_px: f64,
    pub offset_y_px: f64,
    pub viewport_width_px: f64,
    pub viewport_height_px: f64,
}

impl CanvasMapping {
    pub(crate) fn contain(canvas: [u32; 2], output: RenderSpec) -> Self {
        let scale = (f64::from(output.width) / f64::from(canvas[0]))
            .min(f64::from(output.height) / f64::from(canvas[1]));
        let viewport_width_px = f64::from(canvas[0]) * scale;
        let viewport_height_px = f64::from(canvas[1]) * scale;
        Self {
            scale,
            offset_x_px: (f64::from(output.width) - viewport_width_px) / 2.0,
            offset_y_px: (f64::from(output.height) - viewport_height_px) / 2.0,
            viewport_width_px,
            viewport_height_px,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundBand {
    pub author_srgb_straight: AuthorSrgbStraight,
}

impl Default for BackgroundBand {
    fn default() -> Self {
        Self {
            author_srgb_straight: AuthorSrgbStraight([0, 0, 0, 255]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VisualSource {
    Asset {
        id: String,
        asset_kind: SemanticAssetKind,
        digest: ContentDigest,
        descriptor: MediaDescriptor,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluatedVisualClip {
    pub clip_id: String,
    pub track_id: String,
    pub source: VisualSource,
    pub transform: ResolvedTransform,
    pub source_crop: SourceCrop,
    pub source_time_s: f64,
    pub source_boundary: SourceBoundary,
    pub mask: Option<ResolvedMask>,
    pub blend: CompositeBlendMode,
    pub resolved_animation: ResolvedClipAnimation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceBoundary {
    Static,
    ClampToDescriptor,
    ExtendedSemantic,
}
