use serde::{Deserialize, Serialize};

use crate::artifact::{ColorValue, NumberValue, PathValue, PointValue};

use super::ids::{GlassFieldId, GlassSurfaceId};

/// Unique public Motion Glass capability.
pub const MOTION_GLASS_CAPABILITY: &str = "motion-glass";

/// Frozen schema identity mixed into packed ABI and kernel digest.
pub const MOTION_GLASS_SCHEMA_ID: &str = valle_draw::program::MOTION_GLASS_SCHEMA_ID;

pub const DEFAULT_CLARITY: f64 = 0.82;
pub const DEFAULT_DEPTH: f64 = 0.52;
pub const DEFAULT_PRESENCE: f64 = 1.0;
pub const DEFAULT_MERGE_DISTANCE: f64 = 24.0;
pub const DEFAULT_LIGHT_ELEVATION: f64 = 0.55;
pub const DEFAULT_LIGHT_INTENSITY: f64 = 0.65;
pub const MERGE_MIN_DISTANCE: f64 = 1.0;
pub const MERGE_MAX_DISTANCE: f64 = 128.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassLightSpace {
    Screen,
    World,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind"))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum GlassShapeBinding {
    Capsule,
    Circle,
    ContinuousRect {
        radius: NumberValue,
    },
    Path {
        path: PathValue,
        reveal_origin: Option<PointValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassMaterialBinding {
    pub clarity: NumberValue,
    pub depth: NumberValue,
    pub tint: ColorValue,
}

impl GlassMaterialBinding {
    pub fn defaults() -> Self {
        Self {
            clarity: NumberValue::Static {
                value: DEFAULT_CLARITY,
            },
            depth: NumberValue::Static {
                value: DEFAULT_DEPTH,
            },
            tint: ColorValue::Static {
                value: valle_draw::program::AuthorColor::from_srgb8(valle_draw::Rgba::TRANSPARENT),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassLightBinding {
    pub direction: PointValue,
    pub elevation: NumberValue,
    pub intensity: NumberValue,
    pub space: GlassLightSpace,
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
            space: GlassLightSpace::Screen,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassEnvironmentBinding {
    pub light: GlassLightBinding,
}

impl GlassEnvironmentBinding {
    pub fn defaults() -> Self {
        Self {
            light: GlassLightBinding::defaults(),
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

/// Normalized independent or field-member Glass node payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassNode {
    pub surface_id: GlassSurfaceId,
    pub field_id: Option<GlassFieldId>,
    pub shape: GlassShapeBinding,
    pub material: Option<GlassMaterialBinding>,
    /// Independent owners carry an expanded environment. Field members inherit from their
    /// nearest GlassField and therefore must leave this absent.
    pub environment: Option<GlassEnvironmentBinding>,
    pub presence: NumberValue,
}

/// Normalized field payload. The node itself is not a layout box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassFieldNode {
    pub field_id: GlassFieldId,
    pub material: GlassMaterialBinding,
    pub merge: GlassMergeIntent,
    pub environment: GlassEnvironmentBinding,
}
