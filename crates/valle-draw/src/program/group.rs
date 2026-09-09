use serde::{Deserialize, Serialize};

use crate::{
    Rect,
    requirements::{Insets, RuntimeShaderKey, SamplingMode},
};

use super::{LinearColor, NodeId, PathId, Transform2d};

/// Truncated support used by the canonical DrawProgram Gaussian filter contract.
pub const FILTER_GAUSSIAN_SUPPORT_SIGMAS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundRect {
    pub rect: Rect,
    /// Top-left, top-right, bottom-right, bottom-left elliptical radii.
    pub radii: [[f64; 2]; 4],
}

impl RoundRect {
    pub fn circular(rect: Rect, radii: [f64; 4]) -> Self {
        Self {
            rect,
            radii: radii.map(|radius| [radius, radius]),
        }
    }

    pub const fn sharp(rect: Rect) -> Self {
        Self {
            rect,
            radii: [[0.0, 0.0]; 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Clip {
    Rect(Rect),
    RoundRect(RoundRect),
    Path { path: PathId, fill_rule: FillRule },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Filter {
    Blur {
        sigma_x: f32,
        sigma_y: f32,
    },
    ColorMatrix {
        matrix: Box<[f32; 20]>,
    },
    Brightness {
        amount: f32,
    },
    Contrast {
        amount: f32,
    },
    Grayscale {
        amount: f32,
    },
    HueRotate {
        degrees: f32,
    },
    Invert {
        amount: f32,
    },
    Opacity {
        amount: f32,
    },
    Saturate {
        amount: f32,
    },
    Sepia {
        amount: f32,
    },
    DropShadow {
        offset: [f32; 2],
        sigma_x: f32,
        sigma_y: f32,
        color: LinearColor,
    },
    NoiseDisplacement {
        frequency: [f32; 2],
        octaves: u8,
        seed: u32,
        scale: f32,
        turbulence: bool,
    },
    VelocityBlur {
        velocity: [f32; 2],
        shutter_angle_degrees: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    Alpha,
    Luminance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub source: NodeId,
    pub mode: MaskMode,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    content = "key",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum BackdropScope {
    Current,
    LayerEntry(String),
    ScopeEntry(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackdropRead {
    pub scope: BackdropScope,
    pub bounds: Rect,
    pub footprint: Insets,
    pub sampling: SamplingMode,
    /// Empty means an identity backdrop copy. A non-empty chain is applied in author order.
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ShaderUniformValue {
    Float(f32),
    Float2([f32; 2]),
    Color(LinearColor),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderUniformBinding {
    pub name: String,
    pub value: ShaderUniformValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderTextureBinding {
    pub name: String,
    pub texture: crate::requirements::ExternalTexture,
    pub sampling: SamplingMode,
}

/// One admitted shader applied to the complete child contribution of a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderLayer {
    pub shader: RuntimeShaderKey,
    pub bounds: Rect,
    pub uniforms: Vec<ShaderUniformBinding>,
    pub textures: Vec<ShaderTextureBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Group {
    pub children: Vec<NodeId>,
    /// Motion Glass material painted from the Current destination before ordered foreground
    /// children. Foreground remains ordinary DrawProgram content instead of a synthetic payload.
    pub glass: Option<Box<super::MotionGlassProgram>>,
    /// Marks the nested ordinary subtree as the real foreground of one Glass surface. The
    /// material owner is a distinct ancestor group; validators reject orphaned, duplicated, or
    /// mismatched markers instead of letting executors guess painter order.
    pub glass_foreground: Option<Box<super::MotionGlassForegroundProgram>>,
    pub transform: Transform2d,
    pub clip: Option<Clip>,
    pub filters: Vec<Filter>,
    pub mask: Option<Mask>,
    pub opacity: f32,
    pub internal_blend: BlendMode,
    pub isolated: bool,
    pub backdrop: Option<BackdropRead>,
    pub shader: Option<ShaderLayer>,
    /// Explicit author layer bounds. This is validation/cost metadata, never executor policy.
    pub layer_bounds: Option<Rect>,
}

impl Group {
    pub fn plain(children: Vec<NodeId>) -> Self {
        Self {
            children,
            glass: None,
            glass_foreground: None,
            transform: Transform2d::IDENTITY,
            clip: None,
            filters: Vec::new(),
            mask: None,
            opacity: 1.0,
            internal_blend: BlendMode::Normal,
            isolated: false,
            backdrop: None,
            shader: None,
            layer_bounds: None,
        }
    }

    /// Whether this container only transforms its children, without a group pixel operation.
    /// It can share a raster target only when its entire subtree is destination-independent;
    /// in that case isolation and layer-bound metadata do not change the result.
    pub fn is_transform_only(&self) -> bool {
        self.opacity == 1.0 && self.is_raster_group()
    }

    /// Local painter-order group; opacity requires isolation of its children before restore.
    /// Destination reads, clips and other pixel operations retain explicit scheduled passes.
    pub fn is_raster_group(&self) -> bool {
        self.glass.is_none()
            && self.glass_foreground.is_none()
            && self.clip.is_none()
            && self.filters.is_empty()
            && self.mask.is_none()
            && self.internal_blend == BlendMode::Normal
            && self.backdrop.is_none()
            && self.shader.is_none()
    }

    /// Returns true only when replacing this group with its ordered children is pixel-equivalent.
    pub fn is_plain(&self) -> bool {
        self.is_transform_only()
            && self.transform == Transform2d::IDENTITY
            && !self.isolated
            && self.layer_bounds.is_none()
    }
}
