//! Property/value failures shared by CSS and utility entry points.
use serde::{Deserialize, Serialize};

use super::PropertyAdmissionError;
use crate::diag::DiagCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum StyleIssueKind {
    UnknownProperty,
    UnsupportedProperty,
    UnsupportedValue,
    InvalidValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleIssue {
    pub kind: StyleIssueKind,
    pub property: String,
    pub value: String,
    pub reason: String,
    /// Concrete replacement the author can paste. It must be something the current version
    /// accepts: a message that points at a removed or unbuilt feature is worse than no message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl StyleIssue {
    pub fn new(
        kind: StyleIssueKind,
        property: impl Into<String>,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            property: property.into(),
            value: value.into(),
            reason: reason.into(),
            suggestion: None,
        }
    }

    /// Attach the replacement an author should write instead.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn admission(error: PropertyAdmissionError, value: &str) -> Self {
        match error {
            PropertyAdmissionError::Unknown(property) => Self::new(
                StyleIssueKind::UnknownProperty,
                property,
                value,
                "unknown or noncanonical CSS property; use its kebab-case CSS name",
            ),
            PropertyAdmissionError::Unavailable {
                property,
                reason,
                suggestion,
            } => Self::new(StyleIssueKind::UnsupportedProperty, property, value, reason)
                .with_suggestion(suggestion),
        }
    }

    pub fn code(&self) -> DiagCode {
        match self.kind {
            StyleIssueKind::UnknownProperty => DiagCode::StyleUnknownProperty,
            StyleIssueKind::UnsupportedProperty => DiagCode::StyleUnsupportedProperty,
            StyleIssueKind::UnsupportedValue => DiagCode::StyleUnsupportedValue,
            StyleIssueKind::InvalidValue => DiagCode::StyleInvalidValue,
        }
    }

    /// The value, reason and replacement, without the property name.
    ///
    /// A caller that already labels the diagnostic with the property (`style.width: …`) must use
    /// this: printing the property again reads as a stutter and buries the replacement.
    pub fn detail(&self) -> String {
        // A class-level rejection carries the class as both property and value; printing it again
        // would say nothing the label has not said already.
        let mut detail = if self.value.is_empty() || self.value == self.property {
            self.reason.clone()
        } else {
            format!("`{}`: {}", self.value, self.reason)
        };
        if let Some(suggestion) = &self.suggestion {
            detail.push_str("; use ");
            detail.push_str(suggestion);
        }
        detail
    }
}

impl std::fmt::Display for StyleIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}: {}`: {}", self.property, self.value, self.reason)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, "; use {suggestion}")?;
        }
        Ok(())
    }
}
impl std::error::Error for StyleIssue {}
