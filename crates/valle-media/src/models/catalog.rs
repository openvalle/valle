use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::models::spec::{
    Catalog, CatalogModel, CatalogRelease, EMBEDDED_RELEASES, ModelManifest, embedded_catalog,
    embedded_release_manifest,
};
use crate::models::store::{FetchedManifest, HubClient};
use sha2::{Digest, Sha256};

use super::{ModelError, ModelErrorCode};

const DEFAULT_CATALOG_SOURCE: &str =
    "https://huggingface.co/openvalle/valle-models/resolve/main/catalog.v1.json";
static CACHE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct ReleaseBundle {
    pub release: CatalogRelease,
    pub manifest: ModelManifest,
    pub manifest_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct CatalogRepository {
    models_root: PathBuf,
    source: String,
}

impl CatalogRepository {
    pub fn new(models_root: PathBuf) -> Self {
        let source = std::env::var("VALLE_MODEL_CATALOG")
            .ok()
            .filter(|source| !source.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CATALOG_SOURCE.to_owned());
        Self {
            models_root,
            source,
        }
    }

    pub fn with_source(models_root: PathBuf, source: impl Into<String>) -> Self {
        Self {
            models_root,
            source: source.into(),
        }
    }

    pub fn embedded(&self) -> Catalog {
        embedded_catalog()
    }

    /// Resolve without HTTP. `None` always means the version pinned by the embedded snapshot.
    pub fn offline_release(
        &self,
        model_id: &str,
        version: Option<&str>,
    ) -> Result<ReleaseBundle, ModelError> {
        match version {
            None => self.embedded_release(model_id, None),
            Some("latest") => Err(ModelError::new(
                ModelErrorCode::InvalidSelection,
                "latest is an install-time selector, not a reproducible runtime version",
            )
            .with_hint("install with --version latest --refresh-catalog, then run the resolved concrete version")),
            Some(version) => self
                .embedded_release(model_id, Some(version))
                .or_else(|error| {
                    if error.code == ModelErrorCode::UnknownModel
                        || error.code == ModelErrorCode::InvalidSelection
                    {
                        self.cached_release(model_id, version)
                    } else {
                        Err(error)
                    }
                }),
        }
    }

    /// Resolve an install request. Network access is confined to this explicit operation.
    pub fn install_release(
        &self,
        model_id: &str,
        version: Option<&str>,
        refresh: bool,
        hub: &HubClient,
    ) -> Result<ReleaseBundle, ModelError> {
        // Official Qwen repositories publish weights, not Valle release manifests. Their
        // supported revisions are owned by this binary and cannot be replaced by a demo catalog.
        if super::spec::uses_local_manifest(model_id) {
            if version == Some("latest") && !refresh {
                return Err(ModelError::new(
                    ModelErrorCode::InvalidSelection,
                    "--version latest requires --refresh-catalog",
                ));
            }
            return self.embedded_release(model_id, version.filter(|version| *version != "latest"));
        }
        let refreshed = if refresh {
            let catalog = hub
                .load_catalog(&self.source)
                .map_err(|error| catalog_error("refresh model catalog", error))?;
            self.cache_catalog(&catalog)?;
            Some(catalog)
        } else {
            None
        };

        match version {
            None => self.embedded_release(model_id, None),
            Some("latest") => {
                let catalog = refreshed.as_ref().ok_or_else(|| {
                    ModelError::new(
                        ModelErrorCode::InvalidSelection,
                        "--version latest requires --refresh-catalog",
                    )
                })?;
                self.fetch_catalog_release(catalog, model_id, None, hub)
            }
            Some(version) => {
                if let Some(catalog) = refreshed.as_ref() {
                    self.fetch_catalog_release(catalog, model_id, Some(version), hub)
                } else if let Ok(release) = self.embedded_release(model_id, Some(version)) {
                    Ok(release)
                } else {
                    Err(ModelError::new(
                        ModelErrorCode::InvalidSelection,
                        format!(
                            "model {model_id:?}@{version:?} is not in the embedded catalog; installing a concrete external release requires --refresh-catalog"
                        ),
                    )
                    .with_hint(format!(
                        "run: valle models install {model_id} --version {version} --refresh-catalog"
                    )))
                }
            }
        }
    }

    fn embedded_release(
        &self,
        model_id: &str,
        version: Option<&str>,
    ) -> Result<ReleaseBundle, ModelError> {
        let catalog = embedded_catalog();
        let model = catalog.model(model_id).ok_or_else(|| {
            ModelError::new(
                ModelErrorCode::UnknownModel,
                format!("embedded model catalog has no model {model_id:?}"),
            )
        })?;
        let release = model
            .release(version)
            .map_err(|message| ModelError::new(ModelErrorCode::InvalidSelection, message))?;
        let manifest = embedded_release_manifest(model_id, &release.version)
            .map_err(|message| ModelError::new(ModelErrorCode::CatalogUnavailable, message))?;
        let snapshot = EMBEDDED_RELEASES
            .iter()
            .find(|snapshot| snapshot.model == model_id && snapshot.version == release.version)
            .ok_or_else(|| {
                ModelError::new(
                    ModelErrorCode::CatalogUnavailable,
                    format!(
                        "embedded catalog release {model_id}@{} has no embedded manifest",
                        release.version
                    ),
                )
            })?;
        Ok(ReleaseBundle {
            release: release.clone(),
            manifest,
            manifest_bytes: snapshot.json.as_bytes().to_vec(),
        })
    }

    fn fetch_catalog_release(
        &self,
        catalog: &Catalog,
        model_id: &str,
        version: Option<&str>,
        hub: &HubClient,
    ) -> Result<ReleaseBundle, ModelError> {
        let model = catalog.model(model_id).cloned().ok_or_else(|| {
            ModelError::new(
                ModelErrorCode::UnknownModel,
                format!("refreshed model catalog has no model {model_id:?}"),
            )
        })?;
        let release = model
            .release(version)
            .map_err(|message| ModelError::new(ModelErrorCode::InvalidSelection, message))?
            .clone();
        self.fetch_release_bundle(model, release, hub)
    }

    fn fetch_release_bundle(
        &self,
        model: CatalogModel,
        release: CatalogRelease,
        hub: &HubClient,
    ) -> Result<ReleaseBundle, ModelError> {
        let fetched = hub
            .fetch_manifest(&release)
            .map_err(|error| catalog_error("fetch release manifest", error))?;
        validate_identity(&model, &release, &fetched.manifest)?;
        self.cache_release(&release, &fetched)?;
        Ok(ReleaseBundle {
            release,
            manifest: fetched.manifest,
            manifest_bytes: fetched.bytes,
        })
    }

    fn cached_release(&self, model_id: &str, version: &str) -> Result<ReleaseBundle, ModelError> {
        let (model, release) = self.cached_catalog_release(model_id, version)?;
        let path = self.release_cache_path(model_id, version, &release.revision);
        let bytes = std::fs::read(&path).map_err(|error| {
            ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!(
                    "cached release manifest {} is unavailable: {error}",
                    path.display()
                ),
            )
            .with_hint(format!(
                "run: valle models install {model_id} --version {version} --refresh-catalog"
            ))
        })?;
        let manifest: ModelManifest = serde_json::from_slice(&bytes).map_err(|error| {
            ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!(
                    "cached release manifest {} is invalid JSON: {error}",
                    path.display()
                ),
            )
        })?;
        manifest.validate_publishable().map_err(|error| {
            ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!(
                    "cached release manifest {} is invalid: {error}",
                    path.display()
                ),
            )
        })?;
        validate_identity(&model, &release, &manifest)?;
        Ok(ReleaseBundle {
            release,
            manifest,
            manifest_bytes: bytes,
        })
    }

    fn cached_catalog_release(
        &self,
        model_id: &str,
        version: &str,
    ) -> Result<(CatalogModel, CatalogRelease), ModelError> {
        let directory = self.catalog_cache_dir();
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            ModelError::new(
                ModelErrorCode::InvalidSelection,
                format!("model {model_id:?}@{version:?} is not in the embedded catalog: {error}"),
            )
            .with_hint(format!(
                "run: valle models install {model_id} --version {version} --refresh-catalog"
            ))
        })?;
        let mut matches: Vec<(CatalogModel, CatalogRelease)> = Vec::new();
        for path in entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
        {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(catalog) = serde_json::from_slice::<Catalog>(&bytes) else {
                continue;
            };
            if catalog.validate().is_err() {
                continue;
            }
            let Some(model) = catalog.model(model_id) else {
                continue;
            };
            let Ok(release) = model.release(Some(version)) else {
                continue;
            };
            matches.push((model.clone(), release.clone()));
        }
        let Some(first) = matches.first().cloned() else {
            return Err(ModelError::new(
                ModelErrorCode::InvalidSelection,
                format!("no cached catalog contains {model_id}@{version}"),
            )
            .with_hint(format!(
                "run: valle models install {model_id} --version {version} --refresh-catalog"
            )));
        };
        if matches.iter().any(|(_, release)| release != &first.1) {
            return Err(ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!("cached catalogs disagree about {model_id}@{version}"),
            ));
        }
        Ok(first)
    }

    fn cache_catalog(&self, catalog: &Catalog) -> Result<(), ModelError> {
        catalog.validate().map_err(|error| {
            ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!("refusing invalid refreshed catalog: {error}"),
            )
        })?;
        let bytes = serde_json::to_vec_pretty(catalog).map_err(|error| {
            ModelError::new(
                ModelErrorCode::Internal,
                format!("serialize refreshed model catalog: {error}"),
            )
        })?;
        let digest = hex::encode(Sha256::digest(&bytes));
        write_immutable(
            &self.catalog_cache_dir().join(format!("{digest}.json")),
            &bytes,
        )
    }

    fn cache_release(
        &self,
        release: &CatalogRelease,
        fetched: &FetchedManifest,
    ) -> Result<(), ModelError> {
        write_immutable(
            &self.release_cache_path(
                &fetched.manifest.model.id,
                &fetched.manifest.model.version,
                &release.revision,
            ),
            &fetched.bytes,
        )
    }

    fn catalog_cache_dir(&self) -> PathBuf {
        self.models_root.join(".metadata").join("catalogs")
    }

    fn release_cache_path(&self, model: &str, version: &str, revision: &str) -> PathBuf {
        self.models_root
            .join(".metadata")
            .join("releases")
            .join(model)
            .join(version)
            .join(revision)
            .join("release.v1.json")
    }
}

