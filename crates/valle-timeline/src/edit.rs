//! Domain entry point for full-document Timeline edit decoding.

use serde::Serialize;

use crate::{
    canonical_json::{CanonicalJsonError, parse_strict},
    wire::edit::EditTimelineRequestWire,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditJsonIssue {
    Utf8Bom,
    InvalidUnicode,
    DuplicateObjectKey,
    UnsafeInteger,
    NonFiniteNumber,
    MalformedJson,
}

/// Stable edit-request decode failure. Parser implementation strings never
/// cross this boundary.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditDecodeError {
    #[error("input violates valle-json/1: {issue:?}")]
    CanonicalJson { issue: EditJsonIssue, path: String },
    #[error("request body does not match editTimeline")]
    InvalidShape,
}

/// Strict `valle-json/1` decode of the full-document edit wire.
///
/// This function deliberately does not validate or compile the embedded
/// Timeline: shape errors are decode errors, while timeline invariants belong
/// to the later `Rejected(EditReport)` stage.
pub fn decode_edit_request(json: &str) -> Result<EditTimelineRequestWire, EditDecodeError> {
    parse_strict(json.as_bytes()).map_err(map_canonical_json_error)?;
    serde_json::from_str(json).map_err(|_| EditDecodeError::InvalidShape)
}

fn map_canonical_json_error(error: CanonicalJsonError) -> EditDecodeError {
    let (issue, path) = match error {
        CanonicalJsonError::Utf8Bom => (EditJsonIssue::Utf8Bom, String::new()),
        CanonicalJsonError::InvalidUnicode => (EditJsonIssue::InvalidUnicode, String::new()),
        CanonicalJsonError::DuplicateObjectKey { key } => (
            EditJsonIssue::DuplicateObjectKey,
            format!("/<duplicate:{key}>"),
        ),
        CanonicalJsonError::UnsafeInteger { .. } => (EditJsonIssue::UnsafeInteger, String::new()),
        CanonicalJsonError::NonFiniteNumber => (EditJsonIssue::NonFiniteNumber, String::new()),
        CanonicalJsonError::MalformedJson | CanonicalJsonError::Encode => {
            (EditJsonIssue::MalformedJson, String::new())
        }
    };
    EditDecodeError::CanonicalJson { issue, path }
}
