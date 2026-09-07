//! Valle-owned model contracts and pinned Hugging Face download metadata.
//!
//! This module deliberately has no inference or HTTP dependencies. A consumer can inspect a
//! catalog and select an artifact without pulling ONNX Runtime or Apple frameworks into its
//! dependency graph.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

/// Direct upstream models use a manifest owned by Valle, not a republished HF package.
pub fn uses_local_manifest(model_id: &str) -> bool {
    matches!(model_id, "qwen3-asr-0.6b" | "qwen3-aligner-0.6b")
}

/// Official Qwen repositories declare Apache-2.0 but do not ship a standalone license file.
/// The locally supplied text is subject to the same manifest hash checks as downloaded files.
pub(crate) fn embedded_support_file(
    manifest: &ModelManifest,
    file: &ArtifactFile,
) -> Option<&'static [u8]> {
    (uses_local_manifest(&manifest.model.id) && file.path == "card/LICENSE")
        .then_some(include_bytes!("catalog/qwen-LICENSE").as_slice())
}
/// Catalog snapshot shipped with the media crate for deterministic offline discovery.
///
/// Remote catalog refresh is an explicit product operation; it must never be required merely to
/// enumerate the releases known when this crate was built.
pub const EMBEDDED_CATALOG_JSON: &str = include_str!("catalog/catalog.v1.json");

pub fn embedded_catalog() -> Catalog {
    serde_json::from_str(EMBEDDED_CATALOG_JSON)
        .expect("the tested embedded model catalog must remain valid JSON")
}

macro_rules! embedded_release {
    ($model:literal, $version:literal) => {
        EmbeddedReleaseSnapshot {
            model: $model,
            version: $version,
            json: include_str!(concat!("catalog/release-", $model, "-", $version, ".json")),
        }
    };
}

#[derive(Debug, Clone, Copy)]
pub struct EmbeddedReleaseSnapshot {
    pub model: &'static str,
    pub version: &'static str,
    pub json: &'static str,
}

pub const EMBEDDED_RELEASES: &[EmbeddedReleaseSnapshot] = &[
    embedded_release!("birefnet", "1.0.0"),
    embedded_release!("demucs", "1.0.0"),
    embedded_release!("dpdfnet", "1.0.0"),
    embedded_release!("edgetam", "1.0.0"),
    embedded_release!("lama", "1.0.0"),
    embedded_release!("modnet", "1.0.0"),
    embedded_release!("omnishotcut", "1.0.0"),
    embedded_release!("realesrgan", "1.0.0"),
    embedded_release!("rife", "1.0.0"),
    embedded_release!("transnetv2", "1.0.0"),
    embedded_release!("qwen3-asr-0.6b", "1.0.0"),
    embedded_release!("qwen3-aligner-0.6b", "1.0.0"),
];

