use serde::{Deserialize, Serialize};

use crate::{
    Rect,
    requirements::{Insets, RuntimeShaderKey, SamplingMode},
};

use super::{AuthorColor, LinearColor, NodeId, PathId, Transform2d};

/// Truncated support used by the canonical DrawProgram Gaussian filter contract.
pub const FILTER_GAUSSIAN_SUPPORT_SIGMAS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    #[default]
    Normal,
    /// Premultiplied addition in the linear working space, with clamped coverage.
    Plus,
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

/// RGB domain for creative blend functions. Both choices use sRGB primaries;
/// `Linear` omits the transfer curve. Normal and Plus always use working-linear math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BlendSpace {
    #[default]
    Srgb,
    Linear,
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
    Glow {
        color: AuthorColor,
        radius: f32,
        intensity: f32,
    },
    Bloom {
        threshold: f32,
        knee: f32,
        intensity: f32,
        radius: f32,
    },
    RadialBlur {
        center: [f32; 2],
        amount: f32,
    },
    FilmGrain {
        seed: u32,
        amount: f32,
        size: f32,
    },
    LensDistortion {
        k1: f32,
        k2: f32,
    },
    ChromaticAberration {
        /// Red and blue are displaced by half this vector in opposite directions.
        offset: [f32; 2],
    },
    NoiseDisplacement {
        frequency: [f32; 2],
        octaves: u8,
        seed: u32,
        scale: f32,
        turbulence: bool,
    },
    /// The average of copies spread evenly along the travel of the shutter, `velocity` scaled by
    /// the open fraction of the frame and centred on the input; see
    /// [`Filter::velocity_blur_samples`] for how many.
    VelocityBlur {
        velocity: [f32; 2],
        shutter_angle_degrees: f32,
    },
}

impl Filter {
    /// The copies a velocity blur averages for its velocity in device pixels: one for every 2px
    /// of travel along the larger axis, so a long trail stays smooth, but at least 9 and at most
    /// 32, which bounds the cost; past 62px the copies spread further apart.
    pub fn velocity_blur_samples(velocity: [f32; 2], shutter_angle_degrees: f32) -> usize {
        let shutter = (shutter_angle_degrees / 360.0).clamp(0.0, 1.0);
        let travel = velocity[0].abs().max(velocity[1].abs()) * shutter;
        ((travel / 2.0).ceil() as usize)
            .saturating_add(1)
            .clamp(9, 32)
    }

