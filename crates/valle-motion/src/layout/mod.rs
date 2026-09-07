//! Takumi layout for Motion Artifacts: frame evaluation, node trees, layout boxes, and style
//! caches. This module produces no pixels. Dynamic values come from [`crate::MotionContext`]; the
//! CSS animation clock stays at zero.

pub(crate) mod bridge;
mod scene;

pub use bridge::{
    GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LayoutOptions, LayoutTree,
    Scene3DFrameRequest, Scene3DRequest, StyleCache,
};
pub use scene::{
    LayoutError, PreparedScene, build_tree, layout_boxes, prepare as prepare_scene,
    prepare_owned as prepare_owned_scene,
};
pub use takumi_core::viewport::Viewport;

/// Layout implementation identity included in the Artifact build fingerprint.
pub const LAYOUT_ENGINE_ID: &str = "takumi-core@0.23.1+fonts@4+css3d@3";