/// Resolve an offline release manifest and verify it against the embedded catalog identity.
pub fn embedded_release_manifest(model: &str, version: &str) -> Result<ModelManifest, String> {
    let snapshot = EMBEDDED_RELEASES
        .iter()
        .find(|snapshot| snapshot.model == model && snapshot.version == version)
        .ok_or_else(|| format!("no embedded release manifest for {model:?}@{version:?}"))?;
    let manifest: ModelManifest = serde_json::from_str(snapshot.json)
        .map_err(|error| format!("embedded release {model}@{version} is invalid JSON: {error}"))?;
    manifest
        .validate_publishable()
        .map_err(|error| format!("embedded release {model}@{version} is invalid: {error}"))?;

    let catalog = embedded_catalog();
    catalog
        .validate()
        .map_err(|error| format!("embedded catalog is invalid: {error}"))?;
    let catalog_model = catalog
        .model(model)
        .ok_or_else(|| format!("embedded catalog has no model {model:?}"))?;
    let release = catalog_model.release(Some(version))?;
    if manifest.model.id != catalog_model.id
        || manifest.model.version != release.version
        || manifest.model.task != catalog_model.task
        || manifest.model.display_name != catalog_model.display_name
        || manifest.release.repository.as_deref() != Some(release.repository.as_str())
    {
        return Err(format!(
            "embedded release identity for {model}@{version} disagrees with the embedded catalog"
        ));
    }
    Ok(manifest)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub model: ModelIdentity,
    pub license: ModelLicense,
    pub provenance: Provenance,
    pub release: ReleaseState,
    pub contract: TaskContract,
    pub artifacts: Vec<Artifact>,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelIdentity {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub task: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelLicense {
    pub spdx: String,
    pub redistribution: Redistribution,
    #[serde(default)]
    pub files: Vec<ArtifactFile>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Redistribution {
    Allowed,
    Forbidden,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Provenance {
    pub repository: String,
    pub revision: String,
    #[serde(default)]
    pub weights: Vec<WeightSource>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WeightSource {
    pub id: String,
    pub source: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Publishing state of the per-model Hugging Face repository.
///
/// The immutable Hugging Face commit cannot be embedded in this document because that would
/// make the commit hash self-referential. It lives in [`CatalogRelease`] instead.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ReleaseState {
    pub state: ReleaseReadiness,
    /// `namespace/repository`; absent while the model is still a local draft.
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseReadiness {
    Draft,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskContract {
    /// Rust host adapter identifier, for example `modnet-image-matting`.
    pub adapter: String,
    pub version: u32,
    pub inputs: Vec<TensorSpec>,
    pub outputs: Vec<TensorSpec>,
    #[serde(default)]
    pub preprocess: Value,
    #[serde(default)]
    pub postprocess: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TensorSpec {
    pub name: String,
    pub dtype: String,
    pub layout: String,
    pub shape: Vec<Dimension>,
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum Dimension {
    Fixed(u64),
    Symbol(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Artifact {
    pub id: String,
    pub format: String,
    pub precision: String,
    /// File or package-directory path used to load this artifact.
    pub entrypoint: String,
    /// Every file that must be fetched. Multi-file ONNX and `.mlpackage` are first-class.
    pub files: Vec<ArtifactFile>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ArtifactFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Route {
    pub id: String,
    pub artifact: String,
    pub backend: Backend,
    pub platforms: Vec<Platform>,
    pub architectures: Vec<Architecture>,
    pub status: ValidationStatus,
    pub priority: i32,
    #[serde(default)]
    pub requirements: RouteRequirements,
    #[serde(default)]
    pub options: Value,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RouteRequirements {
    pub minimum_os: Option<String>,
    pub minimum_runtime: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    NativeCpu,
    OnnxCpu,
    OnnxCuda,
    OnnxDirectml,
    Coreml,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NativeCpu => "native-cpu",
            Self::OnnxCpu => "onnx-cpu",
            Self::OnnxCuda => "onnx-cuda",
            Self::OnnxDirectml => "onnx-directml",
            Self::Coreml => "coreml",
        })
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "native-cpu" => Ok(Self::NativeCpu),
            "onnx-cpu" => Ok(Self::OnnxCpu),
            "onnx-cuda" => Ok(Self::OnnxCuda),
            "onnx-directml" => Ok(Self::OnnxDirectml),
            "coreml" => Ok(Self::Coreml),
            _ => Err(format!(
                "unknown backend {value:?}; expected native-cpu, onnx-cpu, onnx-cuda, onnx-directml or coreml"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
pub enum Architecture {
    #[serde(rename = "aarch64")]
    Aarch64,
    #[serde(rename = "x86_64")]
    X86_64,
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Verified,
    Unverified,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Environment {
    pub platform: Platform,
    pub architecture: Architecture,
}

impl Environment {
    pub fn current() -> Result<Self, String> {
        let platform = match std::env::consts::OS {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            "linux" => Platform::Linux,
            other => return Err(format!("unsupported operating system {other}")),
        };
        let architecture = match std::env::consts::ARCH {
            "aarch64" => Architecture::Aarch64,
            "x86_64" => Architecture::X86_64,
            other => return Err(format!("unsupported architecture {other}")),
        };
        Ok(Self {
            platform,
            architecture,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPreference {
    Auto,
    Exact(Backend),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Catalog {
    pub schema_version: u32,
    #[serde(default)]
    pub generated_by: String,
    pub models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CatalogModel {
    pub id: String,
    pub display_name: String,
    pub task: String,
    pub latest: String,
    pub releases: Vec<CatalogRelease>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CatalogRelease {
    pub version: String,
    pub repository: String,
    /// Immutable Hugging Face commit, never `main` or a moving branch.
    pub revision: String,
    /// Manifest location relative to the artifact repository. For direct upstream models the
    /// manifest is supplied by Valle and this path anchors the artifact directory only.
    pub manifest_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    issues: Vec<String>,
}

struct ReleaseFileDeclaration {
    path: String,
    bytes: u64,
    sha256: String,
    is_license: bool,
    role: String,
}

impl ValidationError {
    pub fn issues(&self) -> &[String] {
        &self.issues
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "model protocol validation failed:")?;
        for issue in &self.issues {
            writeln!(f, "- {issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl ModelManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut issues = Vec::new();
        let mut release_files = HashMap::new();
        check_schema(self.schema_version, &mut issues);
        check_id("model.id", &self.model.id, &mut issues);
        check_semver("model.version", &self.model.version, &mut issues);
        if self.model.task.trim().is_empty() {
            issues.push("model.task must not be empty".into());
        }
        if self.license.spdx.trim().is_empty() {
            issues.push("license.spdx must not be empty".into());
        }
        if self.license.files.is_empty() {
            issues.push("license.files must include the distributable license text".into());
        }
        let mut license_files = HashSet::new();
        for file in &self.license.files {
            check_relative_path("license file", &file.path, &mut issues);
            if !license_files.insert(file.path.to_ascii_lowercase()) {
                issues.push(format!(
                    "license files contain duplicate or case-colliding path {:?}",
                    file.path
                ));
            }
            check_release_file_collision(&mut release_files, "license", file, true, &mut issues);
            if file.bytes == 0 {
                issues.push(format!("license file {:?} has zero bytes", file.path));
            }
            check_sha256(
                &format!("license file {:?}", file.path),
                &file.sha256,
                &mut issues,
            );
        }
        check_id("contract.adapter", &self.contract.adapter, &mut issues);
        if self.contract.version == 0 {
            issues.push("contract.version must be positive".into());
        }
        check_tensors("contract.inputs", &self.contract.inputs, &mut issues);
        check_tensors("contract.outputs", &self.contract.outputs, &mut issues);

        let mut artifact_ids = HashSet::new();
        for artifact in &self.artifacts {
            check_id("artifact.id", &artifact.id, &mut issues);
            if !artifact_ids.insert(artifact.id.as_str()) {
                issues.push(format!("duplicate artifact id {:?}", artifact.id));
            }
            if artifact.format.trim().is_empty() {
                issues.push(format!("artifact {:?} has an empty format", artifact.id));
            }
            check_relative_path("artifact.entrypoint", &artifact.entrypoint, &mut issues);
            if artifact.files.is_empty() {
                issues.push(format!("artifact {:?} has no files", artifact.id));
            }
            let mut files = HashSet::new();
            for file in &artifact.files {
                check_relative_path("artifact file", &file.path, &mut issues);
                if !files.insert(file.path.to_ascii_lowercase()) {
                    issues.push(format!(
                        "artifact {:?} contains duplicate or case-colliding file {:?}",
                        artifact.id, file.path
                    ));
                }
                check_release_file_collision(
                    &mut release_files,
                    &format!("artifact {:?}", artifact.id),
                    file,
                    false,
                    &mut issues,
                );
                if file.bytes == 0 {
                    issues.push(format!(
                        "artifact {:?} file {:?} has zero bytes",
                        artifact.id, file.path
                    ));
                }
                check_sha256(
                    &format!("artifact {:?} file {:?}", artifact.id, file.path),
                    &file.sha256,
                    &mut issues,
                );
            }
            let entrypoint_is_present = artifact.files.iter().any(|file| {
                file.path == artifact.entrypoint
                    || file
                        .path
                        .strip_prefix(&artifact.entrypoint)
                        .is_some_and(|rest| rest.starts_with('/'))
            });
            if !entrypoint_is_present {
                issues.push(format!(
                    "artifact {:?} entrypoint {:?} is not covered by its file list",
                    artifact.id, artifact.entrypoint
                ));
            }
        }
        if self.artifacts.is_empty() {
            issues.push("manifest must contain at least one artifact".into());
        }

        let mut route_ids = HashSet::new();
        for route in &self.routes {
            check_id("route.id", &route.id, &mut issues);
            if !route_ids.insert(route.id.as_str()) {
                issues.push(format!("duplicate route id {:?}", route.id));
            }
            if !artifact_ids.contains(route.artifact.as_str()) {
                issues.push(format!(
                    "route {:?} references unknown artifact {:?}",
                    route.id, route.artifact
                ));
            }
            if route.platforms.is_empty() {
                issues.push(format!("route {:?} has no platforms", route.id));
            }
            if route.architectures.is_empty() {
                issues.push(format!("route {:?} has no architectures", route.id));
            }
        }
        if self.routes.is_empty() {
            issues.push("manifest must contain at least one runtime route".into());
        }

        if self.release.state == ReleaseReadiness::Ready {
            if self.license.redistribution != Redistribution::Allowed {
                issues.push("a ready release must allow redistribution".into());
            }
            match self.release.repository.as_deref() {
                Some(repository) => check_repository(repository, &mut issues),
                None => issues.push("a ready release must name its Hugging Face repository".into()),
            }
            if !self
                .routes
                .iter()
                .any(|route| route.status == ValidationStatus::Verified)
            {
                issues.push("a ready release must have at least one verified runtime route".into());
            }
        }

        finish_validation(issues)
    }

    pub fn validate_publishable(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.release.state != ReleaseReadiness::Ready {
            return Err(ValidationError {
                issues: vec![
                    "release.state is draft; set it to ready only after all local gates pass"
                        .into(),
                ],
            });
        }
        Ok(())
    }

    pub fn artifact(&self, id: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|artifact| artifact.id == id)
    }

    pub fn resolve_route(
        &self,
        environment: Environment,
        preference: BackendPreference,
        allow_unverified: bool,
    ) -> Result<&Route, String> {
        let mut candidates: Vec<&Route> = self
            .routes
            .iter()
            .filter(|route| route.platforms.contains(&environment.platform))
            .filter(|route| route.architectures.contains(&environment.architecture))
            .filter(|route| match preference {
                BackendPreference::Auto => true,
                BackendPreference::Exact(backend) => route.backend == backend,
            })
            .filter(|route| {
                route.status == ValidationStatus::Verified
                    || (allow_unverified && route.status == ValidationStatus::Unverified)
            })
            .collect();
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        candidates.into_iter().next().ok_or_else(|| {
            let requested = match preference {
                BackendPreference::Auto => "auto".into(),
                BackendPreference::Exact(backend) => backend.to_string(),
            };
            format!(
                "no {requested} route for {}/{} with status {}",
                environment.platform,
                environment.architecture,
                if allow_unverified {
                    "verified or unverified"
                } else {
                    "verified"
                }
            )
        })
    }
}

impl Catalog {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut issues = Vec::new();
        check_schema(self.schema_version, &mut issues);
        let mut model_ids = HashSet::new();
        for model in &self.models {
            check_id("catalog model.id", &model.id, &mut issues);
            if !model_ids.insert(model.id.as_str()) {
                issues.push(format!("duplicate catalog model id {:?}", model.id));
            }
            check_semver("catalog model.latest", &model.latest, &mut issues);
            let mut versions = HashSet::new();
            for release in &model.releases {
                check_semver("catalog release.version", &release.version, &mut issues);
                if !versions.insert(release.version.as_str()) {
                    issues.push(format!(
                        "catalog model {:?} has duplicate release {:?}",
                        model.id, release.version
                    ));
                }
                check_repository(&release.repository, &mut issues);
                if !is_immutable_revision(&release.revision) {
                    issues.push(format!(
                        "catalog release {}@{} must use a 40- or 64-character immutable commit hash",
                        model.id, release.version
                    ));
                }
                check_relative_path(
                    "catalog release.manifest_path",
                    &release.manifest_path,
                    &mut issues,
                );
            }
            if !versions.contains(model.latest.as_str()) {
                issues.push(format!(
                    "catalog model {:?} latest {:?} is not present in releases",
                    model.id, model.latest
                ));
            }
        }
        finish_validation(issues)
    }

    pub fn model(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|model| model.id == id)
    }
}

impl CatalogModel {
    pub fn release(&self, version: Option<&str>) -> Result<&CatalogRelease, String> {
        let selected = version.unwrap_or(&self.latest);
        self.releases
            .iter()
            .find(|release| release.version == selected)
            .ok_or_else(|| format!("model {:?} has no release {:?}", self.id, selected))
    }
}

pub fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.starts_with('/') || value.ends_with('/') {
        return false;
    }
    value.split('/').all(is_portable_path_segment)
}

fn is_portable_path_segment(segment: &str) -> bool {
    if segment.is_empty()
        || matches!(segment, "." | "..")
        || segment.ends_with('.')
        || segment.ends_with(' ')
        || segment
            .chars()
            .any(|character| character.is_control() || r#"<>:"\|?*"#.contains(character))
    {
        return false;
    }
    let basename = segment
        .split_once('.')
        .map_or(segment, |(basename, _)| basename)
        .to_ascii_uppercase();
    !matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !is_numbered_windows_device(&basename, "COM")
        && !is_numbered_windows_device(&basename, "LPT")
}

fn is_numbered_windows_device(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

pub fn is_immutable_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn check_schema(version: u32, issues: &mut Vec<String>) {
    if version != SCHEMA_VERSION {
        issues.push(format!(
            "schema_version {version} is unsupported; expected {SCHEMA_VERSION}"
        ));
    }
}

fn check_id(field: &str, value: &str, issues: &mut Vec<String>) {
    let valid = !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !valid {
        issues.push(format!(
            "{field} {value:?} must contain only lowercase ASCII letters, digits, '-', '_' or '.'"
        ));
    }
}

fn check_semver(field: &str, value: &str, issues: &mut Vec<String>) {
    if let Err(error) = Version::parse(value) {
        issues.push(format!(
            "{field} {value:?} is not semantic versioning: {error}"
        ));
    }
}

fn check_repository(value: &str, issues: &mut Vec<String>) {
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part == &"."
                || part == &".."
                || part.chars().any(char::is_whitespace)
        })
    {
        issues.push(format!(
            "Hugging Face repository {value:?} must be namespace/repository"
        ));
    }
}

fn check_tensors(field: &str, tensors: &[TensorSpec], issues: &mut Vec<String>) {
    if tensors.is_empty() {
        issues.push(format!("{field} must not be empty"));
    }
    let mut names = HashSet::new();
    for tensor in tensors {
        if tensor.name.trim().is_empty() {
            issues.push(format!("{field} contains an empty tensor name"));
        }
        if !names.insert(tensor.name.as_str()) {
            issues.push(format!(
                "{field} contains duplicate tensor {:?}",
                tensor.name
            ));
        }
        if tensor.dtype.trim().is_empty() || tensor.layout.trim().is_empty() {
            issues.push(format!(
                "{field} tensor {:?} must declare dtype and layout",
                tensor.name
            ));
        }
        if tensor.shape.is_empty() {
            issues.push(format!(
                "{field} tensor {:?} has an empty shape",
                tensor.name
            ));
        }
        if let Some([minimum, maximum]) = tensor.range
            && minimum > maximum
        {
            issues.push(format!(
                "{field} tensor {:?} has an inverted range",
                tensor.name
            ));
        }
    }
}

fn check_relative_path(field: &str, value: &str, issues: &mut Vec<String>) {
    if !is_safe_relative_path(value) {
        issues.push(format!("{field} {value:?} is not a safe relative path"));
    }
}

fn check_sha256(field: &str, value: &str, issues: &mut Vec<String>) {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        issues.push(format!("{field} has an invalid sha256 {value:?}"));
    }
}

fn check_release_file_collision(
    files: &mut HashMap<String, ReleaseFileDeclaration>,
    role: &str,
    file: &ArtifactFile,
    is_license: bool,
    issues: &mut Vec<String>,
) {
    let key = file.path.to_ascii_lowercase();
    if let Some(existing) = files.get(&key) {
        if existing.path != file.path {
            issues.push(format!(
                "release paths {:?} and {:?} differ only by case",
                existing.path, file.path
            ));
        }
        if existing.bytes != file.bytes || existing.sha256 != file.sha256 {
            issues.push(format!(
                "release path {:?} has conflicting declarations in {} and {role}",
                file.path, existing.role
            ));
        }
        if existing.is_license != is_license {
            issues.push(format!(
                "release path {:?} may not be both a license and an artifact file",
                file.path
            ));
        }
    } else {
        files.insert(
            key,
            ReleaseFileDeclaration {
                path: file.path.clone(),
                bytes: file.bytes,
                sha256: file.sha256.clone(),
                is_license,
                role: role.to_owned(),
            },
        );
    }
}

fn finish_validation(issues: Vec<String>) -> Result<(), ValidationError> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(ValidationError { issues })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT_RELEASES: &[(&str, &str, &str)] = &[
        (
            "birefnet",
            "1.0.0",
            include_str!("catalog/release-birefnet-1.0.0.json"),
        ),
        (
            "demucs",
            "1.0.0",
            include_str!("catalog/release-demucs-1.0.0.json"),
        ),
        (
            "dpdfnet",
            "1.0.0",
            include_str!("catalog/release-dpdfnet-1.0.0.json"),
        ),
        (
            "edgetam",
            "1.0.0",
            include_str!("catalog/release-edgetam-1.0.0.json"),
        ),
        (
            "lama",
            "1.0.0",
            include_str!("catalog/release-lama-1.0.0.json"),
        ),
        (
            "modnet",
            "1.0.0",
            include_str!("catalog/release-modnet-1.0.0.json"),
        ),
        (
            "omnishotcut",
            "1.0.0",
            include_str!("catalog/release-omnishotcut-1.0.0.json"),
        ),
        (
            "realesrgan",
            "1.0.0",
            include_str!("catalog/release-realesrgan-1.0.0.json"),
        ),
        (
            "rife",
            "1.0.0",
            include_str!("catalog/release-rife-1.0.0.json"),
        ),
        (
            "transnetv2",
            "1.0.0",
            include_str!("catalog/release-transnetv2-1.0.0.json"),
        ),
    ];

    const OFFICIAL_QWEN_RELEASES: &[(&str, &str, &str)] = &[
        (
            "qwen3-asr-0.6b",
            "1.0.0",
            include_str!("catalog/release-qwen3-asr-0.6b-1.0.0.json"),
        ),
        (
            "qwen3-aligner-0.6b",
            "1.0.0",
            include_str!("catalog/release-qwen3-aligner-0.6b-1.0.0.json"),
        ),
    ];

    fn modnet() -> ModelManifest {
        serde_json::from_str(include_str!("catalog/release-modnet-1.0.0.json")).unwrap()
    }

    #[test]
    fn modnet_release_obeys_v1_contract() {
        let manifest = modnet();
        manifest.validate().unwrap();
        manifest.validate_publishable().unwrap();
    }

    #[test]
    fn auto_prefers_verified_coreml_on_apple_silicon() {
        let manifest = modnet();
        let route = manifest
            .resolve_route(
                Environment {
                    platform: Platform::Macos,
                    architecture: Architecture::Aarch64,
                },
                BackendPreference::Auto,
                false,
            )
            .unwrap();
        assert_eq!(route.backend, Backend::Coreml);
    }

    #[test]
    fn native_cpu_backend_round_trips_and_resolves() {
        assert_eq!(
            serde_json::to_string(&Backend::NativeCpu).unwrap(),
            "\"native-cpu\""
        );
        assert_eq!(
            serde_json::from_str::<Backend>("\"native-cpu\"").unwrap(),
            Backend::NativeCpu
        );
        assert_eq!("native-cpu".parse::<Backend>().unwrap(), Backend::NativeCpu);

        let mut manifest = modnet();
        manifest.routes.truncate(1);
        manifest.routes[0].backend = Backend::NativeCpu;
        manifest.routes[0].platforms = vec![Platform::Linux];
        manifest.routes[0].architectures = vec![Architecture::X86_64];
        let route = manifest
            .resolve_route(
                Environment {
                    platform: Platform::Linux,
                    architecture: Architecture::X86_64,
                },
                BackendPreference::Exact(Backend::NativeCpu),
                false,
            )
            .unwrap();
        assert_eq!(route.backend, Backend::NativeCpu);
    }

    #[test]
    fn unverified_windows_route_needs_explicit_opt_in() {
        let manifest = modnet();
        let environment = Environment {
            platform: Platform::Windows,
            architecture: Architecture::X86_64,
        };
        assert!(
            manifest
                .resolve_route(environment, BackendPreference::Auto, false)
                .is_err()
        );
        assert_eq!(
            manifest
                .resolve_route(environment, BackendPreference::Auto, true)
                .unwrap()
                .backend,
            Backend::OnnxCpu
        );
    }

    #[test]
    fn unsupported_route_is_never_selectable() {
        let mut manifest = modnet();
        let route = manifest
            .routes
            .iter_mut()
            .find(|route| route.id == "onnx-cpu-windows-x86_64")
            .unwrap();
        route.status = ValidationStatus::Unsupported;
        let environment = Environment {
            platform: Platform::Windows,
            architecture: Architecture::X86_64,
        };

        for allow_unverified in [false, true] {
            assert!(
                manifest
                    .resolve_route(environment, BackendPreference::Auto, allow_unverified)
                    .is_err()
            );
        }
    }

    #[test]
    fn catalog_rejects_moving_revisions() {
        let catalog: Catalog = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "models": [{
                "id": "modnet",
                "display_name": "MODNet",
                "task": "image-matting",
                "latest": "1.0.0",
                "releases": [{
                    "version": "1.0.0",
                    "repository": "valle/modnet",
                    "revision": "main",
                    "manifest_path": "release.v1.json"
                }]
            }]
        }))
        .unwrap();
        assert!(catalog.validate().is_err());
    }

    #[test]
    fn every_catalog_release_has_a_valid_local_manifest() {
        let catalog = embedded_catalog();
        catalog.validate().unwrap();
        let count: usize = catalog
            .models
            .iter()
            .map(|model| model.releases.len())
            .sum();
        assert_eq!(EMBEDDED_RELEASES.len(), count);
        for model in &catalog.models {
            for release in &model.releases {
                embedded_release_manifest(&model.id, &release.version).unwrap();
            }
        }
        for (model, version, json) in CURRENT_RELEASES {
            let manifest = embedded_release_manifest(model, version).unwrap();
            assert_eq!(
                manifest,
                serde_json::from_str::<ModelManifest>(json).unwrap()
            );
        }
    }

    #[test]
    fn release_snapshots_have_no_verified_routes_for_unaudited_host_classes() {
        let audit = |model: &str, version: &str, json: &str| {
            let manifest: ModelManifest = serde_json::from_str(json).unwrap();
            for route in manifest
                .routes
                .iter()
                .filter(|route| route.status == ValidationStatus::Verified)
            {
                assert_eq!(
                    route.platforms.len(),
                    1,
                    "{}@{} route {:?} combines multiple platform claims",
                    model,
                    version,
                    route.id
                );
                assert_eq!(
                    route.architectures.len(),
                    1,
                    "{}@{} route {:?} combines multiple architecture claims",
                    model,
                    version,
                    route.id
                );
                assert_ne!(
                    route.platforms[0],
                    Platform::Windows,
                    "{}@{} route {:?} lacks a Windows real-fixture gate",
                    model,
                    version,
                    route.id
                );
                assert_ne!(
                    route.architectures[0],
                    Architecture::X86_64,
                    "{}@{} route {:?} lacks an x86_64 real-fixture gate",
                    model,
                    version,
                    route.id
                );
            }
        };

        for snapshot in EMBEDDED_RELEASES {
            audit(snapshot.model, snapshot.version, snapshot.json);
        }
        for (model, version, json) in OFFICIAL_QWEN_RELEASES {
            audit(model, version, json);
        }
    }

    #[test]
    fn qwen_contracts_resolve_directly_to_pinned_official_repositories() {
        let catalog = embedded_catalog();
        for (model, version, json) in OFFICIAL_QWEN_RELEASES {
            let manifest: ModelManifest = serde_json::from_str(json).unwrap();
            manifest.validate_publishable().unwrap();
            assert_eq!(
                (manifest.model.id.as_str(), manifest.model.version.as_str()),
                (*model, *version)
            );
            let release = catalog
                .model(model)
                .unwrap()
                .release(Some(version))
                .unwrap();
            assert!(release.repository.starts_with("Qwen/"));
            assert_eq!(release.revision, manifest.provenance.revision);
            assert_eq!(release.manifest_path, "release.v1.json");
            embedded_release_manifest(model, version).unwrap();
        }
    }

    #[test]
    fn path_validation_requires_normalized_portable_paths() {
        assert!(is_safe_relative_path("models/model.onnx"));
        assert!(is_safe_relative_path("Data/com.apple.CoreML/model.mlmodel"));
        assert!(!is_safe_relative_path("../model.onnx"));
        assert!(!is_safe_relative_path("./model.onnx"));
        assert!(!is_safe_relative_path("models//model.onnx"));
        assert!(!is_safe_relative_path("models/model.onnx/"));
        assert!(!is_safe_relative_path("models\\model.onnx"));
        assert!(!is_safe_relative_path("C:/model.onnx"));
        assert!(!is_safe_relative_path("models/CON.txt"));
        assert!(!is_safe_relative_path("models/model?.onnx"));
        assert!(!is_safe_relative_path("/model.onnx"));
    }

    #[test]
    fn manifest_rejects_case_colliding_release_files() {
        let mut manifest = modnet();
        let mut duplicate = manifest.artifacts[0].files[0].clone();
        duplicate.path = duplicate.path.to_ascii_uppercase();
        manifest.artifacts[0].files.push(duplicate);
        let error = manifest.validate().unwrap_err();
        assert!(
            error
                .issues()
                .iter()
                .any(|issue| issue.contains("case-colliding"))
        );
    }

    #[test]
    fn committed_json_schemas_match_the_rust_types() {
        let generated_manifest =
            serde_json::to_value(schemars::schema_for!(ModelManifest)).unwrap();
        let committed_manifest: serde_json::Value = serde_json::from_str(include_str!(
            "catalog/schemas/model-manifest-v1.schema.json"
        ))
        .unwrap();
        assert_eq!(generated_manifest, committed_manifest);

        let generated_catalog = serde_json::to_value(schemars::schema_for!(Catalog)).unwrap();
        let committed_catalog: serde_json::Value =
            serde_json::from_str(include_str!("catalog/schemas/catalog-v1.schema.json")).unwrap();
        assert_eq!(generated_catalog, committed_catalog);
    }
}
