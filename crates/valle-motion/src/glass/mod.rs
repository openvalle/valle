//! Motion Glass typed contracts, continuity, and composition-local track sampling.
//!
//! G4.7: `motion-glass` is admitted in production — `artifact::BASE_CAPABILITIES` lists
//! the capability, the production compiler emits these nodes, and the Artifact validator
//! covers the typed schema (G3.2).

mod continuity;
mod ids;
mod intent;
mod track;
mod validate;

pub use continuity::{TemporalContinuity, infer_continuity};
pub use ids::{GlassFieldId, GlassIdError, GlassSurfaceId};
pub use intent::{
    DEFAULT_CLARITY, DEFAULT_DEPTH, DEFAULT_INTENSITY, DEFAULT_MERGE_DISTANCE, DEFAULT_PRESENCE,
    DEFAULT_PROTECTION, DEFAULT_SETTLE_SECONDS, GlassCharacter, GlassDriveBinding,
    GlassEnvironmentBinding, GlassFieldMotionBinding, GlassFieldNode, GlassForegroundIntent,
    GlassForegroundTone, GlassLightBinding, GlassLightSpace, GlassMaterialBinding,
    GlassMergeIntent, GlassNode, GlassShapeBinding, GlassSurfaceMotionBinding, GlassUsage,
    GlassUsageProfile, GlassUsageRole, MOTION_GLASS_CAPABILITY, MOTION_GLASS_SCHEMA_ID,
};
pub use track::{GlassLocalShape, GlassSurfaceTrackSample, GlassTrackError, sample_tracks};
pub use validate::{
    field_membership, glass_track_expr_roots, reachable_exprs, validate_glass_schema,
};
