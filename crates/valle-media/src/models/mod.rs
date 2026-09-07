//! Model discovery, installation, route selection, and task adapters.
//!
//! This layer operates on model artifacts and media-domain values. It does not open complete
//! media files and does not know about projects, timelines, or rendering.

mod api;
mod backend;
#[cfg(feature = "model-store")]
mod catalog;
mod manager;

/// Model sessions operating on decoded tensors and PCM. File workflows live in `tools`.
pub mod inference;
#[cfg(any(feature = "model-onnx", feature = "model-coreml"))]
pub mod runtime;
#[cfg(feature = "model-store")]
pub mod spec;
#[cfg(feature = "model-store")]
pub mod store;

pub(crate) mod adapters;
#[cfg_attr(not(feature = "model-store"), allow(dead_code))]
mod legacy_cache_migration;

pub use api::{
    ArtifactStatus, ArtifactVerification, InstallBackendSelection, InstallCapabilityPolicy,
    InstallRequest, InstallResult, InstalledArtifactState, ModelError, ModelErrorCode, ModelStatus,
    ResolveRequest, ResolvedArtifact, VerificationReport, VerifyRequest,
};
pub use backend::{ModelSelection, RunBackendPreference};
pub use manager::ModelManager;
#[cfg(feature = "model-store")]
#[allow(unused_imports)]
pub(crate) use manager::{ResolvedModel, ResolvedModelCandidates};
