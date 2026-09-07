//! Full-document Timeline editing DTOs.
//!
//! `editTimeline` has one job: replace the complete sparse Timeline document
//! against a known Project revision. It deliberately has no patch, command,
//! merge, resource-manifest, or execution-identity vocabulary.

use serde::{Deserialize, Deserializer, Serialize, de};

use super::timeline::{JsonObject, TimelineWire};

/// Submit one complete next `timeline.json` against the revision that was
/// originally read. `intent` is optional human-readable history context.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditTimelineRequestWire {
    /// Project-local linear revision read together with `timeline`.
    #[cfg_attr(
        feature = "schema",
        schemars(range(min = 1, max = 9007199254740991u64))
    )]
    #[serde(deserialize_with = "deserialize_revision")]
    pub base_revision: u64,
    pub timeline: TimelineWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

/// A compact edit result. The stale-base guard is intentionally the only
/// concurrency mechanism in the public API.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EditTimelineResponseWire {
    pub result: EditTimelineResultWire,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EditTimelineResultWire {
    Committed {
        #[cfg_attr(
            feature = "schema",
            schemars(range(min = 1, max = 9007199254740991u64))
        )]
        #[serde(deserialize_with = "deserialize_revision")]
        revision: u64,
    },
    Unchanged {
        #[cfg_attr(
            feature = "schema",
            schemars(range(min = 1, max = 9007199254740991u64))
        )]
        #[serde(deserialize_with = "deserialize_revision")]
        revision: u64,
    },
    StaleBase {
        #[cfg_attr(
            feature = "schema",
            schemars(range(min = 1, max = 9007199254740991u64))
        )]
        #[serde(deserialize_with = "deserialize_revision")]
        revision: u64,
    },
    Rejected {
        errors: Vec<EditErrorWire>,
    },
}

/// Validation failure after the request shape was decoded successfully.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditErrorWire {
    pub code: String,
    /// JSON Pointer from the edit request root. Timeline diagnostics begin at
    /// `/timeline`; request metadata diagnostics may use paths such as
    /// `/intent`.
    pub path: String,
    #[serde(default, skip_serializing_if = "JsonObject::is_empty")]
    pub details: JsonObject,
}

fn deserialize_revision<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let revision = u64::deserialize(deserializer)?;
    if (1..=9_007_199_254_740_991).contains(&revision) {
        Ok(revision)
    } else {
        Err(de::Error::custom(
            "revision must be a positive valle-json safe integer",
        ))
    }
}
