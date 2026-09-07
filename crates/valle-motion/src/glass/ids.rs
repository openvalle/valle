use serde::{Deserialize, Serialize};

/// Stable Glass surface identity inside one admitted Motion Artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
#[serde(transparent)]
pub struct GlassSurfaceId(String);

impl GlassSurfaceId {
    pub fn new(id: impl Into<String>) -> Result<Self, GlassIdError> {
        parse_id(id.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable Glass field identity inside one admitted Motion Artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
#[serde(transparent)]
pub struct GlassFieldId(String);

impl GlassFieldId {
    pub fn new(id: impl Into<String>) -> Result<Self, GlassIdError> {
        parse_id(id.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassIdError {
    Empty,
    TooLong,
}

impl std::fmt::Display for GlassIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "Glass id must be a non-empty static string"),
            Self::TooLong => write!(formatter, "Glass id must be at most 128 bytes"),
        }
    }
}

impl std::error::Error for GlassIdError {}

fn parse_id(id: String) -> Result<String, GlassIdError> {
    if id.is_empty() {
        Err(GlassIdError::Empty)
    } else if id.len() > 128 {
        Err(GlassIdError::TooLong)
    } else {
        Ok(id)
    }
}
