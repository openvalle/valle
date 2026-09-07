use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use valle_compiler::CompileTimelineError;
use valle_timeline::internal::CanonicalTimeline;
use valle_timeline::{Timeline, wire::edit::EditErrorWire};

/// Largest Project revision that every JSON/JavaScript adapter can represent
/// exactly. Public revisions are always in `1..=MAX_PUBLIC_REVISION`.
pub const MAX_PUBLIC_REVISION: u64 = 9_007_199_254_740_991;

/// Validate one Project revision at an adapter or store boundary without
/// introducing a second public revision wrapper.
pub const fn validate_public_revision(revision: u64) -> Result<u64, InvalidRevision> {
    if revision > 0 && revision <= MAX_PUBLIC_REVISION {
        Ok(revision)
    } else {
        Err(InvalidRevision)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("revision must be an integer from 1 through 9007199254740991")]
pub struct InvalidRevision;

/// Stable project identifier and safe on-disk path segment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidProjectId> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(InvalidProjectId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ProjectId {
    type Err = InvalidProjectId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for ProjectId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("project id must be 1-128 ASCII letters, digits, `-`, or `_`")]
pub struct InvalidProjectId;

/// Authenticated actor identity injected by the gateway, never read from an
/// edit payload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Actor(String);

impl Actor {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidActor> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || value.chars().any(|character| character.is_control())
        {
            return Err(InvalidActor);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Actor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("actor must be a non-empty, control-free string of at most 256 bytes")]
pub struct InvalidActor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedContext {
    actor: Actor,
}

impl AuthenticatedContext {
    pub fn new(actor: Actor) -> Self {
        Self { actor }
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }
}

/// Server-issued UTC timestamp. Project storage emits the canonical
/// millisecond `...SS.mmmZ` spelling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(String);

impl Timestamp {
    #[cfg(feature = "host")]
    pub(crate) fn server_issued(value: String) -> Self {
        debug_assert!(Self::is_canonical(&value));
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_canonical(value: &str) -> bool {
        let bytes = value.as_bytes();
        let shape = bytes.len() == 24
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':'
            && bytes[19] == b'.'
            && bytes[23] == b'Z'
            && bytes.iter().enumerate().all(|(index, byte)| {
                matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
            })
            && parse_component(bytes, 11, 13).is_some_and(|hour| hour <= 23)
            && parse_component(bytes, 14, 16).is_some_and(|minute| minute <= 59)
            && parse_component(bytes, 17, 19).is_some_and(|second| second <= 59);
        if !shape {
            return false;
        }
        let Some(year) = parse_component(bytes, 0, 4) else {
            return false;
        };
        let Some(month @ 1..=12) = parse_component(bytes, 5, 7) else {
            return false;
        };
        let Some(day) = parse_component(bytes, 8, 10) else {
            return false;
        };
        day != 0 && day <= days_in_month(year, month)
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 31,
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if Self::is_canonical(&value) {
            Ok(Self(value))
        } else {
            Err(de::Error::custom(
                "timestamp must use canonical UTC millisecond form",
            ))
        }
    }
}

fn parse_component(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    std::str::from_utf8(bytes.get(start..end)?)
        .ok()?
        .parse()
        .ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RevisionCause {
    Genesis,
    TimelineEdit,
    Restore { source_revision: u64 },
}

/// Authoritative immutable sparse Timeline revision metadata.
///
/// The public identity is the project-local linear `revision`. The host store
/// separately verifies one private digest over the persisted metadata and
/// `timeline.json`; runtime resources live outside Project history and never
/// create a new revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineRevision {
    /// Project-local, monotonically increasing public revision.
    pub revision: u64,
    pub parent_revision: Option<u64>,
    pub created_at: Timestamp,
    pub actor: Actor,
    pub cause: RevisionCause,
    pub intent: Option<String>,
}

/// One immutable Project authoring revision.
///
/// `timeline` is the authoritative sparse document persisted as
/// `timeline.json`. `canonical` is a deterministic derived view for internal
/// render consumers; it never participates in Project version identity.
/// Runtime resource fulfillment belongs to a separately admitted fixed
/// package and is never synthesized into this snapshot.
/// Fields stay private and there are no public constructors or setters, so a
/// snapshot returned by the store always carries views from the same read.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTimelineSnapshot {
    pub(crate) project_id: ProjectId,
    pub(crate) revision: TimelineRevision,
    pub(crate) timeline: Timeline,
    pub(crate) canonical: CanonicalTimeline,
}

impl ProjectTimelineSnapshot {
    #[cfg(feature = "host")]
    pub(crate) fn close(
        project_id: ProjectId,
        revision: TimelineRevision,
        timeline: Timeline,
        canonical: CanonicalTimeline,
    ) -> Self {
        Self {
            project_id,
            revision,
            timeline,
            canonical,
        }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn revision(&self) -> &TimelineRevision {
        &self.revision
    }

    /// The complete sparse document submitted by Agent or Studio and stored
    /// in this revision's public `timeline.json`.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Internal compiled projection. Its presence does not establish resource
    /// fulfillment or make the authoring snapshot renderable.
    #[doc(hidden)]
    pub fn canonical(&self) -> &CanonicalTimeline {
        &self.canonical
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotWriteResult {
    Committed { snapshot: ProjectTimelineSnapshot },
    Unchanged { snapshot: ProjectTimelineSnapshot },
    StaleBase { actual: ProjectTimelineSnapshot },
    Rejected { errors: Vec<EditErrorWire> },
}

/// One immutable Timeline revision. No edit log or derived entity diff
/// is persisted: sparse documents intentionally have no stable leaf IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedTimelineRevision {
    pub revision: u64,
    pub parent_revision: Option<u64>,
    pub created_at: Timestamp,
    pub actor: Actor,
    pub cause: RevisionCause,
    pub intent: Option<String>,
}

impl From<TimelineRevision> for ListedTimelineRevision {
    fn from(revision: TimelineRevision) -> Self {
        Self {
            revision: revision.revision,
            parent_revision: revision.parent_revision,
            created_at: revision.created_at,
            actor: revision.actor,
            cause: revision.cause,
            intent: revision.intent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPage {
    pub revisions: Vec<ListedTimelineRevision>,
    pub next_cursor: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreFault {
    #[error("project owner lock is unavailable")]
    OwnerUnavailable,
    #[error("project already exists")]
    AlreadyExists,
    #[error("project does not exist")]
    NotFound,
    #[error("project storage is corrupt: {0}")]
    Corrupt(&'static str),
    #[error("project storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("project metadata canonicalization failed")]
    Canonicalization,
    #[error("intent is invalid")]
    InvalidIntent,
    #[error(transparent)]
    InvalidRevision(#[from] InvalidRevision),
    #[error("timeline input failed admission: {0}")]
    InvalidTimelineInput(#[source] CompileTimelineError),
    #[error("revision page limit must be greater than zero")]
    InvalidPageLimit,
}

pub type ProjectReadError = StoreFault;

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn timestamp_rejects_impossible_calendar_dates() {
        assert!(Timestamp::is_canonical("2024-02-29T23:59:59.999Z"));
        assert!(!Timestamp::is_canonical("2023-02-29T00:00:00.000Z"));
        assert!(!Timestamp::is_canonical("2026-04-31T00:00:00.000Z"));
        assert!(!Timestamp::is_canonical("2026-01-00T00:00:00.000Z"));
    }
}
