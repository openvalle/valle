//! Workspace-internal render, resource, and execution contracts.
//!
//! These types live beside the public Timeline contract to avoid an extra
//! crate boundary, but they are deliberately namespaced under `internal` so
//! authoring callers do not mistake expanded render DTOs for public Timeline
//! input.

pub(crate) use crate::canonical_json;

pub mod canonical;
mod content_digest;
pub mod document;
pub mod fixed_package;
pub mod hash;
pub mod identity;
pub mod quantize;
pub mod resource_manifest;
#[cfg(feature = "schema")]
pub mod schema;
pub mod time;
pub mod wire;

pub use crate::{Color, ExactRational, FrameRate, RationalRate, RationalTime, TimeError};
pub use canonical::{
    CanonicalDecodeError, CanonicalError, CanonicalJsonIssue, CanonicalTimeline,
    ContractDiagnostic, DiagnosticPhase, DiagnosticSeverity, LocalInvariantReport, canonical_bytes,
    decode_canonical, encode_canonical,
};
pub use content_digest::{ContentDigest, InvalidContentDigest};
pub use document::*;
pub use identity::*;
pub use resource_manifest::{ResourceManifest, ResourceManifestError, decode_resource_manifest};
pub use time::{
    DiscontinuityIndex, FrameKey, MotionInstanceId, SampleTime, TimeMapSegment, TrackEpoch,
};
