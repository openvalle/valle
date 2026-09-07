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
