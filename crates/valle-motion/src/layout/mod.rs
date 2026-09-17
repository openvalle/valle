//! Takumi layout for Motion Artifacts: frame evaluation, node trees, layout boxes, and style
//! caches. This module produces no pixels. Dynamic values come from [`crate::MotionContext`]; the
//! CSS animation clock stays at zero.

pub(crate) mod bridge;
mod reuse;
mod scene;
mod transform;
pub use reuse::LayoutCache;

pub use bridge::{
    GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LayoutOptions, LayoutTree,
    Scene3DFrameRequest, Scene3DRequest, StyleCache,
};
pub use scene::{
    LayoutError, PreparedScene, build_tree, layout_boxes, prepare as prepare_scene,
    prepare_owned as prepare_owned_scene,
};
#[cfg(not(target_arch = "wasm32"))]
pub use scene::{LayoutTimings, build_tree_profiled};
pub use takumi_core::viewport::Viewport;

/// Layout implementation identity included in the Artifact build fingerprint.
pub const LAYOUT_ENGINE_ID: &str = "takumi-core@0.23.1+fonts@4+css3d@3+media-box@1";

/// Whether Motion admits an authored CSS property through the shared property specification.
pub fn supports_css_property(name: &str) -> bool {
    crate::style::supports_property(name)
}