#[cfg(test)]
mod direct_upstream_tests {
    use super::*;

    #[test]
    fn qwen_resolves_without_fetching_a_demo_manifest_even_on_refresh() {
        let root = tempfile::tempdir().unwrap();
        let catalog =
            CatalogRepository::with_source(root.path().into(), "http://127.0.0.1:1/missing");
        let hub = HubClient::new("http://127.0.0.1:1", None).unwrap();
        for id in ["qwen3-asr-0.6b", "qwen3-aligner-0.6b"] {
            let pinned = catalog.offline_release(id, None).unwrap();
            assert!(pinned.release.repository.starts_with("Qwen/"));
            for version in [None, Some("1.0.0"), Some("latest")] {
                let bundle = catalog.install_release(id, version, true, &hub).unwrap();
                assert_eq!(bundle.release, pinned.release);
            }
            assert!(
                catalog
                    .install_release(id, Some("latest"), false, &hub)
                    .is_err()
            );
            assert!(
                catalog
                    .install_release(id, Some("99.0.0"), true, &hub)
                    .is_err()
            );
        }
    }
}

fn validate_identity(
    model: &CatalogModel,
    release: &CatalogRelease,
    manifest: &ModelManifest,
) -> Result<(), ModelError> {
    if manifest.model.id != model.id
        || manifest.model.version != release.version
        || manifest.model.display_name != model.display_name
        || manifest.model.task != model.task
        || manifest.release.repository.as_deref() != Some(release.repository.as_str())
    {
        return Err(ModelError::new(
            ModelErrorCode::CatalogUnavailable,
            format!(
                "release manifest identity disagrees with catalog entry {}@{}",
                model.id, release.version
            ),
        ));
    }
    Ok(())
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), ModelError> {
    if let Ok(existing) = std::fs::read(path) {
        return if existing == bytes {
            Ok(())
        } else {
            Err(ModelError::new(
                ModelErrorCode::CatalogUnavailable,
                format!("immutable model metadata changed at {}", path.display()),
            ))
        };
    }
    let parent = path.parent().ok_or_else(|| {
        ModelError::new(
            ModelErrorCode::Internal,
            format!("model metadata path has no parent: {}", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        ModelError::new(
            ModelErrorCode::Internal,
            format!(
                "create model metadata directory {}: {error}",
                parent.display()
            ),
        )
    })?;
    let sequence = CACHE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staging = parent.join(format!(
        ".release.valle-staging-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging)
            .map_err(|error| metadata_io_error(&staging, error))?;
        file.write_all(bytes)
            .map_err(|error| metadata_io_error(&staging, error))?;
        file.sync_all()
            .map_err(|error| metadata_io_error(&staging, error))?;
        drop(file);
        publish_immutable(&staging, path, bytes)
    })();
    let _ = std::fs::remove_file(staging);
    result
}

fn publish_immutable(staging: &Path, path: &Path, bytes: &[u8]) -> Result<(), ModelError> {
    // `rename` replaces an existing destination on Unix, so it cannot enforce immutability when
    // two installers publish the same release metadata concurrently. Both names live in the same
    // directory; a hard-link publish is atomic, never replaces, and leaves the fully synced inode
    // reachable after the private staging name is removed.
    match std::fs::hard_link(staging, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|inspect| metadata_io_error(path, inspect))?;
            if !metadata.file_type().is_file() {
                return Err(ModelError::new(
                    ModelErrorCode::CatalogUnavailable,
                    format!(
                        "immutable model metadata path is not a regular file: {}",
                        path.display()
                    ),
                ));
            }
            let existing =
                std::fs::read(path).map_err(|read_error| metadata_io_error(path, read_error))?;
            if existing == bytes {
                Ok(())
            } else {
                Err(ModelError::new(
                    ModelErrorCode::CatalogUnavailable,
                    format!("immutable model metadata changed at {}", path.display()),
                ))
            }
        }
        Err(error) => Err(metadata_io_error(path, error)),
    }
}

