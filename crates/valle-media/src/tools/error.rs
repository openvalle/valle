use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    ModelNotInstalled,
    CatalogUnavailable,
    AuthRequired,
    ArtifactIntegrity,
    NoCompatibleRoute,
    UnsupportedAdapter,
    RuntimeUnavailable,
    InvalidInput,
    ResourceBusy,
    ModelLoadFailed,
    InferenceFailed,
    OutputValidationFailed,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub retryable: bool,
    /// Sanitized machine-readable context. Never insert tokens, signed URLs, or raw argv here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            retryable: false,
            context: BTreeMap::new(),
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::InvalidInput, message)
    }

    pub fn cancelled() -> Self {
        Self::new(ToolErrorCode::Cancelled, "media tool cancelled")
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Preserve model-selection errors and distinguish a missing/incompatible inference runtime
    /// from a corrupt graph or other model-load failure.
    #[allow(dead_code)] // The helper is shared by independently enabled model-tool features.
    pub(crate) fn model_load(action: &str, error: anyhow::Error) -> Self {
        if let Some(model) = error.downcast_ref::<crate::models::ModelError>() {
            return Self::from(model.clone());
        }
        let details = format!("{error:#}");
        let normalized = details.to_ascii_lowercase();
        let runtime_unavailable = normalized.contains("onnx runtime")
            && (normalized.contains("unsupported onnx runtime")
                || normalized.contains("failed to load")
                || normalized.contains("failed to resolve")
                || normalized.contains("does not export ortgetapibase")
                || normalized.contains("null api base")
                || normalized.contains("not packaged")
                || normalized.contains("not compiled"));
        Self::new(
            if runtime_unavailable {
                ToolErrorCode::RuntimeUnavailable
            } else {
                ToolErrorCode::ModelLoadFailed
            },
            format!("{action}: {details}"),
        )
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(hint) = &self.hint {
            write!(formatter, "\n{hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ToolError {}

impl From<crate::models::ModelError> for ToolError {
    fn from(error: crate::models::ModelError) -> Self {
        use crate::models::ModelErrorCode as Model;

        let code = match error.code {
            Model::CatalogUnavailable => ToolErrorCode::CatalogUnavailable,
            Model::AuthRequired => ToolErrorCode::AuthRequired,
            Model::NoCompatibleRoute => ToolErrorCode::NoCompatibleRoute,
            Model::NotInstalled => ToolErrorCode::ModelNotInstalled,
            Model::IntegrityFailed => ToolErrorCode::ArtifactIntegrity,
            Model::UnsupportedAdapter => ToolErrorCode::UnsupportedAdapter,
            Model::RuntimeUnavailable => ToolErrorCode::RuntimeUnavailable,
            Model::UnknownModel | Model::InvalidSelection => ToolErrorCode::InvalidInput,
            Model::Internal => ToolErrorCode::Internal,
        };
        Self {
            code,
            message: error.message,
            hint: error.hint,
            retryable: false,
            context: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_bundle_failures_have_the_runtime_exit_class() {
        let error = ToolError::model_load(
            "load model",
            anyhow::anyhow!("failed to load ONNX Runtime from /tmp/libonnxruntime.so"),
        );
        assert_eq!(error.code, ToolErrorCode::RuntimeUnavailable);
    }

    #[test]
    fn invalid_graphs_remain_model_load_failures() {
        let error = ToolError::model_load(
            "load model",
            anyhow::anyhow!("failed to load ONNX model: invalid protobuf"),
        );
        assert_eq!(error.code, ToolErrorCode::ModelLoadFailed);
    }
}
