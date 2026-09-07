//! Engine-owned frame-local geometry after Timeline admission.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    #[default]
    Center,
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

pub(crate) const fn anchor_fraction(anchor: Anchor) -> (f64, f64) {
    match anchor {
        Anchor::Center => (0.5, 0.5),
        Anchor::TopLeft => (0.0, 0.0),
        Anchor::Top => (0.5, 0.0),
        Anchor::TopRight => (1.0, 0.0),
        Anchor::Left => (0.0, 0.5),
        Anchor::Right => (1.0, 0.5),
        Anchor::BottomLeft => (0.0, 1.0),
        Anchor::Bottom => (0.5, 1.0),
        Anchor::BottomRight => (1.0, 1.0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RasterFit {
    Contain,
    Cover,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCrop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for SourceCrop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerInset {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LayerBackdrop {
    Color { color: valle_timeline::Color },
    Blur { radius: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompositeBlendMode {
    #[default]
    Normal,
    Screen,
    Lighten,
    ColorDodge,
    Multiply,
    Darken,
    ColorBurn,
    LinearBurn,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl CompositeBlendMode {
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskShape {
    Rect,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedCameraTarget {
    pub margin: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedCamera {
    pub center_x: f64,
    pub center_y: f64,
    pub zoom: f64,
    pub rotation: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ResolvedCameraTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipAnimationApplication {
    Raster,
    Semantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedClipAnimation {
    pub application: ClipAnimationApplication,
    pub scale: f32,
    pub opacity: f32,
    pub rotation_deg: f32,
    pub translate_x_px: f32,
    pub translate_y_px: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_inset: Option<[f32; 4]>,
    pub blur_sigma_px: f32,
}

impl ResolvedClipAnimation {
    pub const RASTER_IDENTITY: Self = Self::identity(ClipAnimationApplication::Raster);
    pub const SEMANTIC_IDENTITY: Self = Self::identity(ClipAnimationApplication::Semantic);

    const fn identity(application: ClipAnimationApplication) -> Self {
        Self {
            application,
            scale: 1.0,
            opacity: 1.0,
            rotation_deg: 0.0,
            translate_x_px: 0.0,
            translate_y_px: 0.0,
            clip_inset: None,
            blur_sigma_px: 0.0,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.scale == 1.0
            && self.opacity == 1.0
            && self.rotation_deg == 0.0
            && self.translate_x_px == 0.0
            && self.translate_y_px == 0.0
            && self.clip_inset.is_none()
            && self.blur_sigma_px == 0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedTransform {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub anchor: Anchor,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<RasterFit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<SourceCrop>,
    pub inset: LayerInset,
    pub flip_x: bool,
    pub flip_y: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<LayerBackdrop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedMask {
    #[serde(rename = "type")]
    pub kind: MaskShape,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub feather: f64,
    pub rotation: f64,
    pub invert: bool,
}
