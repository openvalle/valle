//! Discovery of the former Phase-0 Qwen cache for one-time installation migration.
//!
//! This module is deliberately not a model registry or a runtime resolver. Exact release
//! identity and file integrity come from the formal catalog manifest. The paths below only let
//! `ModelManager::install` locate old bytes so the model store can verify and import them without
//! downloading the multi-gigabyte weights again.

#[cfg(feature = "model-qwen-native")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "model-qwen-native")]
const MIGRATABLE_QWEN_MODELS: &[&str] = &["qwen3-asr-0.6b", "qwen3-aligner-0.6b"];

/// Root used by Valle before the platform-native model store was introduced.
pub(super) fn models_root() -> PathBuf {
    if let Some(root) = std::env::var_os("VALLE_LEGACY_MODEL_CACHE") {
        return PathBuf::from(root);
    }
    if let Some(root) = std::env::var_os("VALLE_CACHE_DIR") {
        return PathBuf::from(root).join("models");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home)
            .join(".cache")
            .join("valle")
            .join("models");
    }
    PathBuf::from(".valle").join("models")
}

/// Return an existing old-style directory only for a known Qwen model.
#[cfg(feature = "model-qwen-native")]
pub(super) fn migration_source(root: &Path, model_id: &str) -> Option<PathBuf> {
    if !MIGRATABLE_QWEN_MODELS.contains(&model_id) {
        return None;
    }
    let directory = root.join(model_id);
    directory.is_dir().then_some(directory)
}

#[cfg(all(test, feature = "model-qwen-native"))]
mod tests {
    use super::*;

    #[test]
    fn only_existing_qwen_directories_are_migration_sources() {
        let root = tempfile::tempdir().unwrap();
        assert!(migration_source(root.path(), "qwen3-asr-0.6b").is_none());
        std::fs::create_dir(root.path().join("qwen3-asr-0.6b")).unwrap();
        assert_eq!(
            migration_source(root.path(), "qwen3-asr-0.6b"),
            Some(root.path().join("qwen3-asr-0.6b"))
        );
        assert!(migration_source(root.path(), "birefnet").is_none());
    }
}
