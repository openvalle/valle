//! Explicit model discovery, installation, and integrity verification.
//!
//! `install` is the only command in this module allowed to contact the network. Listing and
//! verification use the embedded catalog and the local immutable artifact store only.

use std::process::ExitCode;

use anyhow::Result;

use valle_media::models::{
    InstallBackendSelection, InstallCapabilityPolicy, InstallRequest, InstalledArtifactState,
    ModelManager, RunBackendPreference, VerifyRequest,
};
use valle_media::tools::ToolError;

pub(crate) fn run_list(json: bool) -> Result<ExitCode> {
    let manager = ModelManager::new();
    match manager.list() {
        Ok(statuses) => {
            if json {
                crate::output::emit(serde_json::to_value(&statuses)?);
            } else {
                for model in &statuses {
                    eprintln!(
                        "{} ({}) — default {}",
                        model.id, model.display_name, model.default_version
                    );
                    for artifact in &model.artifacts {
                        let state = artifact_state(artifact.state);
                        let compatibility = if artifact.compatible {
                            "compatible".to_owned()
                        } else {
                            format!(
                                "unavailable: {}",
                                artifact
                                    .incompatibility
                                    .as_deref()
                                    .unwrap_or("route is not runnable on this host")
                            )
                        };
                        eprintln!(
                            "  {:<10} {:<9} {:<10} {} ({})",
                            artifact.backend,
                            state,
                            if artifact.route_verified {
                                "verified"
                            } else {
                                "unverified"
                            },
                            artifact.id,
                            compatibility
                        );
                    }
                }
                eprintln!(
                    "\ninstall: valle models install <id>   (cache: {})",
                    manager.models_root().display()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => super::media::render_error(&ToolError::from(error), json),
    }
}

pub(crate) fn run_install(
    id: String,
    version: Option<String>,
    backend: InstallBackendSelection,
    refresh_catalog: bool,
    json: bool,
) -> Result<ExitCode> {
    let manager = product_install_manager();
    let mut progress = |message: &str| {
        if !json || crate::output::events() {
            crate::events::emit(crate::events::EventKind::ModelsProgress {
                message: message.to_owned(),
            });
        }
    };
    match manager.install(
        InstallRequest {
            id,
            version,
            backend,
            refresh_catalog,
        },
        &mut progress,
    ) {
        Ok(result) => {
            if json {
                crate::output::emit(serde_json::to_value(&result)?);
            } else {
                for artifact in &result.artifacts {
                    eprintln!(
                        "installed {}@{} {} ({}) -> {}",
                        artifact.id,
                        artifact.version,
                        artifact.artifact,
                        artifact.backend,
                        artifact.root.display()
                    );
                }
                eprintln!(
                    "downloaded {} file(s), reused {} file(s)",
                    result.downloaded_files, result.reused_files
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => super::media::render_error(&ToolError::from(error), json),
    }
}

fn product_install_manager() -> ModelManager {
    ModelManager::new().with_install_capability_policy(InstallCapabilityPolicy::CompiledAdapters)
}

pub(crate) fn run_verify(
    id: String,
    version: Option<String>,
    backend: Option<RunBackendPreference>,
    artifact: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    let manager = ModelManager::new();
    match manager.verify(VerifyRequest {
        id,
        version,
        backend,
        artifact,
    }) {
        Ok(report) => {
            if json {
                crate::output::emit(serde_json::to_value(&report)?);
            } else {
                eprintln!("{}@{} ({})", report.id, report.version, report.revision);
                for artifact in &report.artifacts {
                    eprintln!(
                        "  {:<9} {} — {} file(s), {} byte(s){}",
                        artifact_state(artifact.state),
                        artifact.artifact,
                        artifact.files,
                        artifact.bytes,
                        artifact
                            .error
                            .as_deref()
                            .map(|error| format!("; {error}"))
                            .unwrap_or_default()
                    );
                }
            }
            let failed = report
                .artifacts
                .iter()
                .any(|artifact| artifact.state != InstalledArtifactState::Ready);
            Ok(if failed {
                ExitCode::from(3)
            } else {
                ExitCode::SUCCESS
            })
        }
        Err(error) => super::media::render_error(&ToolError::from(error), json),
    }
}

fn artifact_state(state: InstalledArtifactState) -> &'static str {
    match state {
        InstalledArtifactState::Missing => "missing",
        InstalledArtifactState::Ready => "ready",
        InstalledArtifactState::Corrupt => "corrupt",
    }
}
