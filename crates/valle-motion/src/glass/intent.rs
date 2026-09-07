use serde::{Deserialize, Serialize};

use crate::artifact::{ColorValue, NumberValue, PathValue, PointValue};

use super::ids::{GlassFieldId, GlassSurfaceId};

/// Unique public Motion Glass capability.
pub const MOTION_GLASS_CAPABILITY: &str = "motion-glass";

/// Frozen schema identity mixed into packed ABI and kernel digest.
pub const MOTION_GLASS_SCHEMA_ID: &str = valle_draw::program::MOTION_GLASS_SCHEMA_ID;

pub const DEFAULT_CLARITY: f64 = 0.82;
pub const DEFAULT_DEPTH: f64 = 0.52;
pub const DEFAULT_INTENSITY: f64 = 0.60;
pub const DEFAULT_SETTLE_SECONDS: f64 = 0.32;
pub const DEFAULT_PRESENCE: f64 = 1.0;
pub const DEFAULT_PROTECTION: f64 = 0.65;
pub const DEFAULT_MERGE_DISTANCE: f64 = 24.0;
pub const DEFAULT_LIGHT_ELEVATION: f64 = 0.55;
pub const DEFAULT_LIGHT_INTENSITY: f64 = 0.65;
pub const SETTLE_MIN_SECONDS: f64 = 0.08;
pub const SETTLE_MAX_SECONDS: f64 = 1.20;
pub const MERGE_MIN_DISTANCE: f64 = 1.0;
pub const MERGE_MAX_DISTANCE: f64 = 128.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassCharacter {
    Responsive,
    Fluid,
    Viscous,
    Elastic,
}

impl GlassCharacter {
    pub const DEFAULT: Self = Self::Fluid;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassForegroundTone {
    Auto,
    Light,
    Dark,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassLightSpace {
    Screen,
    World,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassUsageProfile {
    Interface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GlassUsageRole {
    Control,
    Navigation,
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassUsage {
    pub profile: GlassUsageProfile,
    pub role: GlassUsageRole,
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
                value: valle_draw::Rgba::TRANSPARENT,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassDriveBinding {
    pub translation: Option<PointValue>,
    pub pressure: Option<NumberValue>,
    pub twist: Option<NumberValue>,
}

impl GlassDriveBinding {
    pub fn none() -> Self {
        Self {
            translation: None,
            pressure: None,
            twist: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassSurfaceMotionBinding {
    pub character: Option<GlassCharacter>,
    pub intensity: NumberValue,
    pub settle: Option<f64>,
    pub drive: GlassDriveBinding,
}

impl GlassSurfaceMotionBinding {
    pub fn independent_defaults() -> Self {
        Self {
            character: Some(GlassCharacter::DEFAULT),
            intensity: NumberValue::Static {
                value: DEFAULT_INTENSITY,
            },
            settle: Some(DEFAULT_SETTLE_SECONDS),
            drive: GlassDriveBinding::none(),
        }
    }

    pub fn field_member_defaults() -> Self {
        Self {
            character: None,
            intensity: NumberValue::Static {
                value: DEFAULT_INTENSITY,
            },
            settle: None,
            drive: GlassDriveBinding::none(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassFieldMotionBinding {
    pub character: GlassCharacter,
    pub intensity: NumberValue,
    pub settle: f64,
    pub drive: GlassDriveBinding,
}

impl GlassFieldMotionBinding {
    pub fn defaults() -> Self {
        Self {
            character: GlassCharacter::DEFAULT,
            intensity: NumberValue::Static {
                value: DEFAULT_INTENSITY,
            },
            settle: DEFAULT_SETTLE_SECONDS,
            drive: GlassDriveBinding::none(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassForegroundIntent {
    pub tone: GlassForegroundTone,
    pub protection: NumberValue,
}

impl GlassForegroundIntent {
    pub fn defaults() -> Self {
        Self {
            tone: GlassForegroundTone::Auto,
            protection: NumberValue::Static {
                value: DEFAULT_PROTECTION,
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
    pub motion: GlassSurfaceMotionBinding,
    pub presence: NumberValue,
    pub foreground: GlassForegroundIntent,
}

/// Normalized field payload. The node itself is not a layout box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassFieldNode {
    pub field_id: GlassFieldId,
    pub material: GlassMaterialBinding,
    pub motion: GlassFieldMotionBinding,
    pub merge: GlassMergeIntent,
    pub environment: GlassEnvironmentBinding,
}
