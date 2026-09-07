//! Timeline admission, immutable render evaluation, preparation and
//! compositor semantics shared by native and Web hosts.

mod canonical;
pub mod compositor;
pub mod fixed_package;
pub mod frame;
pub mod prepare;
pub mod product;
pub mod render;
pub mod resource;
// Fixed-package caption shaping is an internal Prepare service, not an authoring surface.
#[cfg(feature = "text")]
pub(crate) mod text_semantic;
#[cfg(target_arch = "wasm32")]
mod wasm_glass;
#[cfg(feature = "web")]
pub mod web;

/// Motion remains a public renderer domain dependency.
pub use valle_motion as motion;
