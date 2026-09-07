use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::{ModelSelection, RunBackendPreference};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallBackendSelection {
    #[default]
    Auto,
    CoreMl,
    Onnx,
    All,
}

/// Capability boundary used when selecting artifacts for installation.
///
/// A model-management-only binary may mirror and verify artifacts even when it deliberately
/// carries no inference adapter. A product binary that also exposes media tools must instead
/// select only routes that its compiled adapters can actually open. The caller chooses the mode
/// explicitly so the model store does not pretend to know product runtime capabilities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallCapabilityPolicy {
    #[default]
    ArtifactManagement,
    CompiledAdapters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRequest {
    pub id: String,
    pub version: Option<String>,
    pub backend: InstallBackendSelection,
    pub refresh_catalog: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyRequest {
    pub id: String,
    pub version: Option<String>,
    pub backend: Option<RunBackendPreference>,
    pub artifact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveRequest {
    pub selection: ModelSelection,
    #[serde(default)]
    pub allow_unverified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledArtifactState {
    Missing,
    Ready,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactStatus {
    pub id: String,
    pub backend: String,
    pub route: String,
    pub route_verified: bool,
    pub compatible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incompatibility: Option<String>,
    pub state: InstalledArtifactState,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelStatus {
    pub id: String,
    pub display_name: String,
    pub default_version: String,
    pub artifacts: Vec<ArtifactStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedArtifact {
    pub id: String,
    pub version: String,
    pub revision: String,
    pub manifest_path: String,
    pub adapter: String,
    pub adapter_version: u32,
    pub artifact: String,
    pub route: String,
    pub backend: String,
    pub precision: String,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallResult {
    pub artifacts: Vec<ResolvedArtifact>,
    pub downloaded_files: usize,
    pub reused_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactVerification {
    pub artifact: String,
    pub state: InstalledArtifactState,
    pub files: usize,
    pub bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationReport {
    pub id: String,
    pub version: String,
    pub revision: String,
    pub artifacts: Vec<ArtifactVerification>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelErrorCode {
    CatalogUnavailable,
    UnknownModel,
    InvalidSelection,
    AuthRequired,
    NoCompatibleRoute,
    NotInstalled,
    IntegrityFailed,
    UnsupportedAdapter,
    RuntimeUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelError {
    pub code: ModelErrorCode,
    pub message: String,
    pub hint: Option<String>,
}

impl ModelError {
    pub fn new(code: ModelErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(hint) = &self.hint {
            write!(formatter, "\n{hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ModelError {}
