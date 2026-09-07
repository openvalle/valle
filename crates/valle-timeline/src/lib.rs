//! Timeline public authoring and full-document edit boundary.
//!
//! Expanded render DTOs, immutable resource manifests, and the fixed render
//! identity are namespaced under [`internal`] instead of widening the authoring
//! root API.

// Internal helpers.
mod canonical_json;

pub mod caption_presets;
pub mod color_value;
pub mod edit;
#[doc(hidden)]
pub mod internal;
pub mod quantize;
#[cfg(feature = "schema")]
pub mod schema;
pub mod time;
pub mod timeline;
pub mod wire;

pub use caption_presets::{
    CAPTION_PRESET_COUNT, CAPTION_PRESET_PACK_ID, CaptionPreset, CaptionPresetDescriptor,
    CaptionPresetPhase, DisplayCaptionPresetName, EnterCaptionPresetName, ExitCaptionPresetName,
};
pub use color_value::Color;
pub use edit::{EditDecodeError, EditJsonIssue, decode_edit_request};
pub use time::{ExactRational, FrameRate, RationalRate, RationalTime, TimeError};
pub use timeline::{
    Timeline, TimelineDecodeError, TimelineDiagnostic, TimelineEncodeError, TimelineJsonIssue,
    TimelineValidationReport, decode_timeline, timeline_bytes,
};
