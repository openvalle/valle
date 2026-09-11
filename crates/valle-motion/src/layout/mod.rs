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
pub const LAYOUT_ENGINE_ID: &str = "takumi-core@0.23.1+fonts@4+css3d@3+media-box@1";

/// Whether Takumi recognizes an authored CSS property. Unknown declarations must not silently disappear.
pub fn supports_css_property(name: &str) -> bool {
    let mut parts = name.split('-');
    let mut camel = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            camel.extend(first.to_uppercase());
        }
        camel.extend(chars);
    }
    serde_json::from_value::<takumi_core::style::Style>(serde_json::json!({camel: "initial"}))
        .is_ok_and(|style| !style.declarations.is_empty())
}
