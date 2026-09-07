//! Shared route-selection and session-load boundary for model-powered tools.
//!
//! Resolution is offline and yields only installed, verified, host-compatible routes. `auto`
//! may reject a route while opening the first session and try the next installed candidate. An
//! explicit backend gets exactly one attempt. Once this function returns, the caller owns one
//! pinned session and must never switch routes during frame/chunk processing.

use anyhow::Error;

#[cfg(feature = "tool-transcribe")]
use crate::models::ResolvedModelCandidates;
#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-matte",
    feature = "tool-segment",
    feature = "tool-separate",
    feature = "tool-shots",
    feature = "tool-upscale"
))]
use crate::models::{ModelManager, ModelSelection, ResolveRequest};
use crate::models::{ResolvedModel, RunBackendPreference};

use super::{ToolError, ToolErrorCode, ToolWarning};

pub(crate) struct ModelSessionCandidates {
    preference: RunBackendPreference,
    candidates: Vec<ResolvedModel>,
    fallback_install_hint: Option<String>,
}

pub(crate) struct OpenedModelSession<T> {
    pub(crate) session: T,
    pub(crate) model: ResolvedModel,
    pub(crate) warnings: Vec<ToolWarning>,
}

impl ModelSessionCandidates {
    #[cfg(any(
        feature = "tool-enhance",
        feature = "tool-inpaint",
        feature = "tool-interpolate",
        feature = "tool-matte",
        feature = "tool-segment",
        feature = "tool-separate",
        feature = "tool-shots",
        feature = "tool-upscale"
    ))]
    pub(crate) fn resolve(
        manager: &ModelManager,
        selection: ModelSelection,
    ) -> Result<Self, ToolError> {
        let preference = selection.backend;
        let candidates = manager.resolve_model_candidates(ResolveRequest {
            selection,
            allow_unverified: false,
        })?;
        Ok(Self {
            preference,
            candidates: candidates.models,
            fallback_install_hint: candidates.fallback_install_hint,
        })
    }

    #[cfg(feature = "tool-transcribe")]
    pub(crate) fn from_resolved(
        preference: RunBackendPreference,
        candidates: ResolvedModelCandidates,
    ) -> Self {
        debug_assert!(!candidates.models.is_empty());
        Self {
            preference,
            candidates: candidates.models,
            fallback_install_hint: candidates.fallback_install_hint,
        }
    }

    /// The preferred resolved route, for a no-op job that never needs to open a session.
    #[cfg(feature = "tool-inpaint")]
    pub(crate) fn preferred_model(&self) -> &ResolvedModel {
        self.candidates
            .first()
            .expect("model candidate resolution never returns an empty list")
    }

    pub(crate) fn open<T>(
        self,
        action: &str,
        mut open: impl FnMut(&ResolvedModel) -> Result<T, Error>,
    ) -> Result<OpenedModelSession<T>, ToolError> {
        let (session, model, warnings) = try_open_candidates(
            self.preference,
            action,
            self.candidates,
            self.fallback_install_hint,
            |model| CandidateIdentity {
                route: model.resolved.route.clone(),
                backend: model.resolved.backend.clone(),
            },
            |model| open(model),
        )?;
        Ok(OpenedModelSession {
            session,
            model,
            warnings,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateIdentity {
    route: String,
    backend: String,
}

struct RejectedCandidate {
    identity: CandidateIdentity,
    reason: String,
    code: ToolErrorCode,
}

fn try_open_candidates<C, T>(
    preference: RunBackendPreference,
    action: &str,
    candidates: Vec<C>,
    fallback_install_hint: Option<String>,
    describe: impl Fn(&C) -> CandidateIdentity,
    mut open: impl FnMut(&C) -> Result<T, Error>,
) -> Result<(T, C, Vec<ToolWarning>), ToolError> {
    debug_assert!(!candidates.is_empty());
    let allow_fallback = preference == RunBackendPreference::Auto;
    let mut rejected = Vec::new();

    for candidate in candidates {
        let identity = describe(&candidate);
        match open(&candidate) {
            Ok(session) => {
                let warnings = rejected
                    .into_iter()
                    .map(|rejection: RejectedCandidate| ToolWarning {
                        code: "model_route_rejected".to_owned(),
                        message: format!(
                            "model route {} ({}) was rejected during session load: {}",
                            rejection.identity.route, rejection.identity.backend, rejection.reason
                        ),
                    })
                    .collect();
                return Ok((session, candidate, warnings));
            }
            Err(error) if !allow_fallback => return Err(ToolError::model_load(action, error)),
            Err(error) => {
                let reason = single_line_reason(&error);
                let code = ToolError::model_load(action, error).code;
                rejected.push(RejectedCandidate {
                    identity,
                    reason,
                    code,
                });
            }
        }
    }

    let code = if rejected
        .iter()
        .all(|rejection| rejection.code == ToolErrorCode::RuntimeUnavailable)
    {
        ToolErrorCode::RuntimeUnavailable
    } else {
        ToolErrorCode::ModelLoadFailed
    };
    let routes = rejected
        .iter()
        .map(|rejection| {
            format!(
                "{} ({}): {}",
                rejection.identity.route, rejection.identity.backend, rejection.reason
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let error = ToolError::new(
        code,
        format!("{action}: every installed verified route failed to load: {routes}"),
    )
    .with_context("attemptedRoutes", rejected.len().to_string());
    Err(if let Some(hint) = fallback_install_hint {
        error.with_hint(hint)
    } else {
        error
    })
}

fn single_line_reason(error: &Error) -> String {
    format!("{error:#}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(candidate: &&str) -> CandidateIdentity {
        CandidateIdentity {
            route: (*candidate).to_owned(),
            backend: if candidate.contains("coreml") {
                "coreml"
            } else {
                "onnx-cpu"
            }
            .to_owned(),
        }
    }

    #[test]
    fn auto_rejects_a_failed_route_and_pins_the_first_session_that_opens() {
        let mut attempted = Vec::new();
        let (session, selected, warnings) = try_open_candidates(
            RunBackendPreference::Auto,
            "load mock model",
            vec!["coreml-primary", "onnx-fallback", "onnx-unused"],
            None,
            identity,
            |candidate| {
                attempted.push(*candidate);
                if *candidate == "coreml-primary" {
                    Err(anyhow::anyhow!("compile failed\nwith details"))
                } else {
                    Ok(format!("session:{candidate}"))
                }
            },
        )
        .unwrap();

        assert_eq!(session, "session:onnx-fallback");
        assert_eq!(selected, "onnx-fallback");
        assert_eq!(attempted, vec!["coreml-primary", "onnx-fallback"]);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "model_route_rejected");
        assert!(warnings[0].message.contains("coreml-primary (coreml)"));
        assert!(!warnings[0].message.contains('\n'));
    }

    #[test]
    fn explicit_backend_never_falls_back_after_a_session_load_failure() {
        let mut attempted = Vec::new();
        let error = try_open_candidates(
            RunBackendPreference::Onnx,
            "load mock model",
            vec!["onnx-primary", "onnx-secondary"],
            None,
            identity,
            |candidate| -> Result<(), Error> {
                attempted.push(*candidate);
                Err(anyhow::anyhow!("invalid graph"))
            },
        )
        .unwrap_err();

        assert_eq!(attempted, vec!["onnx-primary"]);
        assert_eq!(error.code, ToolErrorCode::ModelLoadFailed);
    }

    #[test]
    fn auto_reports_every_failed_candidate_with_a_stable_error_class() {
        let error = try_open_candidates(
            RunBackendPreference::Auto,
            "load mock model",
            vec!["coreml-primary", "onnx-fallback"],
            Some("run: valle models install demo --version 1.0.0 --backend onnx".to_owned()),
            identity,
            |_| -> Result<(), Error> { Err(anyhow::anyhow!("invalid graph")) },
        )
        .unwrap_err();

        assert_eq!(error.code, ToolErrorCode::ModelLoadFailed);
        assert_eq!(error.context["attemptedRoutes"], "2");
        assert!(error.message.contains("coreml-primary (coreml)"));
        assert!(error.message.contains("onnx-fallback (onnx-cpu)"));
        assert_eq!(
            error.hint.as_deref(),
            Some("run: valle models install demo --version 1.0.0 --backend onnx")
        );
    }
}
