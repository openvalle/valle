//! Motion Glass typed contracts.
//!
//! G4.7: `motion-glass` is admitted in production — `artifact::BASE_CAPABILITIES` lists
//! the capability, the production compiler emits these nodes, and the Artifact validator
//! covers the typed schema (G3.2).

mod ids;
mod intent;
mod validate;

pub use ids::{GlassFieldId, GlassIdError, GlassSurfaceId};
pub use intent::{
    DEFAULT_CLARITY, DEFAULT_DEPTH, DEFAULT_MERGE_DISTANCE, DEFAULT_PRESENCE,
    GlassEnvironmentBinding, GlassFieldNode, GlassLightBinding, GlassLightSpace,
    GlassMaterialBinding, GlassMergeIntent, GlassNode, GlassShapeBinding, MOTION_GLASS_CAPABILITY,
    MOTION_GLASS_SCHEMA_ID,
};
pub use validate::{field_membership, validate_glass_schema};
