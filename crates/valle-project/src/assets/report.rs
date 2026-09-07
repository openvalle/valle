//! Shared response envelopes and stable error codes. Success returns data; failure returns a code,
//! message, and optional recovery hint. Asset operations return current state without ledger
//! sequence numbers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable error codes serialized in snake_case for programmatic handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Missing or ambiguous asset, annotation, or entity; removed assets include a revival hint.
    NotFound,
    /// Unknown media format or probe failure.
    UnsupportedMedia,
    /// External reference no longer matches its size and modification-time baseline.
    StaleReference,
    /// Analysis budget reached; completed results are retained.
    BudgetExceeded,
    /// Analyzer timeout, nonzero exit, invalid JSON, or dependency failure.
    AnalyzerFailed,
    /// Write-lock acquisition timed out; retrying is safe.
    Locked,
    /// Invalid search or SQL query, including non-read-only SQL.
    BadQuery,
    /// Operation refused because a precondition requires an explicit override, such as purging
    /// human annotations without force.
    Refused,
    /// Filesystem error or corrupt stored data.
    Io,
}

/// Internal error with a stable code, readable message, and optional recovery hint.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct AssetsError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,
}

impl AssetsError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        AssetsError {
            code,
            message: message.into(),
            hint: None,
        }
    }

    /// Attach a recovery hint.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::NotFound, message)
    }
    pub fn unsupported_media(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::UnsupportedMedia, message)
    }
    pub fn stale_reference(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::StaleReference, message)
    }
    pub fn budget_exceeded(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::BudgetExceeded, message)
    }
    pub fn analyzer_failed(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::AnalyzerFailed, message)
    }
    pub fn locked(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::Locked, message)
    }
    pub fn bad_query(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::BadQuery, message)
    }
    pub fn refused(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::Refused, message)
    }
    pub fn io(message: impl Into<String>) -> Self {
        AssetsError::new(ErrorCode::Io, message)
    }
}

/// Map filesystem errors to the stable IO code.
impl From<std::io::Error> for AssetsError {
    fn from(e: std::io::Error) -> Self {
        AssetsError::new(ErrorCode::Io, e.to_string())
    }
}

/// Map SQLite errors to IO; a damaged projection can be rebuilt.
impl From<rusqlite::Error> for AssetsError {
    fn from(e: rusqlite::Error) -> Self {
        AssetsError::new(ErrorCode::Io, format!("sqlite: {e}"))
            .with_hint("rebuild the index with `valle assets reindex`")
    }
}

pub type Result<T> = std::result::Result<T, AssetsError>;

/// Error payload for the response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Shared response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Nonfatal warnings, such as unavailable probing or a reference fallback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Report {
    pub fn data(data: Value) -> Self {
        Report {
            ok: true,
            data: Some(data),
            warnings: Vec::new(),
            error: None,
        }
    }

    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    pub fn from_error(e: AssetsError) -> Self {
        Report {
            ok: false,
            data: None,
            warnings: Vec::new(),
            error: Some(ErrorBody {
                code: e.code,
                message: e.message,
                hint: e.hint,
            }),
        }
    }
}

impl From<Result<Report>> for Report {
    fn from(r: Result<Report>) -> Self {
        match r {
            Ok(report) => report,
            Err(e) => Report::from_error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_serialize_snake_case() {
        let all = [
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::UnsupportedMedia, "unsupported_media"),
            (ErrorCode::StaleReference, "stale_reference"),
            (ErrorCode::BudgetExceeded, "budget_exceeded"),
            (ErrorCode::AnalyzerFailed, "analyzer_failed"),
            (ErrorCode::Locked, "locked"),
            (ErrorCode::BadQuery, "bad_query"),
            (ErrorCode::Refused, "refused"),
            (ErrorCode::Io, "io"),
        ];
        for (code, s) in all {
            assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(s));
            let back: ErrorCode = serde_json::from_value(serde_json::json!(s)).unwrap();
            assert_eq!(back, code);
        }
    }

    #[test]
    fn envelope_roundtrip() {
        let ok = Report::data(serde_json::json!({"hash": "3f8ac2"}));
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v.get("error").is_none());
        let back: Report = serde_json::from_value(v).unwrap();
        assert!(back.ok);

        let err = Report::from_error(
            AssetsError::stale_reference("source file changed").with_hint("add again or verify"),
        );
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "stale_reference");
        assert_eq!(v["error"]["hint"], "add again or verify");
    }
}
