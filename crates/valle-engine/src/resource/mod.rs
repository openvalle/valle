//! Immutable semantic identities and platform-free execution resource contracts.
//!
//! This module is the only bridge between authored/compiled semantics and backend-owned objects.
//! Semantic resources are frozen before a render can be compiled. Per-frame execution resources
//! are described by [`ResourceRequest`] values and bound by stable [`ExternalHandleId`] values;
//! decoders, promises, DOM objects, Skia objects and GPU handles never enter Engine IR.

mod color;
mod execution;
mod output;
mod semantic;

pub use valle_timeline::internal::ContentDigest;

pub use color::*;
pub use execution::*;
pub use output::*;
pub use semantic::*;