    /// Whether the filter leaves every pixel unchanged, so canonical programs omit it. Only
    /// sampling and generated-light filters at zero strength qualify; colour filters may clamp
    /// extended-range working values even at their neutral setting.
    pub fn is_identity(&self) -> bool {
        match self {
            Self::Blur { sigma_x, sigma_y } => *sigma_x == 0.0 && *sigma_y == 0.0,
            Self::Glow {
                color,
                radius,
                intensity,
            } => *radius == 0.0 || *intensity == 0.0 || color.alpha == 0.0,
            Self::Bloom {
                radius, intensity, ..
            } => *radius == 0.0 || *intensity == 0.0,
            Self::RadialBlur { amount, .. } => *amount == 0.0,
            Self::FilmGrain { amount, .. } => *amount == 0.0,
            Self::LensDistortion { k1, k2 } => *k1 == 0.0 && *k2 == 0.0,
            Self::ChromaticAberration { offset } => offset[0] == 0.0 && offset[1] == 0.0,
            Self::NoiseDisplacement { scale, .. } => *scale == 0.0,
            Self::VelocityBlur {
                velocity,
                shutter_angle_degrees,
            } => (velocity[0] == 0.0 && velocity[1] == 0.0) || *shutter_angle_degrees == 0.0,
            Self::ColorMatrix { .. }
            | Self::Brightness { .. }
            | Self::Contrast { .. }
            | Self::Grayscale { .. }
            | Self::HueRotate { .. }
            | Self::Invert { .. }
            | Self::Opacity { .. }
            | Self::Saturate { .. }
            | Self::Sepia { .. }
            | Self::DropShadow { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    Alpha,
    Luminance,
    AlphaInverted,
    LuminanceInverted,
}

impl MaskMode {
    pub fn is_inverted(self) -> bool {
        matches!(self, Self::AlphaInverted | Self::LuminanceInverted)
    }

    pub fn is_luminance(self) -> bool {
        matches!(self, Self::Luminance | Self::LuminanceInverted)
    }

    pub fn inverted(self) -> Self {
        match self {
            Self::Alpha => Self::AlphaInverted,
            Self::Luminance => Self::LuminanceInverted,
            Self::AlphaInverted => Self::Alpha,
            Self::LuminanceInverted => Self::Luminance,
        }
    }
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
    Float3([f32; 3]),
    Float4([f32; 4]),
    Float2x2([f32; 4]),
    Float3x3([f32; 9]),
    Float4x4([f32; 16]),

    /// Linear sRGB, straight alpha; RGB remains meaningful at zero coverage.
    Color([f32; 4]),
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
    pub wrap: super::SpreadMode,
    pub name: String,
    /// What the input samples; an unbound input reads transparent pixels.
    pub source: Option<ShaderTextureSource>,
    pub sampling: SamplingMode,
}

impl ShaderTextureBinding {
    /// The external texture the input samples, if it samples one.
    pub fn texture(&self) -> Option<&crate::requirements::ExternalTexture> {
        match &self.source {
            Some(ShaderTextureSource::External { texture }) => Some(texture),
            Some(ShaderTextureSource::Backdrop { .. }) | None => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ShaderTextureSource {
    /// A texture the host supplies before execution.
    External {
        texture: crate::requirements::ExternalTexture,
    },
    /// What a group's shader layer draws over: the destination at the entry of the group
    /// `through` levels above it, 0 being its own, sampled in place as its content is. The read
    /// covers the shader's output bounds and [`SHADER_BACKDROP_FOOTPRINT`]; each group it reaches
    /// across holds the next among its children and draws no transition.
    Backdrop { through: u32 },
}

/// Maximum local shader output area, including static padding.
pub const MAX_SHADER_LAYER_PIXELS: u64 = 1920 * 1080;

/// How far a shader layer's backdrop read reaches beyond its output bounds, in local pixels: the
/// device filters a sample at their edge with the pixel past it.
pub const SHADER_BACKDROP_FOOTPRINT: crate::requirements::Insets =
    crate::requirements::Insets::new(1.0, 1.0, 1.0, 1.0);

/// One admitted shader applied to the complete child contribution of a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderLayer {
    /// Static local pixel outsets, in left/top/right/bottom order.
    pub padding: [u32; 4],
    pub shader: RuntimeShaderKey,
    pub bounds: Rect,
    pub uniforms: Vec<ShaderUniformBinding>,
    pub textures: Vec<ShaderTextureBinding>,
}

impl ShaderLayer {
    pub fn output_bounds(&self) -> Rect {
        let [left, top, right, bottom] = self.padding.map(f64::from);
        Rect::new(
            self.bounds.x - left,
            self.bounds.y - top,
            self.bounds.width + left + right,
            self.bounds.height + top + bottom,
        )
    }
}

/// What a transition draws from its two inputs: a built-in kernel with its canonical parameters,
/// or an admitted shader that samples them as its `from` and `to` children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TransitionKernel {
    Builtin {
        kind: crate::transition::TransitionKind,
        params: crate::transition::TransitionValues,
    },
    /// The shader's own uniforms and inputs; executors add `progress` beside `resolution`.
    Shader {
        shader: RuntimeShaderKey,
        uniforms: Vec<ShaderUniformBinding>,
        textures: Vec<ShaderTextureBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionLayer {
    pub kernel: TransitionKernel,
    pub progress: f32,
    /// Origin-anchored local canvas shared by both independent child subtrees.
    pub bounds: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Group {
    pub children: Vec<NodeId>,
    pub transform: Transform2d,
    pub clip: Option<Clip>,
    pub filters: Vec<Filter>,
    pub mask: Option<Mask>,
    pub opacity: f32,
    pub internal_blend: BlendMode,
    pub blend_space: BlendSpace,
    pub isolated: bool,
    pub backdrop: Option<BackdropRead>,
    pub shader: Option<ShaderLayer>,
    pub transition: Option<TransitionLayer>,
    /// Explicit author layer bounds. This is validation/cost metadata, never executor policy.
    pub layer_bounds: Option<Rect>,
}

impl Group {
    pub fn plain(children: Vec<NodeId>) -> Self {
        Self {
            children,
            transform: Transform2d::IDENTITY,
            clip: None,
            filters: Vec::new(),
            mask: None,
            opacity: 1.0,
            internal_blend: BlendMode::Normal,
            blend_space: BlendSpace::Srgb,
            isolated: false,
            backdrop: None,
            shader: None,
            transition: None,
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
        self.clip.is_none() && self.is_raster_tree_group()
    }

    /// A destination-independent raster subtree. Clips are applied once to the flattened
    /// children, using a separate coverage layer before the group's opacity is restored.
    pub fn is_raster_tree_group(&self) -> bool {
        self.filters.is_empty()
            && self.mask.is_none()
            && self.internal_blend == BlendMode::Normal
            && self.backdrop.is_none()
            && self.shader.is_none()
            && self.transition.is_none()
    }

    /// Returns true only when replacing this group with its ordered children is pixel-equivalent.
    pub fn is_plain(&self) -> bool {
        self.is_transform_only()
            && self.transform == Transform2d::IDENTITY
            && !self.isolated
            && self.layer_bounds.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::Filter;

    #[test]
    fn a_velocity_blur_takes_a_copy_for_every_two_pixels_of_travel_within_bounds() {
        let samples = Filter::velocity_blur_samples;
        assert_eq!(samples([0.0, 0.0], 180.0), 9);
        assert_eq!(samples([16.0, 0.0], 360.0), 9);
        assert_eq!(samples([-17.0, 3.0], 360.0), 10);
        assert_eq!(samples([5.0, -40.0], 180.0), 11);
        assert_eq!(samples([62.0, 0.0], 360.0), 32);
        assert_eq!(samples([1.0e9, 0.0], 720.0), 32);
    }
}
