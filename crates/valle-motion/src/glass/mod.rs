//! Motion Glass: a box drawn in glass, and GlassField, which merges its Glass into one surface.
//! Glass lowers to ordinary DrawProgram groups and one built-in shader; see [`optics`].

mod intent;
pub mod optics;
mod validate;

pub use intent::{
    DEFAULT_BEVEL_OF_SHORTER_SIDE, DEFAULT_LIGHT_ELEVATION, DEFAULT_LIGHT_INTENSITY,
    DEFAULT_MERGE_DISTANCE, DEFAULT_PRESENCE, DEFAULT_REFRACTION_OF_BEVEL, GlassFieldNode,
    GlassLightBinding, GlassMaterialBinding, GlassMergeIntent, GlassNode, MERGE_MAX_DISTANCE,
    MERGE_MIN_DISTANCE, MOTION_GLASS_CAPABILITY,
};
pub use validate::{field_membership, validate_glass_schema};
