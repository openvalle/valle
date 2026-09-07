use std::path::{Path, PathBuf};

use super::legacy_cache_migration;

#[cfg(feature = "model-store")]
use crate::models::spec::{Artifact, Backend, Environment, ModelManifest, Route, ValidationStatus};
#[cfg(feature = "model-store")]
use crate::models::store::{ArtifactKey, HubClient, InstallationState};
#[cfg(feature = "model-store")]
use std::collections::HashSet;

#[cfg(feature = "model-store")]
use super::{
    ArtifactStatus, ArtifactVerification, InstallBackendSelection, InstallCapabilityPolicy,
    InstallRequest, InstallResult, InstalledArtifactState, ModelError, ModelErrorCode, ModelStatus,
    ResolveRequest, ResolvedArtifact, RunBackendPreference, VerificationReport, VerifyRequest,
    catalog::{CatalogRepository, ReleaseBundle},
};

/// Product-side model management entry point.
///
/// Installation is the only operation allowed to use HTTP. Listing, verification, route
/// resolution, and every media tool invocation are deterministic and offline.
#[derive(Debug, Clone)]
pub struct ModelManager {
    models_root: PathBuf,
    #[cfg(feature = "model-store")]
    install_capability_policy: InstallCapabilityPolicy,
    #[cfg(all(feature = "model-store", feature = "model-qwen-native"))]
    legacy_models_root: PathBuf,
    #[cfg(feature = "model-store")]
    catalog: CatalogRepository,
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelManager {
    pub fn new() -> Self {
        #[cfg(feature = "model-store")]
        let models_root = crate::models::store::default_models_root()
            .unwrap_or_else(|_| legacy_cache_migration::models_root());
        #[cfg(not(feature = "model-store"))]
        let models_root = legacy_cache_migration::models_root();
        Self {
            #[cfg(feature = "model-store")]
            install_capability_policy: InstallCapabilityPolicy::ArtifactManagement,
            #[cfg(all(feature = "model-store", feature = "model-qwen-native"))]
            legacy_models_root: legacy_cache_migration::models_root(),
            #[cfg(feature = "model-store")]
            catalog: CatalogRepository::new(models_root.clone()),
            models_root,
        }
    }

    /// Build a manager rooted at an explicit model cache. Intended for packaging and tests.
    pub fn from_models_root(models_root: impl Into<PathBuf>) -> Self {
        let models_root = models_root.into();
        Self {
            #[cfg(feature = "model-store")]
            install_capability_policy: InstallCapabilityPolicy::ArtifactManagement,
            #[cfg(feature = "model-store")]
            catalog: CatalogRepository::new(models_root.clone()),
            #[cfg(all(feature = "model-store", feature = "model-qwen-native"))]
            legacy_models_root: models_root.clone(),
            models_root,
        }
    }

    /// Override the explicit-refresh source while retaining the same offline cache root.
    #[cfg(feature = "model-store")]
    pub fn with_catalog_source(models_root: impl Into<PathBuf>, source: impl Into<String>) -> Self {
        let models_root = models_root.into();
        Self {
            install_capability_policy: InstallCapabilityPolicy::ArtifactManagement,
            catalog: CatalogRepository::with_source(models_root.clone(), source),
            #[cfg(feature = "model-qwen-native")]
            legacy_models_root: models_root.clone(),
            models_root,
        }
    }

    pub fn models_root(&self) -> &Path {
        &self.models_root
    }

    /// Apply the product binary's adapter capability policy to subsequent installs.
    ///
    /// Keep [`InstallCapabilityPolicy::ArtifactManagement`] for a manager-only build. A binary
    /// exposing inference tools should inject [`InstallCapabilityPolicy::CompiledAdapters`] so
    /// `--backend auto` cannot install a higher-priority artifact that the binary cannot run.
    #[cfg(feature = "model-store")]
    pub fn with_install_capability_policy(mut self, policy: InstallCapabilityPolicy) -> Self {
        self.install_capability_policy = policy;
        self
    }

    /// Whether this `valle-media` build contains at least one runnable task adapter.
    ///
    /// This is exposed for the product assembly layer; it is not inferred by the model store.
    #[cfg(feature = "model-store")]
    pub const fn has_compiled_adapter_capabilities() -> bool {
        cfg!(any(
            feature = "model-birefnet-onnx",
            feature = "model-dpdfnet-onnx",
            feature = "model-demucs-onnx",
            feature = "model-edgetam-onnx",
            feature = "model-lama-onnx",
            feature = "model-modnet-onnx",
            all(
                feature = "model-omnishotcut-onnx",
                feature = "model-transnetv2-onnx"
            ),
            feature = "model-realesrgan-onnx",
            feature = "model-rife-onnx",
            feature = "model-qwen-native",
            all(feature = "model-coreml", target_os = "macos")
        ))
    }

    /// Device-compiled CoreML products are derived cache data, never installed artifact files.
    #[cfg(feature = "tool-matte")]
    pub(crate) fn compiled_coreml_root(&self) -> PathBuf {
        self.models_root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("compiled-coreml")
    }

    /// List the embedded pinned releases and local state of each declared route.
    #[cfg(feature = "model-store")]
    pub fn list(&self) -> Result<Vec<ModelStatus>, ModelError> {
        let environment = current_environment()?;
        let catalog = self.catalog.embedded();
        catalog.validate().map_err(|error| {
            ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!("embedded model catalog is invalid: {error}"),
            )
        })?;
        let statuses = catalog
            .models
            .iter()
            .map(|model| {
                let bundle = self.catalog.offline_release(&model.id, None)?;
                let artifacts = bundle
                    .manifest
                    .routes
                    .iter()
                    .map(|route| self.route_status(&bundle, route, environment))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ModelStatus {
                    id: model.id.clone(),
                    display_name: model.display_name.clone(),
                    default_version: model.latest.clone(),
                    artifacts,
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        Ok(statuses)
    }

    /// Explicitly install one or more host-runnable artifact routes.
    #[cfg(feature = "model-store")]
    pub fn install(
        &self,
        request: InstallRequest,
        progress: &mut dyn FnMut(&str),
    ) -> Result<InstallResult, ModelError> {
        let hub =
            HubClient::from_env().map_err(|error| store_error("initialize model store", error))?;
        let bundle = self.catalog.install_release(
            &request.id,
            request.version.as_deref(),
            request.refresh_catalog,
            &hub,
        )?;
        let environment = current_environment()?;
        let routes = select_install_routes(
            &bundle.manifest,
            request.backend,
            environment,
            self.install_capability_policy,
        )?;
        let fetched = crate::models::store::FetchedManifest {
            manifest: bundle.manifest.clone(),
            bytes: bundle.manifest_bytes.clone(),
        };
        let mut artifacts = Vec::with_capacity(routes.len());
        let mut downloaded_files = 0_usize;
        let mut reused_files = 0_usize;
        for route in routes {
            let artifact = bundle.manifest.artifact(&route.artifact).ok_or_else(|| {
                ModelError::new(
                    ModelErrorCode::CatalogUnavailable,
                    format!(
                        "route {} references missing artifact {}",
                        route.id, route.artifact
                    ),
                )
            })?;
            progress(&format!(
                "installing {}@{} artifact {} via {}",
                bundle.manifest.model.id,
                bundle.manifest.model.version,
                route.artifact,
                route.backend
            ));
            #[cfg(feature = "model-qwen-native")]
            if let Some(migrated) =
                self.migrate_legacy_qwen(&bundle, &fetched, route, artifact, progress)?
            {
                reused_files = reused_files
                    .saturating_add(migrated.migrated_legacy_files)
                    .saturating_add(migrated.materialized_embedded_files)
                    .saturating_add(migrated.reused_files);
                artifacts.push(resolved_artifact(
                    &bundle,
                    route,
                    artifact,
                    migrated.root,
                    migrated.entrypoint,
                ));
                continue;
            }
            let installed = hub
                .install_artifact(
                    &bundle.release,
                    &fetched,
                    &route.artifact,
                    &self.models_root,
                )
                .map_err(|error| store_error("install model artifact", error))?;
            downloaded_files = downloaded_files.saturating_add(installed.downloaded_files);
            reused_files = reused_files.saturating_add(installed.reused_files);
            artifacts.push(resolved_artifact(
                &bundle,
                route,
                artifact,
                installed.root,
                installed.entrypoint,
            ));
        }
        Ok(InstallResult {
            artifacts,
            downloaded_files,
            reused_files,
        })
    }

    /// Import a complete Phase-0 Qwen cache into its immutable artifact slot before considering
    /// any large download. The published manifest remains the sole source of file hashes and
    /// release identity; the old cache is only a migration source and is never a runtime route.
    #[cfg(all(feature = "model-store", feature = "model-qwen-native"))]
    fn migrate_legacy_qwen(
        &self,
        bundle: &ReleaseBundle,
        fetched: &crate::models::store::FetchedManifest,
        route: &Route,
        artifact: &Artifact,
        progress: &mut dyn FnMut(&str),
    ) -> Result<Option<crate::models::store::MigrationResult>, ModelError> {
        if artifact.id != "safetensors-bf16" {
            return Ok(None);
        }
        let Some(legacy_root) = legacy_cache_migration::migration_source(
            &self.legacy_models_root,
            &bundle.manifest.model.id,
        ) else {
            return Ok(None);
        };
        progress(&format!(
            "migrating verified legacy cache {} into immutable artifact {}",
            legacy_root.display(),
            artifact.id
        ));
        let embedded = [crate::models::store::EmbeddedMigrationFile {
            path: crate::models::inference::qwen_asr::APACHE_2_LICENSE_PATH,
            bytes: crate::models::inference::qwen_asr::APACHE_2_LICENSE_BYTES,
        }];
        let migrated = crate::models::store::migrate_legacy_artifact(
            &bundle.release,
            fetched,
            &route.artifact,
            &legacy_root,
            &embedded,
            &self.models_root,
        )
        .map_err(|error| store_error("migrate legacy Qwen artifact", error))?;
        progress(&format!(
            "migrated {} legacy file(s), materialized {} release file(s), reused {} installed file(s)",
            migrated.migrated_legacy_files,
            migrated.materialized_embedded_files,
            migrated.reused_files
        ));
        Ok(Some(migrated))
    }

    /// Verify selected installed artifacts without network access.
    #[cfg(feature = "model-store")]
    pub fn verify(&self, request: VerifyRequest) -> Result<VerificationReport, ModelError> {
        let bundle = self
            .catalog
            .offline_release(&request.id, request.version.as_deref())?;
        let selected = select_verification_artifacts(&bundle.manifest, &request)?;
        let mut artifacts = Vec::with_capacity(selected.len());
        for artifact in selected {
            let key = artifact_key(&bundle, &artifact.id);
            let status = crate::models::store::installed_artifact_status(&self.models_root, &key)
                .map_err(|error| store_error("inspect installed artifact", error))?;
            artifacts.push(ArtifactVerification {
                artifact: artifact.id,
                state: installation_state(status.state),
                files: status.files.unwrap_or(0),
                bytes: status.bytes.unwrap_or(0),
                error: status.error,
            });
        }
        Ok(VerificationReport {
            id: bundle.manifest.model.id,
            version: bundle.manifest.model.version,
            revision: bundle.release.revision,
            artifacts,
        })
    }

    /// Resolve and integrity-check one installed artifact without network access.
    #[cfg(feature = "model-store")]
    pub fn resolve_installed(
        &self,
        request: ResolveRequest,
    ) -> Result<ResolvedArtifact, ModelError> {
        Ok(self.resolve_model(request)?.resolved)
    }

    /// Internal form retained for task adapters; public callers only receive provenance and paths.
    #[cfg(feature = "model-store")]
    pub(crate) fn resolve_model(
        &self,
        request: ResolveRequest,
    ) -> Result<ResolvedModel, ModelError> {
        let mut candidates = self.resolve_model_candidates(request)?;
        Ok(candidates.models.remove(0))
    }

    /// Resolve every installed route that is eligible for this job, in preference order.
    ///
    /// Task session factories use the complete list only for `auto` load-time fallback. Explicit
    /// backend requests deliberately receive the same ordered list but must stop after the first
    /// load attempt; that policy remains in the tools layer where session creation occurs.
    #[cfg(feature = "model-store")]
    pub(crate) fn resolve_model_candidates(
        &self,
        request: ResolveRequest,
    ) -> Result<ResolvedModelCandidates, ModelError> {
        let bundle = self
            .catalog
            .offline_release(&request.selection.id, request.selection.version.as_deref())?;
        let environment = current_environment()?;
        let candidates = compatible_routes(
            &bundle.manifest,
            request.selection.backend,
            environment,
            request.allow_unverified,
        );
        if candidates.is_empty() {
            return Err(no_compatible_route_error(
                &bundle.manifest,
                request.selection.backend,
                environment,
            ));
        }

        let mut corrupt = Vec::new();
        let mut resolved_candidates = Vec::new();
        let mut fallback_install_hint = None;
        let explicit_backend = request.selection.backend != RunBackendPreference::Auto;
        for route in candidates {
            let key = artifact_key(&bundle, &route.artifact);
            let root = key
                .install_root(&self.models_root)
                .map_err(|error| store_error("locate installed artifact", error))?;
            if !root
                .try_exists()
                .map_err(|error| store_error("inspect installed artifact", error.into()))?
            {
                if !explicit_backend
                    && !resolved_candidates.is_empty()
                    && fallback_install_hint.is_none()
                {
                    fallback_install_hint = install_hint_for_route(
                        &bundle.manifest.model.id,
                        &bundle.manifest.model.version,
                        route,
                    );
                }
                continue;
            }
            let installed = match crate::models::store::resolve_installed(&self.models_root, &key) {
                Ok(installed) => installed,
                Err(error) => {
                    corrupt.push(format!("{}: {error:#}", route.id));
                    continue;
                }
            };
            if installed.manifest != bundle.manifest {
                return Err(ModelError::new(
                    ModelErrorCode::IntegrityFailed,
                    format!(
                        "installed manifest for {}@{} differs from the pinned release",
                        bundle.manifest.model.id, bundle.manifest.model.version
                    ),
                ));
            }
            let artifact = installed.artifact;
            let resolved = resolved_artifact(
                &bundle,
                route,
                &artifact,
                installed.root,
                installed.entrypoint,
            );
            resolved_candidates.push(ResolvedModel {
                manifest: bundle.manifest.clone(),
                route: route.clone(),
                artifact,
                resolved,
            });
            if explicit_backend {
                break;
            }
        }

        if !resolved_candidates.is_empty() {
            return Ok(ResolvedModelCandidates {
                models: resolved_candidates,
                fallback_install_hint,
            });
        }

        if !corrupt.is_empty() {
            return Err(ModelError::new(
                ModelErrorCode::IntegrityFailed,
                format!(
                    "installed routes for {}@{} failed verification: {}",
                    bundle.manifest.model.id,
                    bundle.manifest.model.version,
                    corrupt.join("; ")
                ),
            )
            .with_hint(format!(
                "run: valle models verify {} --version {}",
                bundle.manifest.model.id, bundle.manifest.model.version
            )));
        }
        let backend = run_backend_name(request.selection.backend);
        Err(ModelError::new(
            ModelErrorCode::NotInstalled,
            format!(
                "no installed {backend} artifact for {}@{}",
                bundle.manifest.model.id, bundle.manifest.model.version
            ),
        )
        .with_hint(format!(
            "run: valle models install {} --version {} --backend {backend}",
            bundle.manifest.model.id, bundle.manifest.model.version
        )))
    }

    #[cfg(feature = "model-store")]
    fn route_status(
        &self,
        bundle: &ReleaseBundle,
        route: &Route,
        environment: Environment,
    ) -> Result<ArtifactStatus, ModelError> {
        let key = artifact_key(bundle, &route.artifact);
        let status = crate::models::store::installed_artifact_status(&self.models_root, &key)
            .map_err(|error| store_error("inspect installed artifact", error))?;
        let compatibility = route_compatibility(
            &bundle.manifest,
            route,
            RunBackendPreference::Auto,
            environment,
            false,
        );
        Ok(ArtifactStatus {
            id: route.artifact.clone(),
            backend: route.backend.to_string(),
            route: route.id.clone(),
            route_verified: route.status == ValidationStatus::Verified,
            compatible: compatibility.is_ok(),
            incompatibility: compatibility.err(),
            state: installation_state(status.state),
            root: status.root,
        })
    }
}

#[cfg(feature = "model-store")]
#[derive(Debug, Clone)]
#[allow(dead_code)] // Consumed by independently enabled task adapters.
pub(crate) struct ResolvedModel {
    pub manifest: ModelManifest,
    pub route: Route,
    pub artifact: Artifact,
    pub resolved: ResolvedArtifact,
}

#[cfg(feature = "model-store")]
#[allow(dead_code)] // Extra fallback metadata is consumed only by inference-tool builds.
pub(crate) struct ResolvedModelCandidates {
    pub models: Vec<ResolvedModel>,
    /// Exact command to install the first lower-priority route that was absent locally.
    ///
    /// This is reported only if every already-installed candidate later fails to open. A
    /// successful preferred session must not warn merely because an optional fallback is absent.
    pub fallback_install_hint: Option<String>,
}

#[cfg(feature = "model-store")]
fn current_environment() -> Result<Environment, ModelError> {
    Environment::current()
        .map_err(|message| ModelError::new(ModelErrorCode::RuntimeUnavailable, message))
}

#[cfg(feature = "model-store")]
fn select_install_routes(
    manifest: &ModelManifest,
    selection: InstallBackendSelection,
    environment: Environment,
    capability_policy: InstallCapabilityPolicy,
) -> Result<Vec<&Route>, ModelError> {
    let preference = match selection {
        InstallBackendSelection::Auto | InstallBackendSelection::All => RunBackendPreference::Auto,
        InstallBackendSelection::CoreMl => RunBackendPreference::CoreMl,
        InstallBackendSelection::Onnx => RunBackendPreference::Onnx,
    };
    let mut routes =
        install_compatible_routes(manifest, preference, environment, capability_policy);
    if routes.is_empty() {
        return Err(no_installable_route_error(
            manifest,
            preference,
            environment,
            capability_policy,
        ));
    }
    if selection != InstallBackendSelection::All {
        routes.truncate(1);
        return Ok(routes);
    }
    let mut artifacts = HashSet::new();
    routes.retain(|route| artifacts.insert(route.artifact.as_str()));
    Ok(routes)
}

#[cfg(feature = "model-store")]
fn install_compatible_routes(
    manifest: &ModelManifest,
    preference: RunBackendPreference,
    environment: Environment,
    capability_policy: InstallCapabilityPolicy,
) -> Vec<&Route> {
    let mut routes = manifest
        .routes
        .iter()
        .filter(|route| {
            install_route_compatibility(route, preference, environment).is_ok()
                && (capability_policy == InstallCapabilityPolicy::ArtifactManagement
                    || (adapter_available(
                        &manifest.contract.adapter,
                        manifest.contract.version,
                        route.backend,
                    ) && minimum_runtime_compatibility(manifest, route).is_ok()))
        })
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    routes
}

#[cfg(feature = "model-store")]
fn compatible_routes(
    manifest: &ModelManifest,
    preference: RunBackendPreference,
    environment: Environment,
    allow_unverified: bool,
) -> Vec<&Route> {
    let mut routes = manifest
        .routes
        .iter()
        .filter(|route| {
            route_compatibility(manifest, route, preference, environment, allow_unverified).is_ok()
        })
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    routes
}

#[cfg(feature = "model-store")]
fn route_compatibility(
    manifest: &ModelManifest,
    route: &Route,
    preference: RunBackendPreference,
    environment: Environment,
    allow_unverified: bool,
) -> Result<(), String> {
    route_contract_compatibility(route, preference, environment, allow_unverified)?;
    if !adapter_available(
        &manifest.contract.adapter,
        manifest.contract.version,
        route.backend,
    ) {
        return Err(format!(
            "adapter {}@{} is not compiled for backend {}",
            manifest.contract.adapter, manifest.contract.version, route.backend
        ));
    }
    minimum_runtime_compatibility(manifest, route)?;
    Ok(())
}

#[cfg(feature = "model-store")]
fn install_route_compatibility(
    route: &Route,
    preference: RunBackendPreference,
    environment: Environment,
) -> Result<(), String> {
    route_contract_compatibility(route, preference, environment, false)
}

#[cfg(feature = "model-store")]
fn route_contract_compatibility(
    route: &Route,
    preference: RunBackendPreference,
    environment: Environment,
    allow_unverified: bool,
) -> Result<(), String> {
    if !route.platforms.contains(&environment.platform)
        || !route.architectures.contains(&environment.architecture)
    {
        return Err(format!(
            "route does not support {}/{}",
            environment.platform, environment.architecture
        ));
    }
    if route.status != ValidationStatus::Verified
        && !(allow_unverified && route.status == ValidationStatus::Unverified)
    {
        return Err(format!("route status is {:?}", route.status));
    }
    let backend_matches = match preference {
        RunBackendPreference::Auto => {
            matches!(
                route.backend,
                Backend::NativeCpu | Backend::Coreml | Backend::OnnxCpu
            )
        }
        RunBackendPreference::CoreMl => route.backend == Backend::Coreml,
        RunBackendPreference::Onnx => route.backend == Backend::OnnxCpu,
    };
    if !backend_matches {
        return Err(format!("route backend {} was not requested", route.backend));
    }
    if !minimum_os_satisfied(route) {
        return Err(format!(
            "host does not satisfy minimum OS {:?}",
            route.requirements.minimum_os
        ));
    }
    Ok(())
}

#[cfg(feature = "model-store")]
fn adapter_available(adapter: &str, version: u32, backend: Backend) -> bool {
    match (adapter, version, backend) {
        #[cfg(feature = "model-birefnet-onnx")]
        ("birefnet-image-matting", 1, Backend::OnnxCpu) => true,
        #[cfg(all(feature = "model-coreml", target_os = "macos"))]
        ("birefnet-image-matting", 1, Backend::Coreml)
        | ("modnet-image-matting", 1, Backend::Coreml) => true,
        #[cfg(feature = "model-modnet-onnx")]
        ("modnet-image-matting", 1, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-qwen-native")]
        ("qwen3-asr-transcription", 1, Backend::NativeCpu)
        | ("qwen3-forced-alignment", 1, Backend::NativeCpu) => true,
        #[cfg(feature = "model-dpdfnet-onnx")]
        ("dpdfnet-streaming-enhancement", 1, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-demucs-onnx")]
        ("demucs-source-separation", 2, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-edgetam-onnx")]
        ("edgetam-video-segmentation", 1, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-lama-onnx")]
        ("lama-image-inpainting", 1, Backend::OnnxCpu) => true,
        #[cfg(all(feature = "model-omnishotcut-onnx", feature = "model-transnetv2-onnx"))]
        ("omnishotcut-shot-detection", 1, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-realesrgan-onnx")]
        ("realesrgan-super-resolution", 1, Backend::OnnxCpu) => true,
        #[cfg(feature = "model-rife-onnx")]
        ("rife-frame-interpolation", 1, Backend::OnnxCpu) => true,
        #[cfg(all(feature = "model-omnishotcut-onnx", feature = "model-transnetv2-onnx"))]
        ("transnetv2-shot-detection", 1, Backend::OnnxCpu) => true,
        _ => false,
    }
}

#[cfg(feature = "model-store")]
fn minimum_runtime_compatibility(manifest: &ModelManifest, route: &Route) -> Result<(), String> {
    let Some(requirement) = route.requirements.minimum_runtime.as_deref() else {
        return Ok(());
    };

    // These are product runtime capabilities, not claims inferred from a manifest. Keep them in
    // lockstep with the inference implementations and third-party runtimes owned by Valle.
    let (runtime_name, available) = match route.backend {
        Backend::OnnxCpu => ("ONNX Runtime", "1.28.0"),
        Backend::NativeCpu
            if matches!(
                (
                    manifest.contract.adapter.as_str(),
                    manifest.contract.version
                ),
                ("qwen3-asr-transcription", 1) | ("qwen3-forced-alignment", 1)
            ) =>
        {
            ("qwen-asr", "0.11.0")
        }
        _ => {
            return Err(format!(
                "runtime requirement {requirement:?} is not understood for backend {}",
                route.backend
            ));
        }
    };
    let required = requirement
        .strip_prefix(runtime_name)
        .and_then(|suffix| suffix.split_whitespace().next())
        .filter(|version| !version.is_empty())
        .ok_or_else(|| format!("invalid runtime requirement {requirement:?}"))?;
    if !numeric_version_at_least(available, required) {
        return Err(format!(
            "{runtime_name} {available} does not satisfy {requirement:?}"
        ));
    }
    Ok(())
}

#[cfg(feature = "model-store")]
fn numeric_version_at_least(actual: &str, required: &str) -> bool {
    fn parse(value: &str) -> Option<(u32, u32, u32)> {
        let mut fields = value.split('.');
        let major = fields.next()?.parse().ok()?;
        let minor = fields.next().unwrap_or("0").parse().ok()?;
        let patch = fields.next().unwrap_or("0").parse().ok()?;
        fields.next().is_none().then_some((major, minor, patch))
    }

    parse(actual)
        .zip(parse(required))
        .is_some_and(|(actual, required)| actual >= required)
}

#[cfg(feature = "model-store")]
fn minimum_os_satisfied(route: &Route) -> bool {
    let Some(requirement) = route.requirements.minimum_os.as_deref() else {
        return true;
    };
    #[cfg(target_os = "macos")]
    if let Some(required) = requirement.strip_prefix("macOS ") {
        let Some(required_major) = required
            .split('.')
            .next()
            .and_then(|value| value.parse().ok())
        else {
            return false;
        };
        return macos_product_major().is_some_and(|actual| actual >= required_major);
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if let Some(required) = requirement
        .split("glibc ")
        .nth(1)
        .and_then(|value| value.strip_suffix(')'))
    {
        return glibc_version().is_some_and(|actual| version_at_least(&actual, required));
    }
    false
}

#[cfg(all(feature = "model-store", target_os = "macos"))]
fn macos_product_major() -> Option<u32> {
    use std::ffi::{c_char, c_void};

    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> i32;
    }
    let name = b"kern.osproductversion\0";
    let mut length = 0_usize;
    // SAFETY: the name is NUL-terminated and `length` points to writable storage.
    if unsafe {
        sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length == 0
    {
        return None;
    }
    let mut bytes = vec![0_u8; length];
    // SAFETY: `bytes` has the size returned by the first sysctl call and remains live throughout.
    if unsafe {
        sysctlbyname(
            name.as_ptr().cast(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    bytes.truncate(length);
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .ok()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

#[cfg(all(feature = "model-store", target_os = "linux", target_env = "gnu"))]
fn glibc_version() -> Option<String> {
    use std::ffi::{CStr, c_char};
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const c_char;
    }
    // SAFETY: glibc returns a process-lifetime NUL-terminated static version string.
    let pointer = unsafe { gnu_get_libc_version() };
    (!pointer.is_null()).then(|| {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    })
}

#[cfg(all(feature = "model-store", target_os = "linux", target_env = "gnu"))]
fn version_at_least(actual: &str, required: &str) -> bool {
    let parse = |value: &str| {
        let mut fields = value
            .split('.')
            .filter_map(|field| field.parse::<u32>().ok());
        (fields.next().unwrap_or(0), fields.next().unwrap_or(0))
    };
    parse(actual) >= parse(required)
}

#[cfg(feature = "model-store")]
fn no_installable_route_error(
    manifest: &ModelManifest,
    preference: RunBackendPreference,
    environment: Environment,
    capability_policy: InstallCapabilityPolicy,
) -> ModelError {
    let has_contract_route = manifest
        .routes
        .iter()
        .any(|route| install_route_compatibility(route, preference, environment).is_ok());
    if capability_policy == InstallCapabilityPolicy::CompiledAdapters && has_contract_route {
        return ModelError::new(
            ModelErrorCode::UnsupportedAdapter,
            format!(
                "the current build has no compiled adapter for a verified {} route of {}@{} on {}/{}",
                run_backend_name(preference),
                manifest.model.id,
                manifest.model.version,
                environment.platform,
                environment.architecture
            ),
        )
        .with_hint(
            "choose a backend supported by this build, or use a Valle build containing the required media-tool adapter",
        );
    }
    ModelError::new(
        ModelErrorCode::NoCompatibleRoute,
        format!(
            "no verified {} artifact route for {}@{} on {}/{}",
            run_backend_name(preference),
            manifest.model.id,
            manifest.model.version,
            environment.platform,
            environment.architecture
        ),
    )
}

#[cfg(feature = "model-store")]
fn no_compatible_route_error(
    manifest: &ModelManifest,
    preference: RunBackendPreference,
    environment: Environment,
) -> ModelError {
    let platform_routes = manifest.routes.iter().filter(|route| {
        route.platforms.contains(&environment.platform)
            && route.architectures.contains(&environment.architecture)
            && match preference {
                RunBackendPreference::Auto => true,
                RunBackendPreference::CoreMl => route.backend == Backend::Coreml,
                RunBackendPreference::Onnx => route.backend == Backend::OnnxCpu,
            }
    });
    let code = if platform_routes.clone().any(|route| {
        !adapter_available(
            &manifest.contract.adapter,
            manifest.contract.version,
            route.backend,
        )
    }) {
        ModelErrorCode::UnsupportedAdapter
    } else {
        ModelErrorCode::NoCompatibleRoute
    };
    ModelError::new(
        code,
        format!(
            "no runnable {} route for {}@{} on {}/{}",
            run_backend_name(preference),
            manifest.model.id,
            manifest.model.version,
            environment.platform,
            environment.architecture
        ),
    )
}

#[cfg(feature = "model-store")]
fn select_verification_artifacts(
    manifest: &ModelManifest,
    request: &VerifyRequest,
) -> Result<Vec<Artifact>, ModelError> {
    if let Some(id) = request.artifact.as_deref() {
        return manifest
            .artifact(id)
            .cloned()
            .map(|artifact| vec![artifact])
            .ok_or_else(|| {
                ModelError::new(
                    ModelErrorCode::InvalidSelection,
                    format!("model {} has no artifact {id:?}", manifest.model.id),
                )
            });
    }
    let mut ids = HashSet::new();
    if let Some(preference) = request.backend {
        for route in &manifest.routes {
            let matches = match preference {
                RunBackendPreference::Auto => {
                    matches!(
                        route.backend,
                        Backend::NativeCpu | Backend::Coreml | Backend::OnnxCpu
                    )
                }
                RunBackendPreference::CoreMl => route.backend == Backend::Coreml,
                RunBackendPreference::Onnx => route.backend == Backend::OnnxCpu,
            };
            if matches {
                ids.insert(route.artifact.as_str());
            }
        }
    } else {
        ids.extend(
            manifest
                .artifacts
                .iter()
                .map(|artifact| artifact.id.as_str()),
        );
    }
    let artifacts = manifest
        .artifacts
        .iter()
        .filter(|artifact| ids.contains(artifact.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if artifacts.is_empty() {
        return Err(ModelError::new(
            ModelErrorCode::InvalidSelection,
            format!(
                "no artifacts match the requested verification filter for {}",
                manifest.model.id
            ),
        ));
    }
    Ok(artifacts)
}

#[cfg(feature = "model-store")]
fn artifact_key(bundle: &ReleaseBundle, artifact: &str) -> ArtifactKey {
    ArtifactKey {
        model: bundle.manifest.model.id.clone(),
        version: bundle.manifest.model.version.clone(),
        revision: bundle.release.revision.clone(),
        artifact: artifact.to_owned(),
    }
}

#[cfg(feature = "model-store")]
fn resolved_artifact(
    bundle: &ReleaseBundle,
    route: &Route,
    artifact: &Artifact,
    root: PathBuf,
    entrypoint: PathBuf,
) -> ResolvedArtifact {
    ResolvedArtifact {
        id: bundle.manifest.model.id.clone(),
        version: bundle.manifest.model.version.clone(),
        revision: bundle.release.revision.clone(),
        manifest_path: bundle.release.manifest_path.clone(),
        adapter: bundle.manifest.contract.adapter.clone(),
        adapter_version: bundle.manifest.contract.version,
        artifact: artifact.id.clone(),
        route: route.id.clone(),
        backend: route.backend.to_string(),
        precision: artifact.precision.clone(),
        root,
        entrypoint,
    }
}

#[cfg(feature = "model-store")]
fn installation_state(state: InstallationState) -> InstalledArtifactState {
    match state {
        InstallationState::Missing => InstalledArtifactState::Missing,
        InstallationState::Ready => InstalledArtifactState::Ready,
        InstallationState::Corrupt => InstalledArtifactState::Corrupt,
    }
}

#[cfg(feature = "model-store")]
fn run_backend_name(backend: RunBackendPreference) -> &'static str {
    match backend {
        RunBackendPreference::Auto => "auto",
        RunBackendPreference::CoreMl => "coreml",
        RunBackendPreference::Onnx => "onnx",
    }
}

#[cfg(feature = "model-store")]
fn install_hint_for_route(model: &str, version: &str, route: &Route) -> Option<String> {
    let backend = match route.backend {
        Backend::Coreml => "coreml",
        Backend::OnnxCpu => "onnx",
        // Native routes are intentionally not part of the public backend selector.
        _ => return None,
    };
    Some(format!(
        "run: valle models install {model} --version {version} --backend {backend}"
    ))
}

#[cfg(feature = "model-store")]
fn store_error(action: &str, error: anyhow::Error) -> ModelError {
    let message = format!("{action}: {error:#}");
    let code = if message.contains("401") || message.contains("403") {
        ModelErrorCode::AuthRequired
    } else if message.contains("sha256")
        || message.contains("checksum")
        || message.contains("integrity")
        || message.contains("corrupt")
        || message.contains("failed verification")
        || message.contains("does not match")
    {
        ModelErrorCode::IntegrityFailed
    } else {
        ModelErrorCode::Internal
    };
    ModelError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_root_is_retained() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-model-manager-missing-{}",
            std::process::id()
        ));
        let manager = ModelManager::from_models_root(&root);
        assert_eq!(manager.models_root(), root);
    }

    #[cfg(feature = "model-store")]
    #[test]
    fn explicit_latest_never_reaches_offline_resolution() {
        let root = tempfile::tempdir().unwrap();
        let manager = ModelManager::from_models_root(root.path());
        let error = manager
            .resolve_installed(ResolveRequest {
                selection: super::super::ModelSelection {
                    id: "birefnet".to_owned(),
                    version: Some("latest".to_owned()),
                    backend: RunBackendPreference::Auto,
                },
                allow_unverified: false,
            })
            .unwrap_err();
        assert_eq!(error.code, ModelErrorCode::InvalidSelection);
    }

    #[cfg(feature = "model-store")]
    #[test]
    fn list_uses_only_the_embedded_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let manager = ModelManager::with_catalog_source(
            root.path(),
            "https://invalid.example/never-contact-this-source.json",
        );
        let models = manager.list().unwrap();
        assert!(models.iter().any(|model| model.id == "birefnet"));
        assert!(models.iter().any(|model| model.id == "demucs"));
    }

    #[cfg(feature = "model-store")]
    fn route_without_compiled_adapter() -> (ModelManifest, Environment) {
        let environment = current_environment().unwrap();
        let mut manifest = crate::models::spec::embedded_release_manifest("birefnet", "1.0.0")
            .expect("embedded BiRefNet release");
        let mut route = manifest
            .routes
            .iter()
            .find(|route| route.backend == Backend::OnnxCpu)
            .expect("BiRefNet has an ONNX CPU route")
            .clone();
        route.id = "verified-onnx-current-host".to_owned();
        route.platforms = vec![environment.platform];
        route.architectures = vec![environment.architecture];
        route.status = ValidationStatus::Verified;
        route.requirements.minimum_os = None;
        route.requirements.minimum_runtime = Some("ONNX Runtime 1.28.0".to_owned());
        manifest.routes = vec![route];
        manifest.contract.adapter = "test-adapter-not-compiled".to_owned();
        (manifest, environment)
    }

    #[cfg(feature = "model-store")]
    #[test]
    fn model_store_only_install_selection_does_not_require_an_adapter() {
        let (manifest, environment) = route_without_compiled_adapter();
        assert!(!adapter_available(
            &manifest.contract.adapter,
            manifest.contract.version,
            Backend::OnnxCpu
        ));

        let routes = select_install_routes(
            &manifest,
            InstallBackendSelection::Onnx,
            environment,
            InstallCapabilityPolicy::ArtifactManagement,
        )
        .expect("a verified artifact route is installable without an inference adapter");

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].id, "verified-onnx-current-host");
        assert_eq!(
            routes[0].requirements.minimum_runtime.as_deref(),
            Some("ONNX Runtime 1.28.0")
        );
    }

    #[cfg(feature = "model-store")]
    #[test]
    fn runtime_requirement_parser_is_strict_and_version_aware() {
        let (manifest, _) = route_without_compiled_adapter();
        let mut route = manifest.routes[0].clone();

        route.requirements.minimum_runtime = Some("ONNX Runtime 1.28.0 shared library".to_owned());
        assert!(minimum_runtime_compatibility(&manifest, &route).is_ok());

        route.requirements.minimum_runtime = Some("ONNX Runtime 99.0.0".to_owned());
        assert!(minimum_runtime_compatibility(&manifest, &route).is_err());

        route.requirements.minimum_runtime = Some("some runtime eventually".to_owned());
        assert!(minimum_runtime_compatibility(&manifest, &route).is_err());
    }

    #[cfg(feature = "model-birefnet-onnx")]
    #[test]
    fn runtime_requirement_participates_in_product_route_selection() {
        let (mut manifest, environment) = route_without_compiled_adapter();
        manifest.contract.adapter = "birefnet-image-matting".to_owned();
        manifest.routes[0].requirements.minimum_runtime = Some("ONNX Runtime 99.0.0".to_owned());

        let compatibility = route_compatibility(
            &manifest,
            &manifest.routes[0],
            RunBackendPreference::Onnx,
            environment,
            false,
        );
        assert!(compatibility.unwrap_err().contains("does not satisfy"));
        assert!(
            install_compatible_routes(
                &manifest,
                RunBackendPreference::Onnx,
                environment,
                InstallCapabilityPolicy::CompiledAdapters,
            )
            .is_empty()
        );
        assert_eq!(
            install_compatible_routes(
                &manifest,
                RunBackendPreference::Onnx,
                environment,
                InstallCapabilityPolicy::ArtifactManagement,
            )
            .len(),
            1,
            "a store-only build may still manage an artifact for another compatible product"
        );
    }

    #[cfg(feature = "model-demucs-onnx")]
    #[test]
    fn product_install_auto_skips_a_preferred_backend_without_a_compiled_adapter() {
        let environment = current_environment().unwrap();
        let mut manifest = crate::models::spec::embedded_release_manifest("demucs", "1.0.0")
            .expect("embedded Demucs release");
        for route in &mut manifest.routes {
            route.platforms = vec![environment.platform];
            route.architectures = vec![environment.architecture];
            route.status = ValidationStatus::Verified;
            route.requirements.minimum_os = None;
            route.priority = if route.backend == Backend::Coreml {
                100
            } else {
                10
            };
        }

        let routes = select_install_routes(
            &manifest,
            InstallBackendSelection::Auto,
            environment,
            InstallCapabilityPolicy::CompiledAdapters,
        )
        .expect("the compiled ONNX adapter is a valid product fallback");

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].backend, Backend::OnnxCpu);
    }

    #[cfg(feature = "model-demucs-onnx")]
    #[test]
    fn explicit_install_backend_reports_a_missing_compiled_adapter() {
        let environment = current_environment().unwrap();
        let mut manifest = crate::models::spec::embedded_release_manifest("demucs", "1.0.0")
            .expect("embedded Demucs release");
        for route in &mut manifest.routes {
            route.platforms = vec![environment.platform];
            route.architectures = vec![environment.architecture];
            route.status = ValidationStatus::Verified;
            route.requirements.minimum_os = None;
        }

        let error = select_install_routes(
            &manifest,
            InstallBackendSelection::CoreMl,
            environment,
            InstallCapabilityPolicy::CompiledAdapters,
        )
        .unwrap_err();

        assert_eq!(error.code, ModelErrorCode::UnsupportedAdapter);
        assert!(error.message.contains("no compiled adapter"));
    }

    #[cfg(feature = "model-store")]
    #[test]
    fn runtime_route_resolution_remains_unavailable_without_an_adapter() {
        let (manifest, environment) = route_without_compiled_adapter();

        assert!(
            compatible_routes(&manifest, RunBackendPreference::Onnx, environment, false,)
                .is_empty()
        );
        assert_eq!(
            no_compatible_route_error(&manifest, RunBackendPreference::Onnx, environment).code,
            ModelErrorCode::UnsupportedAdapter
        );
    }
}
