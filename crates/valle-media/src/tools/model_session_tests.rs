use std::path::Path;

use super::*;
use crate::models::{
    ResolvedArtifact,
    spec::{Backend, embedded_release_manifest},
};

/// A resolved metadata fixture for workflow tests that stop before loading real weights.
/// Production resolution and artifact verification are not bypassed outside unit tests.
pub(crate) fn candidates(id: &str, root: &Path) -> ModelSessionCandidates {
    let manifest = embedded_release_manifest(id, "1.0.0").unwrap();
    let route = manifest
        .routes
        .iter()
        .find(|r| r.backend == Backend::OnnxCpu)
        .unwrap()
        .clone();
    let artifact = manifest
        .artifacts
        .iter()
        .find(|a| a.id == route.artifact)
        .unwrap()
        .clone();
    let resolved = ResolvedArtifact {
        id: id.into(),
        version: manifest.model.version.clone(),
        revision: "0000000000000000000000000000000000000000".into(),
        manifest_path: "release-fixture.json".into(),
        adapter: manifest.contract.adapter.clone(),
        adapter_version: manifest.contract.version,
        artifact: artifact.id.clone(),
        route: route.id.clone(),
        backend: route.backend.to_string(),
        precision: artifact.precision.clone(),
        root: root.into(),
        entrypoint: root.join(&artifact.entrypoint),
    };
    ModelSessionCandidates {
        preference: RunBackendPreference::Onnx,
        candidates: vec![ResolvedModel {
            manifest,
            route,
            artifact,
            resolved,
        }],
        fallback_install_hint: None,
    }
}
