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
/// An absent bevel is this fraction of the shorter side of the box: of the smallest member's
/// box in a field.
pub const DEFAULT_BEVEL_OF_SHORTER_SIDE: f64 = 0.4;
/// An absent refraction is this fraction of the bevel.
pub const DEFAULT_REFRACTION_OF_BEVEL: f64 = 0.3;

/// What the glass is made of. Lengths are local pixels of the Glass, or of a field's parent, and
/// scale with its transforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassMaterialBinding {
    /// Width of the curved rim; absent, [`DEFAULT_BEVEL_OF_SHORTER_SIDE`] of the shorter side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<NumberValue>,
    /// How far the backdrop shifts across a straight rim; absent, [`DEFAULT_REFRACTION_OF_BEVEL`]
    /// of the bevel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refraction: Option<NumberValue>,
    /// How much further blue and less far red shift than green, as a fraction of `refraction`.
    pub dispersion: NumberValue,
    /// Backdrop blur, a standard deviation as in CSS `blur()`.
    pub frost: NumberValue,
    /// The glass's own color, mixed in under the highlights by its alpha.
    pub tint: ColorValue,
}

impl GlassMaterialBinding {
    pub fn defaults() -> Self {
        Self {
            bevel: None,
            refraction: None,
            dispersion: NumberValue::Static { value: 0.0 },
            frost: NumberValue::Static { value: 0.0 },
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
    /// 0 grazes the glass, 1 shines straight at it.
    pub elevation: NumberValue,
    /// Highlight strength; 0 draws none.
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
