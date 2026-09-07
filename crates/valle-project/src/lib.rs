//! Immutable Timeline projects and the content-addressed asset library.
//!
//! Project history is a linear sequence of complete sparse Timeline
//! snapshots. Canonical Timeline and ResourceManifest values remain internal
//! render views. New authoring content has exactly one public persistence
//! entry point: revision::ProjectStore::edit_timeline.

pub use valle_timeline::internal::ContentDigest;

#[cfg(feature = "host")]
pub mod assets;
#[cfg(feature = "host")]
mod resource_gc;
pub mod revision;