fn metadata_io_error(path: &Path, error: std::io::Error) -> ModelError {
    ModelError::new(
        ModelErrorCode::Internal,
        format!("write model metadata {}: {error}", path.display()),
    )
}

fn catalog_error(action: &str, error: anyhow::Error) -> ModelError {
    let message = format!("{action}: {error:#}");
    let code = if message.contains("401") || message.contains("403") {
        ModelErrorCode::AuthRequired
    } else {
        ModelErrorCode::CatalogUnavailable
    };
    ModelError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_version_is_always_the_embedded_default() {
        let root = tempfile::tempdir().unwrap();
        let repository = CatalogRepository::new(root.path().to_owned());
        let release = repository.offline_release("birefnet", None).unwrap();
        assert_eq!(release.release.version, "1.0.0");
        assert_eq!(release.manifest.model.version, "1.0.0");
    }

    #[test]
    fn latest_is_rejected_by_offline_resolution() {
        let root = tempfile::tempdir().unwrap();
        let repository = CatalogRepository::new(root.path().to_owned());
        let error = repository
            .offline_release("birefnet", Some("latest"))
            .unwrap_err();
        assert_eq!(error.code, ModelErrorCode::InvalidSelection);
    }

    #[test]
    fn cached_catalog_cannot_authorize_a_new_install_without_explicit_refresh() {
        let root = tempfile::tempdir().unwrap();
        let repository = CatalogRepository::new(root.path().to_owned());
        let mut catalog = embedded_catalog();
        let model = catalog
            .models
            .iter_mut()
            .find(|model| model.id == "birefnet")
            .unwrap();
        let mut external = model.releases[0].clone();
        external.version = "9.9.9".to_owned();
        model.releases.push(external);
        repository.cache_catalog(&catalog).unwrap();
        let hub = HubClient::new("http://127.0.0.1:1", None).unwrap();

        let error = repository
            .install_release("birefnet", Some("9.9.9"), false, &hub)
            .unwrap_err();

        assert_eq!(error.code, ModelErrorCode::InvalidSelection);
        assert!(error.message.contains("requires --refresh-catalog"));
        assert!(error.hint.as_deref().unwrap().contains("--refresh-catalog"));
    }

    #[test]
    fn immutable_metadata_can_be_reused_but_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("release.json");
        write_immutable(&path, b"one").unwrap();
        write_immutable(&path, b"one").unwrap();
        assert_eq!(
            write_immutable(&path, b"two").unwrap_err().code,
            ModelErrorCode::CatalogUnavailable
        );
        assert_eq!(std::fs::read(path).unwrap(), b"one");
    }

    #[test]
    fn concurrent_immutable_publish_never_replaces_the_winner() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("release.json");
        let first_staging = root.path().join("first.staging");
        let same_staging = root.path().join("same.staging");
        let conflicting_staging = root.path().join("conflicting.staging");
        std::fs::write(&first_staging, b"one").unwrap();
        std::fs::write(&same_staging, b"one").unwrap();
        std::fs::write(&conflicting_staging, b"two").unwrap();

        publish_immutable(&first_staging, &path, b"one").unwrap();
        publish_immutable(&same_staging, &path, b"one").unwrap();
        assert_eq!(
            publish_immutable(&conflicting_staging, &path, b"two")
                .unwrap_err()
                .code,
            ModelErrorCode::CatalogUnavailable
        );
        assert_eq!(std::fs::read(path).unwrap(), b"one");
    }
}
