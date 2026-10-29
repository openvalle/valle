use serde::{Deserialize, Serialize};

use valle_draw::program::recording::FillRule;

use crate::artifact::{ColorValue, NumberValue, PathValue, PointValue};

/// Unique public Motion Glass capability.
pub const MOTION_GLASS_CAPABILITY: &str = "motion-glass";

pub const DEFAULT_PRESENCE: f64 = 1.0;
pub const DEFAULT_MERGE_DISTANCE: f64 = 24.0;
pub const DEFAULT_LIGHT_ELEVATION: f64 = 0.55;
pub const DEFAULT_LIGHT_INTENSITY: f64 = 0.65;
pub const MERGE_MIN_DISTANCE: f64 = 1.0;
pub const MERGE_MAX_DISTANCE: f64 = 128.0;
/// An absent bevel is this fraction of the shorter side of the box, of the smallest member's
/// box in a field, and at most [`DEFAULT_BEVEL_MAX`] local pixels.
pub const DEFAULT_BEVEL_OF_SHORTER_SIDE: f64 = 0.25;
pub const DEFAULT_BEVEL_MAX: f64 = 20.0;
/// An absent refraction is this multiple of the bevel, and at most
/// [`DEFAULT_REFRACTION_OF_SHORTER_SIDE`] of the shorter side, so the rim's view folds over
/// without reaching past the middle.
pub const DEFAULT_REFRACTION_OF_BEVEL: f64 = 3.0;
pub const DEFAULT_REFRACTION_OF_SHORTER_SIDE: f64 = 0.5;

/// What the glass is made of. Lengths are local pixels of the Glass, or of a field's parent, and
/// scale with its transforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassMaterialBinding {
    /// Width of the curved rim; absent, [`DEFAULT_BEVEL_OF_SHORTER_SIDE`] of the shorter side, at
    /// most [`DEFAULT_BEVEL_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<NumberValue>,
    /// The backdrop's shift at the edge, falling to none `bevel` inward; absent,
    /// [`DEFAULT_REFRACTION_OF_BEVEL`] times the bevel, at most
    /// [`DEFAULT_REFRACTION_OF_SHORTER_SIDE`] of the shorter side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refraction: Option<NumberValue>,
    /// How much further blue and less far red shift than green, as a fraction of `refraction`.
    pub dispersion: NumberValue,
    /// Backdrop blur, a standard deviation as in CSS `blur()`.
    pub frost: NumberValue,
    /// Backdrop saturation, as in CSS `saturate()`: 1 leaves it, 0 is gray.
    pub saturation: NumberValue,
    /// The glass's own color, mixed in under the light by its alpha.
    pub tint: ColorValue,
}

impl GlassMaterialBinding {
    pub fn defaults() -> Self {
        Self {
            bevel: None,
            refraction: None,
            dispersion: NumberValue::Static { value: 0.0 },
            frost: NumberValue::Static { value: 0.0 },
            saturation: NumberValue::Static { value: 1.0 },
            tint: ColorValue::Static {
                value: valle_draw::program::AuthorColor::from_srgb8(valle_draw::Rgba::TRANSPARENT),
            },
        }
    }
}

/// The light the glass reflects. It stays fixed on the screen, so a turning glass does not turn
/// its highlights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassLightBinding {
    /// Screen direction the light comes from, y down.
    pub direction: PointValue,
    /// Elevation of the environment highlight, 0–1. Higher values lift its reflection
    /// along the curved bevel and spread the edge glint further around the rim.
    pub elevation: NumberValue,
    /// Strength of the glint, its glow, the rim's shading and the dark edge; 0 draws none of
    /// them.
    pub intensity: NumberValue,
}

impl GlassLightBinding {
    pub fn defaults() -> Self {
        Self {
            direction: PointValue::Static {
                value: valle_draw::Point::new(-0.35, -0.94),
            },
            elevation: NumberValue::Static {
                value: DEFAULT_LIGHT_ELEVATION,
            },
            intensity: NumberValue::Static {
                value: DEFAULT_LIGHT_INTENSITY,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassMergeIntent {
    pub distance: f64,
}

impl GlassMergeIntent {
    pub fn defaults() -> Self {
        Self {
            distance: DEFAULT_MERGE_DISTANCE,
        }
    }
}

/// A box drawn in glass. Its shape is `path` in its local pixels, or else its border box rounded
/// by `borderRadius`. A field member takes its field's material and light, so it leaves them
/// absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathValue>,
    pub fill_rule: FillRule,
    pub material: Option<GlassMaterialBinding>,
    pub light: Option<GlassLightBinding>,
    /// 0–1: scales every effect, the frost included, so the glass forms rather than fades in.
    pub presence: NumberValue,
}

/// Glass members merged into one surface. The node is not a layout box: its children lay out
/// as its parent's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassFieldNode {
    pub material: GlassMaterialBinding,
    pub light: GlassLightBinding,
    pub merge: GlassMergeIntent,
}
