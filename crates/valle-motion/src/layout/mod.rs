//! Takumi layout for Motion Artifacts: frame evaluation, node trees, layout boxes, and style
//! caches. This module produces no pixels. Dynamic values come from [`crate::MotionContext`]; the
//! CSS animation clock stays at zero.

mod activation;
pub(crate) mod bridge;
mod dependencies;
mod hit;
mod reuse;
mod review;
mod scene;
mod temporal;
mod transform;
pub use dependencies::{
    NodeDependencies, SceneDependencies, SceneReference, SceneReferenceKind, TemporalWindow,
};
pub use hit::layout_box_at_point;
pub use reuse::LayoutCache;
pub use review::{
    MotionReview, MotionReviewIssue, MotionTrajectory, MotionTrajectoryPoint, review_motion,
};

pub use bridge::{
    GlassLayoutEnvironment, GlassLayoutField, GlassLayoutForeground, GlassLayoutFrame,
    GlassLayoutMaterial, GlassLayoutMotion, GlassLayoutSurface, LayoutOptions, LayoutTree,
    ResolvedUnit, Scene3DFrameRequest, Scene3DRequest, StyleCache,
};
pub use scene::{
    FrameEvaluationStats, LayoutError, PreparedScene, build_tree, layout_boxes,
    prepare as prepare_scene, prepare_owned as prepare_owned_scene,
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
