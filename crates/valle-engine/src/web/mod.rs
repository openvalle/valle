//! Thin WASM host for the Product Compositor.
//!
//! `ProductEngine` is the sole browser semantic engine. JavaScript fulfills platform resources
//! and executes the packed RenderPlan; it never receives a Scene or Motion recording language.

mod product;
mod timeline;
pub use product::ProductEngine;
pub use timeline::{
    canonicalize_timeline_document, compile_timeline, normalize_timeline, timeline_document_view,
    timeline_source_time_delta_from_frames, timeline_time_from_frames,
};

/// Rebind ordinary Motion fonts supplied by the host before opening the player.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn with_motion_fonts(
    package_json: &str,
    timeline_json: &str,
    manifest_json: &str,
    bundle_json: &str,
    fonts_json: &str,
) -> Result<String, wasm_bindgen::JsError> {
    let fonts: Vec<crate::fixed_package::MotionFontBytes> = serde_json::from_str(fonts_json)
        .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
    let package = crate::fixed_package::with_motion_fonts(
        package_json,
        timeline_json,
        manifest_json,
        bundle_json,
        &fonts,
    )
    .map_err(|error| wasm_bindgen::JsError::new(&error))?;
    serde_json::to_string(&package).map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))
}
