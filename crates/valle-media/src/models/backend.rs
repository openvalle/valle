use serde::{Deserialize, Serialize};

/// Product-level inference backend preference.
///
/// Execution providers remain an implementation detail of an ONNX route.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunBackendPreference {
    #[default]
    Auto,
    CoreMl,
    Onnx,
}

/// A model is selected independently from the backend route used to run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub backend: RunBackendPreference,
}

impl ModelSelection {
    pub fn pinned_default(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: None,
            backend: RunBackendPreference::Auto,
        }
    }
}
