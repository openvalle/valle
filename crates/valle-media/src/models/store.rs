//! Explicit Hugging Face downloads with immutable revisions and per-file integrity checks.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::models::spec::{
    Artifact, ArtifactFile, Catalog, CatalogRelease, ModelManifest, ReleaseReadiness,
    is_immutable_revision, is_safe_relative_path,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use fs2::FileExt;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

const DEFAULT_ENDPOINT: &str = "https://huggingface.co";
const INSTALLED_MANIFEST_PATH: &str = "release.v1.json";
const INSTALL_LOCK_PATH: &str = "install.lock.json";
static DOWNLOAD_ATTEMPT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static INSTALL_ATTEMPT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct HubClient {
    client: Client,
    endpoint: Url,
    token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FetchedManifest {
    pub manifest: ModelManifest,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub key: ArtifactKey,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    pub artifact: String,
    pub downloaded_files: usize,
    pub reused_files: usize,
}

/// One small release file bundled by the caller for a fully offline legacy migration.
///
/// Legacy caches commonly contain only runtime weights. Callers can provide exact license/card
/// bytes from their pinned release package without granting the store network access.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedMigrationFile<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationResult {
    pub key: ArtifactKey,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    pub artifact: String,
    pub migrated_legacy_files: usize,
    pub materialized_embedded_files: usize,
    pub reused_files: usize,
}

/// Immutable identity of one installed artifact slot.
///
/// `models_root` is deliberately not part of the identity. Callers may relocate the cache while
/// preserving the model/release/artifact coordinates recorded here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ArtifactKey {
    pub model: String,
    pub version: String,
    pub revision: String,
    pub artifact: String,
}

impl ArtifactKey {
    pub fn install_root(&self, models_root: &Path) -> Result<PathBuf> {
        ensure_install_component("model", &self.model)?;
        ensure_install_component("version", &self.version)?;
        ensure!(
            is_immutable_revision(&self.revision),
            "revision must be a 40- or 64-character immutable commit hash"
        );
        ensure_install_component("artifact", &self.artifact)?;
        Ok(models_root
            .join(&self.model)
            .join(&self.version)
            .join(&self.revision)
            .join(&self.artifact))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationReport {
    pub files: usize,
    pub bytes: u64,
}

/// A fully verified artifact resolved from the local store without network access.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledArtifact {
    pub key: ArtifactKey,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    pub manifest_path: String,
    pub manifest_sha256: String,
    pub manifest: ModelManifest,
    pub artifact: Artifact,
    pub verification: VerificationReport,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstallationState {
    Missing,
    Ready,
    Corrupt,
}

/// Per-artifact local state suitable for a product-side model list.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledArtifactStatus {
    pub key: ArtifactKey,
    pub root: PathBuf,
    pub state: InstallationState,
    pub entrypoint: Option<PathBuf>,
    pub files: Option<usize>,
    pub bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct InstallLock {
    schema_version: u32,
    model: String,
    version: String,
    repository: String,
    revision: String,
    manifest_path: String,
    manifest_sha256: String,
    artifact: String,
    license_files: Vec<ArtifactFile>,
    files: Vec<ArtifactFile>,
}

struct InstallPlan {
    artifact: Artifact,
    key: ArtifactKey,
    root: PathBuf,
    lock: InstallLock,
}

impl HubClient {
    pub fn from_env() -> Result<Self> {
        let endpoint = env::var("HF_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.into());
        let token = env::var("HF_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Self::new(&endpoint, token)
    }

    pub fn new(endpoint: &str, token: Option<String>) -> Result<Self> {
        let redacted_endpoint = redact_url_like(endpoint);
        let endpoint = Url::parse(endpoint)
            .with_context(|| format!("invalid Hugging Face endpoint {redacted_endpoint:?}"))?;
        ensure!(
            matches!(endpoint.scheme(), "http" | "https"),
            "Hugging Face endpoint must use http or https"
        );
        let client = Client::builder()
            .user_agent(concat!("valle-media/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            client,
            endpoint,
            token,
        })
    }

    pub fn load_catalog(&self, source: &str) -> Result<Catalog> {
        let bytes = self.read_source(source)?;
        let redacted_source = redact_http_source(source);
        let catalog: Catalog = serde_json::from_slice(&bytes)
            .with_context(|| format!("catalog {redacted_source:?} is not valid JSON"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn fetch_manifest(&self, release: &CatalogRelease) -> Result<FetchedManifest> {
        ensure!(
            is_immutable_revision(&release.revision),
            "refusing moving Hugging Face revision {:?}",
            release.revision
        );
        ensure!(
            is_safe_relative_path(&release.manifest_path),
            "unsafe manifest path {:?}",
            release.manifest_path
        );
        let url = self.resolve_url(
            &release.repository,
            &release.revision,
            &release.manifest_path,
        )?;
        let bytes = self.get_bytes(url.clone())?;
        let redacted_url = redact_url(&url);
        let manifest: ModelManifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("manifest at {redacted_url} is not valid JSON"))?;
        manifest.validate()?;
        ensure!(
            manifest.model.version == release.version,
            "catalog version {:?} does not match manifest version {:?}",
            release.version,
            manifest.model.version
        );
        if let Some(repository) = manifest.release.repository.as_deref() {
            ensure!(
                repository == release.repository,
                "catalog repository {:?} does not match manifest repository {:?}",
                release.repository,
                repository
            );
        }
        ensure!(
            manifest.release.state == ReleaseReadiness::Ready,
            "refusing to install a draft release"
        );
        Ok(FetchedManifest { manifest, bytes })
    }

    pub fn install_artifact(
        &self,
        release: &CatalogRelease,
        fetched: &FetchedManifest,
        artifact_id: &str,
        models_root: &Path,
    ) -> Result<InstallResult> {
        let manifest = &fetched.manifest;
        let plan = prepare_install(release, fetched, artifact_id, models_root)?;
        let artifact = &plan.artifact;
        let key = plan.key;
        let root = plan.root;
        let lock = plan.lock;
        let _guard = ArtifactInstallGuard::acquire(&root)?;

        if verify_install_slot(&root, &lock).is_ok() {
            let entrypoint = safe_join(&root, &artifact.entrypoint)?;
            return Ok(InstallResult {
                key,
                root,
                entrypoint,
                artifact: artifact.id.clone(),
                downloaded_files: 0,
                reused_files: lock.license_files.len() + lock.files.len(),
            });
        }

        remove_stale_staging_slots(&root)?;
        let staging = StagingSlot::create(&root)?;

        let mut downloaded_files = 0;
        for file in self_contained_files(manifest, artifact) {
            let target = safe_join(staging.path(), &file.path)?;
            if let Some(bytes) = crate::models::spec::embedded_support_file(manifest, file) {
                ensure!(
                    bytes.len() as u64 == file.bytes && sha256_bytes(bytes) == file.sha256,
                    "embedded support file {} does not match the model manifest",
                    file.path
                );
                write_new_synced(&target, bytes)?;
                continue;
            }
            let repository_path = repository_release_path(&release.manifest_path, &file.path)?;
            let url = self.resolve_url(&release.repository, &release.revision, &repository_path)?;
            self.download_verified(url, &target, file)?;
            downloaded_files += 1;
        }

        write_new_synced(
            &staging.path().join(INSTALLED_MANIFEST_PATH),
            &fetched.bytes,
        )?;
        write_new_synced(
            &staging.path().join(INSTALL_LOCK_PATH),
            &serde_json::to_vec_pretty(&lock)?,
        )?;
        verify_install_slot(staging.path(), &lock)?;
        sync_directory_tree(staging.path())?;
        staging.commit(&root)?;

        let entrypoint = safe_join(&root, &artifact.entrypoint)?;
        ensure!(
            entrypoint.exists(),
            "installed artifact entrypoint is missing: {}",
            entrypoint.display()
        );
        Ok(InstallResult {
            key,
            root,
            entrypoint,
            artifact: artifact.id.clone(),
            downloaded_files,
            reused_files: 0,
        })
    }

    pub fn resolve_url(&self, repository: &str, revision: &str, path: &str) -> Result<Url> {
        ensure!(
            is_safe_relative_path(path),
            "unsafe Hugging Face path {path:?}"
        );
        let parts: Vec<&str> = repository.split('/').collect();
        ensure!(
            parts.len() == 2 && parts.iter().all(|part| !part.is_empty()),
            "repository must be namespace/name"
        );
        let mut url = self.endpoint.clone();
        url.set_query(None);
        url.set_fragment(None);
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Hugging Face endpoint cannot be a base URL"))?;
            segments.pop_if_empty();
            for part in parts {
                segments.push(part);
            }
            segments.push("resolve").push(revision);
            for component in path.split('/') {
                segments.push(component);
            }
        }
        Ok(url)
    }

    fn read_source(&self, source: &str) -> Result<Vec<u8>> {
        if source.starts_with("https://") || source.starts_with("http://") {
            let redacted_source = redact_http_source(source);
            let url =
                Url::parse(source).with_context(|| format!("invalid URL {redacted_source:?}"))?;
            self.get_bytes(url)
        } else {
            fs::read(source).with_context(|| format!("failed to read {source}"))
        }
    }

    fn get_bytes(&self, url: Url) -> Result<Vec<u8>> {
        let redacted_url = redact_url(&url);
        self.get(url)?
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|error| {
                anyhow!(
                    "failed to read response body from {redacted_url}: {}",
                    error.without_url()
                )
            })
    }

    fn get(&self, url: Url) -> Result<Response> {
        let redacted_url = redact_url(&url);
        let mut request = self.client.get(url.clone());
        if let Some(token) = &self.token
            && url.origin() == self.endpoint.origin()
        {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .context("HF_TOKEN contains invalid header characters")?;
            request = request.header(AUTHORIZATION, value);
        }
        request
            .send()
            .map_err(|error| anyhow!("request failed for {redacted_url}: {}", error.without_url()))?
            .error_for_status()
            .map_err(|error| {
                anyhow!(
                    "Hugging Face returned an error for {redacted_url}: {}",
                    error.without_url()
                )
            })
    }

    fn download_verified(&self, url: Url, target: &Path, expected: &ArtifactFile) -> Result<()> {
        let redacted_url = redact_url(&url);
        let parent = target
            .parent()
            .ok_or_else(|| anyhow!("download target has no parent: {}", target.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        let (partial, mut output) = create_download_partial(target)?;

        let result = (|| -> Result<()> {
            let mut response = self.get(url.clone())?;
            let mut hasher = Sha256::new();
            let mut bytes = 0_u64;
            let mut buffer = [0_u8; 128 * 1024];
            loop {
                let count = response
                    .read(&mut buffer)
                    .with_context(|| format!("failed while downloading {redacted_url}"))?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .with_context(|| format!("failed to write {}", partial.display()))?;
                hasher.update(&buffer[..count]);
                bytes += count as u64;
            }
            output
                .sync_all()
                .with_context(|| format!("failed to sync {}", partial.display()))?;
            let actual_hash = hex_digest(hasher.finalize());
            if bytes != expected.bytes || actual_hash != expected.sha256 {
                bail!(
                    "integrity check failed for {}: bytes {bytes}/{} sha256 {actual_hash}/{}",
                    expected.path,
                    expected.bytes,
                    expected.sha256
                );
            }
            Ok(())
        })();
        drop(output);
        let result = result.and_then(|()| replace_file(&partial, target));

        remove_partial_after_error(&partial, result)
    }
}

fn redact_url(url: &Url) -> String {
    let mut redacted = url.clone();
    redacted.set_query(None);
    redacted.set_fragment(None);
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.to_string()
}

fn redact_http_source(source: &str) -> String {
    if source.starts_with("https://") || source.starts_with("http://") {
        redact_url_like(source)
    } else {
        source.to_owned()
    }
}

fn redact_url_like(value: &str) -> String {
    Url::parse(value).map_or_else(
        |_| {
            let boundary = value
                .char_indices()
                .find_map(|(index, character)| matches!(character, '?' | '#').then_some(index))
                .unwrap_or(value.len());
            let without_suffix = &value[..boundary];
            let Some(authority_start) = without_suffix.find("://").map(|index| index + 3) else {
                return without_suffix.to_owned();
            };
            let authority_end = without_suffix[authority_start..]
                .find('/')
                .map(|index| authority_start + index)
                .unwrap_or(without_suffix.len());
            let Some(userinfo_end) = without_suffix[authority_start..authority_end].rfind('@')
            else {
                return without_suffix.to_owned();
            };
            format!(
                "{}{}",
                &without_suffix[..authority_start],
                &without_suffix[authority_start + userinfo_end + 1..]
            )
        },
        |url| redact_url(&url),
    )
}

fn prepare_install(
    release: &CatalogRelease,
    fetched: &FetchedManifest,
    artifact_id: &str,
    models_root: &Path,
) -> Result<InstallPlan> {
    let manifest = &fetched.manifest;
    manifest.validate()?;
    ensure!(
        is_immutable_revision(&release.revision),
        "refusing moving Hugging Face revision {:?}",
        release.revision
    );
    ensure!(
        release.version == manifest.model.version,
        "catalog and manifest versions disagree"
    );
    ensure!(
        manifest.release.repository.as_deref() == Some(release.repository.as_str()),
        "catalog and manifest repositories disagree"
    );
    ensure!(
        is_safe_relative_path(&release.manifest_path),
        "unsafe manifest path {:?}",
        release.manifest_path
    );
    let artifact = manifest
        .artifact(artifact_id)
        .cloned()
        .ok_or_else(|| anyhow!("manifest has no artifact {artifact_id:?}"))?;
    let metadata_paths = [INSTALLED_MANIFEST_PATH, INSTALL_LOCK_PATH];
    ensure!(
        self_contained_files(manifest, &artifact)
            .all(|file| !metadata_paths.contains(&file.path.as_str())),
        "release files collide with installer metadata"
    );
    let mut unique_paths = std::collections::HashSet::new();
    ensure!(
        self_contained_files(manifest, &artifact)
            .all(|file| unique_paths.insert(file.path.as_str())),
        "license and artifact files contain duplicate paths"
    );
    let key = ArtifactKey {
        model: manifest.model.id.clone(),
        version: manifest.model.version.clone(),
        revision: release.revision.clone(),
        artifact: artifact.id.clone(),
    };
    let root = key.install_root(models_root)?;
    let lock = InstallLock {
        schema_version: 1,
        model: manifest.model.id.clone(),
        version: manifest.model.version.clone(),
        repository: release.repository.clone(),
        revision: release.revision.clone(),
        manifest_path: release.manifest_path.clone(),
        manifest_sha256: sha256_bytes(&fetched.bytes),
        artifact: artifact.id.clone(),
        license_files: manifest.license.files.clone(),
        files: artifact.files.clone(),
    };
    Ok(InstallPlan {
        artifact,
        key,
        root,
        lock,
    })
}

pub fn default_models_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("VALLE_MODEL_CACHE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("VALLE_CACHE_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join("models"));
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("Valle").join("models"));
    }
    #[cfg(target_os = "macos")]
    if let Some(path) = env::var_os("HOME") {
        return Ok(PathBuf::from(path)
            .join("Library")
            .join("Caches")
            .join("valle")
            .join("models"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(path).join("valle").join("models"));
        }
        if let Some(path) = env::var_os("HOME") {
            return Ok(PathBuf::from(path)
                .join(".cache")
                .join("valle")
                .join("models"));
        }
    }
    Err(anyhow!(
        "cannot determine cache directory; set VALLE_MODEL_CACHE"
    ))
}

#[deprecated(note = "use default_models_root; the returned path is already the models root")]
pub fn default_cache_dir() -> Result<PathBuf> {
    default_models_root()
}

/// Move a verified legacy artifact into the immutable store without network access.
///
/// Artifact files must already exist under `legacy_root`. Small release-owned files that were not
/// part of the old cache (for example `card/LICENSE`) must be supplied in `embedded_files`; their
/// bytes are checked against the new release manifest before commit. The legacy directory is
/// never modified. All work happens in a hidden staging slot and becomes visible in one directory
/// rename only after the complete artifact, manifest, lock and entrypoint verify successfully.
pub fn migrate_legacy_artifact(
    release: &CatalogRelease,
    fetched: &FetchedManifest,
    artifact_id: &str,
    legacy_root: &Path,
    embedded_files: &[EmbeddedMigrationFile<'_>],
    models_root: &Path,
) -> Result<MigrationResult> {
    ensure!(
        legacy_root.is_dir(),
        "legacy artifact directory is missing: {}",
        legacy_root.display()
    );
    let manifest = &fetched.manifest;
    let plan = prepare_install(release, fetched, artifact_id, models_root)?;
    let artifact = &plan.artifact;
    let key = plan.key;
    let root = plan.root;
    let lock = plan.lock;
    let mut embedded_by_path = std::collections::HashMap::new();
    for embedded in embedded_files {
        ensure!(
            is_safe_relative_path(embedded.path),
            "unsafe embedded migration path {:?}",
            embedded.path
        );
        ensure!(
            embedded_by_path
                .insert(embedded.path, embedded.bytes)
                .is_none(),
            "duplicate embedded migration path {:?}",
            embedded.path
        );
    }
    ensure!(
        embedded_by_path.keys().all(|path| manifest
            .license
            .files
            .iter()
            .any(|file| file.path == **path)),
        "embedded migration files may only provide declared license files"
    );

    // Fail before creating a staging slot if any large legacy artifact is incomplete or corrupt.
    verify_files(legacy_root, &artifact.files)
        .context("legacy artifact does not match the new release manifest")?;

    let _guard = ArtifactInstallGuard::acquire(&root)?;
    if verify_install_slot(&root, &lock).is_ok() {
        return Ok(MigrationResult {
            key,
            root: root.clone(),
            entrypoint: safe_join(&root, &artifact.entrypoint)?,
            artifact: artifact.id.clone(),
            migrated_legacy_files: 0,
            materialized_embedded_files: 0,
            reused_files: lock.license_files.len() + lock.files.len(),
        });
    }

    remove_stale_staging_slots(&root)?;
    let staging = StagingSlot::create(&root)?;
    let mut migrated_legacy_files = 0;
    let mut materialized_embedded_files = 0;

    for file in &artifact.files {
        migrate_legacy_file(legacy_root, staging.path(), file)?;
        migrated_legacy_files += 1;
    }
    for file in &manifest.license.files {
        let legacy_path = safe_join(legacy_root, &file.path)?;
        if legacy_path.exists() {
            migrate_legacy_file(legacy_root, staging.path(), file)?;
            migrated_legacy_files += 1;
            continue;
        }
        let bytes = embedded_by_path.get(file.path.as_str()).ok_or_else(|| {
            anyhow!(
                "legacy cache is missing {:?} and no exact embedded release file was provided",
                file.path
            )
        })?;
        ensure!(
            bytes.len() as u64 == file.bytes && sha256_bytes(bytes) == file.sha256,
            "embedded migration file {:?} does not match the new release manifest",
            file.path
        );
        write_new_synced(&safe_join(staging.path(), &file.path)?, bytes)?;
        materialized_embedded_files += 1;
    }

    write_new_synced(
        &staging.path().join(INSTALLED_MANIFEST_PATH),
        &fetched.bytes,
    )?;
    write_new_synced(
        &staging.path().join(INSTALL_LOCK_PATH),
        &serde_json::to_vec_pretty(&lock)?,
    )?;
    verify_install_slot(staging.path(), &lock)?;
    sync_directory_tree(staging.path())?;
    staging.commit(&root)?;

    Ok(MigrationResult {
        key,
        root: root.clone(),
        entrypoint: safe_join(&root, &artifact.entrypoint)?,
        artifact: artifact.id.clone(),
        migrated_legacy_files,
        materialized_embedded_files,
        reused_files: 0,
    })
}

/// Resolve and verify one immutable artifact using only files in `models_root`.
///
/// This function never constructs a [`HubClient`] and never performs HTTP. A successful result
/// means the stored release identity, manifest checksum, artifact-specific lock, every declared
/// license/artifact checksum, and the entrypoint all match.
pub fn resolve_installed(models_root: &Path, key: &ArtifactKey) -> Result<InstalledArtifact> {
    let root = key.install_root(models_root)?;
    load_installed_slot(&root, key).map(|(installed, _)| installed)
}

/// Re-run all local integrity checks for one immutable artifact without network access.
pub fn verify_installed(models_root: &Path, key: &ArtifactKey) -> Result<VerificationReport> {
    Ok(resolve_installed(models_root, key)?.verification)
}

/// Inspect one artifact slot without collapsing missing and corrupt states.
pub fn installed_artifact_status(
    models_root: &Path,
    key: &ArtifactKey,
) -> Result<InstalledArtifactStatus> {
    let root = key.install_root(models_root)?;
    if !root.exists() {
        return Ok(InstalledArtifactStatus {
            key: key.clone(),
            root,
            state: InstallationState::Missing,
            entrypoint: None,
            files: None,
            bytes: None,
            error: None,
        });
    }
    Ok(match load_installed_slot(&root, key) {
        Ok((installed, _)) => InstalledArtifactStatus {
            key: installed.key,
            root: installed.root,
            state: InstallationState::Ready,
            entrypoint: Some(installed.entrypoint),
            files: Some(installed.verification.files),
            bytes: Some(installed.verification.bytes),
            error: None,
        },
        Err(error) => InstalledArtifactStatus {
            key: key.clone(),
            root,
            state: InstallationState::Corrupt,
            entrypoint: None,
            files: None,
            bytes: None,
            error: Some(format!("{error:#}")),
        },
    })
}

pub fn verify_artifact(root: &Path, artifact: &Artifact) -> Result<VerificationReport> {
    let mut bytes = 0;
    for file in &artifact.files {
        let path = safe_join(root, &file.path)?;
        verify_file(&path, file)?;
        bytes += file.bytes;
    }
    Ok(VerificationReport {
        files: artifact.files.len(),
        bytes,
    })
}

pub fn verify_license(root: &Path, manifest: &ModelManifest) -> Result<VerificationReport> {
    verify_files(root, &manifest.license.files)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_digest(hasher.finalize()))
}

fn verify_file(path: &Path, expected: &ArtifactFile) -> Result<bool> {
    let metadata =
        fs::metadata(path).with_context(|| format!("missing artifact file {}", path.display()))?;
    ensure!(metadata.is_file(), "{} is not a file", path.display());
    ensure!(
        metadata.len() == expected.bytes,
        "{} has {} bytes, expected {}",
        path.display(),
        metadata.len(),
        expected.bytes
    );
    let actual = sha256_file(path)?;
    ensure!(
        actual == expected.sha256,
        "{} has sha256 {}, expected {}",
        path.display(),
        actual,
        expected.sha256
    );
    Ok(true)
}

fn verify_files(root: &Path, files: &[ArtifactFile]) -> Result<VerificationReport> {
    let mut bytes = 0;
    for file in files {
        let path = safe_join(root, &file.path)?;
        verify_file(&path, file)?;
        bytes += file.bytes;
    }
    Ok(VerificationReport {
        files: files.len(),
        bytes,
    })
}

fn migrate_legacy_file(source_root: &Path, staging_root: &Path, file: &ArtifactFile) -> Result<()> {
    let source = safe_join(source_root, &file.path)?;
    let source_metadata = fs::symlink_metadata(&source)
        .with_context(|| format!("missing legacy artifact file {}", source.display()))?;
    ensure!(
        source_metadata.file_type().is_file(),
        "legacy artifact source is not a regular file: {}",
        source.display()
    );
    verify_file(&source, file).with_context(|| {
        format!(
            "legacy artifact file failed verification: {}",
            source.display()
        )
    })?;

    let target = safe_join(staging_root, &file.path)?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("migration target has no parent: {}", target.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create migration directory {}", parent.display()))?;
    if let Err(link_error) = fs::hard_link(&source, &target) {
        fs::copy(&source, &target).with_context(|| {
            format!(
                "failed to hard-link ({link_error}) or copy legacy file {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }
    File::open(&target)
        .with_context(|| format!("failed to open migrated file {}", target.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync migrated file {}", target.display()))?;
    verify_file(&target, file)
        .with_context(|| format!("migrated file failed verification: {}", target.display()))?;
    Ok(())
}

fn self_contained_files<'a>(
    manifest: &'a ModelManifest,
    artifact: &'a Artifact,
) -> impl Iterator<Item = &'a ArtifactFile> {
    manifest.license.files.iter().chain(&artifact.files)
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        is_safe_relative_path(relative),
        "unsafe relative path {relative:?}"
    );
    Ok(root.join(relative))
}

fn ensure_install_component(field: &str, value: &str) -> Result<()> {
    ensure!(
        !value.contains('/') && is_safe_relative_path(value),
        "unsafe {field} install component {value:?}"
    );
    Ok(())
}

struct ArtifactInstallGuard {
    file: File,
}

impl ArtifactInstallGuard {
    fn acquire(root: &Path) -> Result<Self> {
        let lock_path = coordination_lock_path(root)?;
        let parent = lock_path
            .parent()
            .ok_or_else(|| anyhow!("install lock has no parent: {}", lock_path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create model cache {}", parent.display()))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open install lock {}", lock_path.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("failed to acquire install lock {}", lock_path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for ArtifactInstallGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

struct StagingSlot {
    path: PathBuf,
    committed: bool,
}

impl StagingSlot {
    fn create(root: &Path) -> Result<Self> {
        let parent = root
            .parent()
            .ok_or_else(|| anyhow!("artifact slot has no parent: {}", root.display()))?;
        let prefix = staging_slot_prefix(root)?;
        loop {
            let sequence = INSTALL_ATTEMPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{prefix}{}.{}", std::process::id(), sequence));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        committed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create staging slot {}", path.display())
                    });
                }
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(mut self, root: &Path) -> Result<()> {
        remove_path_if_present(root)?;
        fs::rename(&self.path, root).with_context(|| {
            format!(
                "failed to commit staging slot {} to {}",
                self.path.display(),
                root.display()
            )
        })?;
        self.committed = true;
        if let Some(parent) = root.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }
}

impl Drop for StagingSlot {
    fn drop(&mut self) {
        if !self.committed {
            let _ = remove_path_if_present(&self.path);
        }
    }
}

fn coordination_lock_path(root: &Path) -> Result<PathBuf> {
    let parent = root
        .parent()
        .ok_or_else(|| anyhow!("artifact slot has no parent: {}", root.display()))?;
    let artifact = root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("artifact slot has no UTF-8 file name: {}", root.display()))?;
    ensure_install_component("artifact", artifact)?;
    Ok(parent.join(format!(".{artifact}.install.lock")))
}

fn staging_slot_prefix(root: &Path) -> Result<String> {
    let artifact = root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("artifact slot has no UTF-8 file name: {}", root.display()))?;
    ensure_install_component("artifact", artifact)?;
    Ok(format!(".{artifact}.staging."))
}

fn remove_stale_staging_slots(root: &Path) -> Result<()> {
    let parent = root
        .parent()
        .ok_or_else(|| anyhow!("artifact slot has no parent: {}", root.display()))?;
    let prefix = staging_slot_prefix(root)?;
    for entry in fs::read_dir(parent)
        .with_context(|| format!("failed to inspect model cache {}", parent.display()))?
    {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            remove_path_if_present(&entry.path())?;
        }
    }
    Ok(())
}

fn verify_install_slot(root: &Path, expected_lock: &InstallLock) -> Result<VerificationReport> {
    let key = ArtifactKey {
        model: expected_lock.model.clone(),
        version: expected_lock.version.clone(),
        revision: expected_lock.revision.clone(),
        artifact: expected_lock.artifact.clone(),
    };
    let (installed, installed_lock) = load_installed_slot(root, &key)?;
    ensure!(
        installed_lock == *expected_lock
            && installed.manifest_sha256 == expected_lock.manifest_sha256,
        "installed artifact lock does not match the requested artifact"
    );
    Ok(installed.verification)
}

fn load_installed_slot(
    root: &Path,
    expected_key: &ArtifactKey,
) -> Result<(InstalledArtifact, InstallLock)> {
    ensure!(
        root.is_dir(),
        "artifact slot is missing: {}",
        root.display()
    );
    let installed_lock: InstallLock =
        serde_json::from_slice(&fs::read(root.join(INSTALL_LOCK_PATH)).with_context(|| {
            format!(
                "failed to read install lock {}",
                root.join(INSTALL_LOCK_PATH).display()
            )
        })?)
        .context("installed artifact lock is not valid JSON")?;
    ensure!(
        installed_lock.schema_version == 1,
        "unsupported install lock schema version {}",
        installed_lock.schema_version
    );
    ensure!(
        installed_lock.model == expected_key.model
            && installed_lock.version == expected_key.version
            && installed_lock.revision == expected_key.revision
            && installed_lock.artifact == expected_key.artifact,
        "installed artifact lock identity does not match the requested artifact"
    );
    ensure!(
        is_safe_relative_path(&installed_lock.manifest_path),
        "installed artifact lock contains an unsafe manifest path"
    );
    let manifest_bytes = fs::read(root.join(INSTALLED_MANIFEST_PATH)).with_context(|| {
        format!(
            "failed to read installed manifest {}",
            root.join(INSTALLED_MANIFEST_PATH).display()
        )
    })?;
    ensure!(
        sha256_bytes(&manifest_bytes) == installed_lock.manifest_sha256,
        "installed manifest checksum does not match the install lock"
    );
    let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes)
        .context("installed release manifest is not valid JSON")?;
    manifest.validate()?;
    ensure!(
        manifest.model.id == installed_lock.model
            && manifest.model.version == installed_lock.version
            && manifest.release.repository.as_deref() == Some(installed_lock.repository.as_str()),
        "installed manifest identity does not match the install lock"
    );
    ensure!(
        manifest.license.files == installed_lock.license_files,
        "installed license file list does not match the install lock"
    );
    let artifact = manifest
        .artifact(&installed_lock.artifact)
        .cloned()
        .ok_or_else(|| anyhow!("installed manifest has no locked artifact"))?;
    ensure!(
        artifact.files == installed_lock.files,
        "installed artifact file list does not match the install lock"
    );

    let verification = verify_files(
        root,
        &installed_lock
            .license_files
            .iter()
            .chain(&installed_lock.files)
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let entrypoint = safe_join(root, &artifact.entrypoint)?;
    ensure!(
        entrypoint.exists(),
        "installed artifact entrypoint is missing: {}",
        entrypoint.display()
    );
    Ok((
        InstalledArtifact {
            key: expected_key.clone(),
            root: root.to_path_buf(),
            entrypoint,
            manifest_path: installed_lock.manifest_path.clone(),
            manifest_sha256: installed_lock.manifest_sha256.clone(),
            manifest,
            artifact,
            verification,
        },
        installed_lock,
    ))
}

fn remove_path_if_present(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove file {}", path.display()))
    }
}

fn repository_release_path(manifest_path: &str, relative: &str) -> Result<String> {
    ensure!(
        is_safe_relative_path(manifest_path),
        "unsafe manifest path {manifest_path:?}"
    );
    ensure!(
        is_safe_relative_path(relative),
        "unsafe release file path {relative:?}"
    );
    Ok(match manifest_path.rsplit_once('/') {
        Some((directory, _)) => format!("{directory}/{relative}"),
        None => relative.to_owned(),
    })
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = digest.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn write_new_synced(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("target has no parent: {}", target.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .with_context(|| format!("failed to create {}", target.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", target.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", target.display()))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open directory {} for sync", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn sync_directory_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to inspect staging slot {}", root.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            sync_directory_tree(&path)?;
        }
    }
    sync_directory(root)
}

fn replace_file(source: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        return fs::rename(source, target).with_context(|| {
            format!(
                "failed to move {} to {}",
                source.display(),
                target.display()
            )
        });
    }
    let backup = target.with_extension(format!("invalid.{}", std::process::id()));
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(target, &backup)?;
    match fs::rename(source, target) {
        Ok(()) => {
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, target);
            Err(error).with_context(|| {
                format!(
                    "failed to replace {} with {}",
                    target.display(),
                    source.display()
                )
            })
        }
    }
}

fn remove_partial_after_error<T>(partial: &Path, result: Result<T>) -> Result<T> {
    let Err(error) = result else {
        return result;
    };
    match fs::remove_file(partial) {
        Ok(()) => Err(error),
        Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => Err(error),
        Err(cleanup_error) => Err(error.context(format!(
            "also failed to remove partial download {}: {cleanup_error}",
            partial.display()
        ))),
    }
}

fn create_download_partial(target: &Path) -> Result<(PathBuf, File)> {
    loop {
        let sequence = DOWNLOAD_ATTEMPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let partial = target.with_extension(format!("partial.{}.{}", std::process::id(), sequence));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
        {
            Ok(file) => return Ok((partial, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", partial.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Shutdown, TcpListener};
    use std::process::{Command, Output, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::models::spec::ReleaseReadiness;

    use super::*;

    #[test]
    fn resolve_url_encodes_each_path_segment() {
        let hub = HubClient::new("https://huggingface.co", None).unwrap();
        let url = hub
            .resolve_url(
                "valle/modnet",
                "0123456789012345678901234567890123456789",
                "models/a file.onnx",
            )
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://huggingface.co/valle/modnet/resolve/0123456789012345678901234567890123456789/models/a%20file.onnx"
        );
    }

    #[test]
    fn safe_join_rejects_parent_components() {
        assert!(safe_join(Path::new("cache"), "model/model.onnx").is_ok());
        assert!(safe_join(Path::new("cache"), "../model.onnx").is_err());
    }

    #[test]
    fn release_files_are_relative_to_the_manifest_directory() {
        assert_eq!(
            repository_release_path("modnet/release.v1.json", "artifacts/model.onnx").unwrap(),
            "modnet/artifacts/model.onnx"
        );
        assert_eq!(
            repository_release_path("release.v1.json", "model.onnx").unwrap(),
            "model.onnx"
        );
        assert!(repository_release_path("modnet/release.v1.json", "../model.onnx").is_err());
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_bytes(b"valle"),
            "abecf0e6de8dec8beed7ca71e08c7f6ef30d1d231c53501f123b41b6fd0d18bd"
        );
    }

    #[test]
    fn qwen_downloads_only_official_artifact_files_and_installs_embedded_license() {
        let release = crate::models::spec::embedded_catalog()
            .model("qwen3-asr-0.6b")
            .unwrap()
            .release(None)
            .unwrap()
            .clone();
        let mut manifest =
            crate::models::spec::embedded_release_manifest("qwen3-asr-0.6b", "1.0.0").unwrap();
        // Exercise the real official-source plan with tiny payloads, not multi-GB test weights.
        for file in &mut manifest.artifacts[0].files {
            file.bytes = 4;
            file.sha256 = sha256_bytes(b"test");
        }
        let requested = manifest.artifacts[0]
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let revision = release.revision.clone();
        let server = thread::spawn(move || {
            for file in requested {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 8192];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                assert!(
                    request.starts_with(&format!(
                        "GET /Qwen/Qwen3-ASR-0.6B/resolve/{revision}/{file} "
                    )),
                    "{request}"
                );
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest",
                    )
                    .unwrap();
            }
        });
        let fetched = FetchedManifest {
            bytes: serde_json::to_vec(&manifest).unwrap(),
            manifest,
        };
        let root = tempfile::tempdir().unwrap();
        let hub = HubClient::new(&endpoint, None).unwrap();
        let installed = hub
            .install_artifact(&release, &fetched, "safetensors-bf16", root.path())
            .unwrap();
        server.join().unwrap();
        assert_eq!(installed.downloaded_files, 4);
        assert_eq!(
            std::fs::read(installed.root.join("card/LICENSE")).unwrap(),
            include_bytes!("catalog/qwen-LICENSE")
        );
        let reused = hub
            .install_artifact(&release, &fetched, "safetensors-bf16", root.path())
            .unwrap();
        assert_eq!(reused.downloaded_files, 0);
        assert_eq!(reused.reused_files, 5);
    }

    #[test]
    fn fetches_nested_manifest_with_token_for_configured_endpoint() {
        let mut manifest: ModelManifest =
            serde_json::from_str(include_str!("catalog/release-modnet-1.0.0.json")).unwrap();
        manifest.release.state = ReleaseReadiness::Ready;
        manifest.release.repository = Some("openvalle/valle-models".into());
        let body = serde_json::to_vec(&manifest).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with(
                "GET /openvalle/valle-models/resolve/0123456789012345678901234567890123456789/modnet/release.v1.json "
            ));
            assert!(
                request
                    .lines()
                    .any(|line| { line.eq_ignore_ascii_case("authorization: Bearer test-token") })
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let hub = HubClient::new(&format!("http://{address}"), Some("test-token".into())).unwrap();
        let release = CatalogRelease {
            version: "1.0.0".into(),
            repository: "openvalle/valle-models".into(),
            revision: "0123456789012345678901234567890123456789".into(),
            manifest_path: "modnet/release.v1.json".into(),
        };
        let fetched = hub.fetch_manifest(&release).unwrap();
        server.join().unwrap();
        assert_eq!(fetched.manifest.model.id, "modnet");
    }

    #[test]
    fn does_not_send_hf_token_to_external_catalog_origin() {
        let body = br#"{"schema_version":1,"generated_by":"test","models":[]}"#.to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with("GET /catalog.v1.json "));
            assert!(
                !request
                    .lines()
                    .any(|line| { line.to_ascii_lowercase().starts_with("authorization:") })
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let hub = HubClient::new("http://127.0.0.1:1", Some("must-not-leak".into())).unwrap();
        let catalog = hub
            .load_catalog(&format!("http://{address}/catalog.v1.json"))
            .unwrap();
        server.join().unwrap();
        assert!(catalog.models.is_empty());
    }

    #[test]
    fn catalog_errors_never_expose_url_query_or_fragment_secrets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4_096];
            let _ = stream.read(&mut request).unwrap();
            let body = b"not-json";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        let hub = HubClient::new("http://127.0.0.1:1", None).unwrap();
        let error = hub
            .load_catalog(&format!(
                "http://{address}/catalog.v1.json?token=query-secret#fragment-secret"
            ))
            .unwrap_err();
        server.join().unwrap();
        let error = format!("{error:#}");

        assert!(error.contains("/catalog.v1.json"));
        assert!(!error.contains("query-secret"));
        assert!(!error.contains("fragment-secret"));
        assert!(!error.contains("token="));
    }

    #[test]
    fn catalog_parse_and_request_failures_use_only_redacted_urls() {
        let hub = HubClient::new("http://127.0.0.1:1", None).unwrap();
        let parse_error = hub
            .load_catalog(
                "http://private-user:private-password@[::1?token=parse-secret#fragment-secret",
            )
            .unwrap_err();
        let parse_error = format!("{parse_error:#}");
        assert!(!parse_error.contains("private-user"));
        assert!(!parse_error.contains("private-password"));
        assert!(!parse_error.contains("parse-secret"));
        assert!(!parse_error.contains("fragment-secret"));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let request_error = hub
            .load_catalog(&format!(
                "http://{address}/catalog.v1.json?signature=request-secret"
            ))
            .unwrap_err();
        let request_error = format!("{request_error:#}");
        assert!(request_error.contains("/catalog.v1.json"));
        assert!(!request_error.contains("request-secret"));
        assert!(!request_error.contains("signature="));
    }

    #[test]
    fn install_downloads_once_then_reuses_verified_cache() {
        let body = b"model".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for path in ["LICENSE", "tiny.onnx"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                assert!(request.starts_with(&format!(
                    "GET /valle/modnet/resolve/0123456789012345678901234567890123456789/modnet/{path} "
                )));
                assert!(
                    !request
                        .lines()
                        .any(|line| { line.to_ascii_lowercase().starts_with("authorization:") })
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });

        let (fetched, release) = tiny_release();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache = env::temp_dir().join(format!(
            "valle-model-store-test-{}-{unique}",
            std::process::id()
        ));
        let hub = HubClient::new(&format!("http://{address}"), None).unwrap();
        let first = hub
            .install_artifact(&release, &fetched, "onnx-fp32", &cache)
            .unwrap();
        server.join().unwrap();
        assert_eq!(first.downloaded_files, 2);
        assert_eq!(first.reused_files, 0);
        assert_eq!(first.key.artifact, "onnx-fp32");
        assert!(
            first
                .root
                .ends_with("modnet/1.0.0/0123456789012345678901234567890123456789/onnx-fp32")
        );
        assert_eq!(fs::read(&first.entrypoint).unwrap(), b"model");
        assert_eq!(fs::read(first.root.join("LICENSE")).unwrap(), b"model");
        assert!(first.root.join(INSTALLED_MANIFEST_PATH).is_file());
        let lock: InstallLock =
            serde_json::from_slice(&fs::read(first.root.join(INSTALL_LOCK_PATH)).unwrap()).unwrap();
        assert_eq!(lock.manifest_path, "modnet/release.v1.json");

        let second = hub
            .install_artifact(&release, &fetched, "onnx-fp32", &cache)
            .unwrap();
        assert_eq!(second.downloaded_files, 0);
        assert_eq!(second.reused_files, 2);

        let installed = resolve_installed(&cache, &first.key).unwrap();
        assert_eq!(installed.key, first.key);
        assert_eq!(installed.entrypoint, first.entrypoint);
        assert_eq!(installed.artifact.id, "onnx-fp32");
        assert_eq!(
            installed.verification,
            VerificationReport {
                files: 2,
                bytes: 10,
            }
        );
        assert_eq!(
            verify_installed(&cache, &first.key).unwrap(),
            installed.verification
        );
        let status = installed_artifact_status(&cache, &first.key).unwrap();
        assert_eq!(status.state, InstallationState::Ready);
        assert_eq!(
            status.entrypoint.as_deref(),
            Some(first.entrypoint.as_path())
        );
        assert_eq!(status.files, Some(2));
        assert_eq!(status.bytes, Some(10));
        assert!(status.error.is_none());
        fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn offline_status_distinguishes_missing_and_corrupt_artifacts() {
        let missing_root = unique_test_root("offline-missing");
        let (fetched, release) = tiny_release();
        let key = test_artifact_key(&fetched, &release);
        let status = installed_artifact_status(&missing_root, &key).unwrap();
        assert_eq!(status.state, InstallationState::Missing);
        assert!(status.error.is_none());
        assert!(resolve_installed(&missing_root, &key).is_err());

        let corrupt_root = unique_test_root("offline-corrupt");
        let slot = write_test_install_slot(&corrupt_root);
        fs::write(slot.join("tiny.onnx"), b"wrong").unwrap();
        let status = installed_artifact_status(&corrupt_root, &key).unwrap();
        assert_eq!(status.state, InstallationState::Corrupt);
        assert!(status.entrypoint.is_none());
        assert!(status.error.unwrap().contains("sha256"));
        assert!(verify_installed(&corrupt_root, &key).is_err());

        fs::remove_dir_all(corrupt_root).unwrap();
    }

    #[test]
    fn offline_resolution_rejects_lock_identity_and_manifest_tampering() {
        let identity_root = unique_test_root("offline-lock-identity");
        let (fetched, release) = tiny_release();
        let key = test_artifact_key(&fetched, &release);
        let slot = write_test_install_slot(&identity_root);
        let lock_path = slot.join(INSTALL_LOCK_PATH);
        let mut lock: InstallLock = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
        lock.artifact = "other-artifact".into();
        fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        let error = resolve_installed(&identity_root, &key).unwrap_err();
        assert!(error.to_string().contains("identity"));
        fs::remove_dir_all(identity_root).unwrap();

        let manifest_root = unique_test_root("offline-manifest-checksum");
        let slot = write_test_install_slot(&manifest_root);
        fs::write(slot.join(INSTALLED_MANIFEST_PATH), b"{}").unwrap();
        let error = resolve_installed(&manifest_root, &key).unwrap_err();
        assert!(error.to_string().contains("manifest checksum"));
        fs::remove_dir_all(manifest_root).unwrap();
    }

    #[test]
    fn legacy_migration_is_offline_verified_and_reusable() {
        let models_root = unique_test_root("legacy-migration");
        let legacy_root = models_root.join("modnet");
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(legacy_root.join("tiny.onnx"), b"model").unwrap();
        let (fetched, release) = tiny_release();
        let embedded = [EmbeddedMigrationFile {
            path: "LICENSE",
            bytes: b"model",
        }];

        let first = migrate_legacy_artifact(
            &release,
            &fetched,
            "onnx-fp32",
            &legacy_root,
            &embedded,
            &models_root,
        )
        .unwrap();
        assert_eq!(first.migrated_legacy_files, 1);
        assert_eq!(first.materialized_embedded_files, 1);
        assert_eq!(first.reused_files, 0);
        assert_eq!(fs::read(legacy_root.join("tiny.onnx")).unwrap(), b"model");
        let installed = resolve_installed(&models_root, &first.key).unwrap();
        assert_eq!(installed.entrypoint, first.entrypoint);
        assert_eq!(installed.verification.files, 2);

        let second = migrate_legacy_artifact(
            &release,
            &fetched,
            "onnx-fp32",
            &legacy_root,
            &embedded,
            &models_root,
        )
        .unwrap();
        assert_eq!(second.migrated_legacy_files, 0);
        assert_eq!(second.materialized_embedded_files, 0);
        assert_eq!(second.reused_files, 2);
        fs::remove_dir_all(models_root).unwrap();
    }

    #[test]
    fn failed_legacy_migration_preserves_source_and_hides_new_slot() {
        let models_root = unique_test_root("legacy-migration-failure");
        let legacy_root = models_root.join("modnet");
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(legacy_root.join("tiny.onnx"), b"model").unwrap();
        let (fetched, release) = tiny_release();
        let key = test_artifact_key(&fetched, &release);
        let root = key.install_root(&models_root).unwrap();

        let error = migrate_legacy_artifact(
            &release,
            &fetched,
            "onnx-fp32",
            &legacy_root,
            &[],
            &models_root,
        )
        .unwrap_err();
        assert!(error.to_string().contains("no exact embedded release file"));
        assert_eq!(fs::read(legacy_root.join("tiny.onnx")).unwrap(), b"model");
        assert!(!root.exists());
        assert!(staging_slots(&root).is_empty());

        fs::write(legacy_root.join("tiny.onnx"), b"wrong").unwrap();
        let embedded = [EmbeddedMigrationFile {
            path: "LICENSE",
            bytes: b"model",
        }];
        let error = migrate_legacy_artifact(
            &release,
            &fetched,
            "onnx-fp32",
            &legacy_root,
            &embedded,
            &models_root,
        )
        .unwrap_err();
        assert!(error.to_string().contains("legacy artifact"));
        assert_eq!(fs::read(legacy_root.join("tiny.onnx")).unwrap(), b"wrong");
        assert!(!root.exists());
        assert!(staging_slots(&root).is_empty());
        fs::remove_dir_all(models_root).unwrap();
    }

    #[test]
    fn artifact_slot_is_invisible_until_the_complete_stage_is_committed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (artifact_request_tx, artifact_request_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut license_stream, _) = listener.accept().unwrap();
            read_http_request(&mut license_stream, "/modnet/LICENSE ");
            write_model_response(&mut license_stream);

            let (mut artifact_stream, _) = listener.accept().unwrap();
            read_http_request(&mut artifact_stream, "/modnet/tiny.onnx ");
            artifact_request_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            write_model_response(&mut artifact_stream);
        });

        let (fetched, release) = tiny_release();
        let models_root = unique_test_root("visibility");
        let root = artifact_install_root(&models_root);
        let install_root = models_root.clone();
        let install = thread::spawn(move || {
            HubClient::new(&format!("http://{address}"), None)
                .unwrap()
                .install_artifact(&release, &fetched, "onnx-fp32", &install_root)
        });

        artifact_request_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(
            !root.exists(),
            "the final artifact slot became visible before commit"
        );
        assert_eq!(staging_slots(&root).len(), 1);

        finish_tx.send(()).unwrap();
        let installed = install.join().unwrap().unwrap();
        server.join().unwrap();
        assert_eq!(installed.root, root);
        assert!(root.join(INSTALL_LOCK_PATH).is_file());
        assert!(staging_slots(&root).is_empty());
        fs::remove_dir_all(models_root).unwrap();
    }

    #[test]
    fn concurrent_processes_serialize_one_artifact_install() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let server_stop = Arc::clone(&stop);
        let server_requests = Arc::clone(&requests);
        let server = thread::spawn(move || {
            while !server_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // The nonblocking listener can yield a nonblocking accepted socket on
                        // some platforms. Requests are handled serially in this test thread, so
                        // restore blocking I/O before reading the complete child request.
                        stream.set_nonblocking(false).unwrap();
                        let mut request = [0_u8; 4096];
                        let count = stream.read(&mut request).unwrap();
                        let request = String::from_utf8_lossy(&request[..count]);
                        assert!(
                            request.contains("/modnet/LICENSE ")
                                || request.contains("/modnet/tiny.onnx ")
                        );
                        server_requests.fetch_add(1, Ordering::AcqRel);
                        thread::sleep(Duration::from_millis(100));
                        write_model_response(&mut stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("test server failed: {error}"),
                }
            }
        });

        let models_root = unique_test_root("process-lock");
        let current_exe = env::current_exe().unwrap();
        let spawn_child = || {
            Command::new(&current_exe)
                .args([
                    "--exact",
                    "models::store::tests::concurrent_install_child",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env("VALLE_MODEL_STORE_CHILD_ROOT", &models_root)
                .env(
                    "VALLE_MODEL_STORE_CHILD_ENDPOINT",
                    format!("http://{address}"),
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        };

        let first = spawn_child();
        let second = spawn_child();
        let first = first.wait_with_output().unwrap();
        let second = second.wait_with_output().unwrap();
        stop.store(true, Ordering::Release);
        server.join().unwrap();

        assert_child_succeeded("first", &first);
        assert_child_succeeded("second", &second);
        assert_eq!(
            requests.load(Ordering::Acquire),
            2,
            "only the lock owner should download the two release files"
        );
        let root = artifact_install_root(&models_root);
        let (fetched, release) = tiny_release();
        let lock = expected_test_lock(&fetched, &release);
        verify_install_slot(&root, &lock).unwrap();
        assert!(staging_slots(&root).is_empty());
        fs::remove_dir_all(models_root).unwrap();
    }

    #[test]
    #[ignore = "invoked as a subprocess by concurrent_processes_serialize_one_artifact_install"]
    fn concurrent_install_child() {
        let Some(models_root) = env::var_os("VALLE_MODEL_STORE_CHILD_ROOT") else {
            return;
        };
        let endpoint = env::var("VALLE_MODEL_STORE_CHILD_ENDPOINT").unwrap();
        let (fetched, release) = tiny_release();
        HubClient::new(&endpoint, None)
            .unwrap()
            .install_artifact(&release, &fetched, "onnx-fp32", Path::new(&models_root))
            .unwrap();
    }

    #[test]
    fn distinct_artifacts_keep_distinct_files_and_install_locks() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for path in ["LICENSE", "tiny.onnx", "LICENSE", "tiny.mlpackage"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                assert!(request.contains(&format!("/modnet/{path} ")));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                stream.write_all(b"model").unwrap();
            }
        });

        let (mut fetched, release) = tiny_release();
        let mut coreml = fetched.manifest.artifacts[0].clone();
        coreml.id = "coreml-fp16".into();
        coreml.format = "coreml-package".into();
        coreml.entrypoint = "tiny.mlpackage".into();
        coreml.files[0].path = "tiny.mlpackage".into();
        fetched.manifest.artifacts.push(coreml);
        let mut route = fetched.manifest.routes[0].clone();
        route.id = "coreml-test".into();
        route.artifact = "coreml-fp16".into();
        route.backend = crate::models::spec::Backend::Coreml;
        fetched.manifest.routes.push(route);
        fetched.manifest.validate().unwrap();
        fetched.bytes = serde_json::to_vec(&fetched.manifest).unwrap();

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let models_root = env::temp_dir().join(format!(
            "valle-model-store-artifact-slots-test-{}-{unique}",
            std::process::id()
        ));
        let hub = HubClient::new(&format!("http://{address}"), None).unwrap();
        let onnx = hub
            .install_artifact(&release, &fetched, "onnx-fp32", &models_root)
            .unwrap();
        let coreml = hub
            .install_artifact(&release, &fetched, "coreml-fp16", &models_root)
            .unwrap();
        server.join().unwrap();

        assert_ne!(onnx.root, coreml.root);
        assert!(onnx.root.join("tiny.onnx").is_file());
        assert!(!onnx.root.join("tiny.mlpackage").exists());
        assert!(coreml.root.join("tiny.mlpackage").is_file());
        assert!(!coreml.root.join("tiny.onnx").exists());
        let onnx_lock: InstallLock =
            serde_json::from_slice(&fs::read(onnx.root.join(INSTALL_LOCK_PATH)).unwrap()).unwrap();
        let coreml_lock: InstallLock =
            serde_json::from_slice(&fs::read(coreml.root.join(INSTALL_LOCK_PATH)).unwrap())
                .unwrap();
        assert_eq!(onnx_lock.artifact, "onnx-fp32");
        assert_eq!(coreml_lock.artifact, "coreml-fp16");
        fs::remove_dir_all(models_root).unwrap();
    }

    #[test]
    fn integrity_failure_removes_partial_and_does_not_write_install_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (path, body) in [
                ("LICENSE", b"model".as_slice()),
                ("tiny.onnx", b"wrong".as_slice()),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                assert!(request.starts_with(&format!(
                    "GET /valle/modnet/resolve/0123456789012345678901234567890123456789/modnet/{path} "
                )));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let (fetched, release) = tiny_release();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache = env::temp_dir().join(format!(
            "valle-model-store-integrity-test-{}-{unique}",
            std::process::id()
        ));
        let hub = HubClient::new(&format!("http://{address}"), None).unwrap();
        let error = hub
            .install_artifact(&release, &fetched, "onnx-fp32", &cache)
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("integrity check failed"));
        let root = artifact_install_root(&cache);
        assert!(!root.exists());
        assert!(staging_slots(&root).is_empty());
        fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn truncated_response_removes_partial_and_does_not_write_install_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut license_stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let count = license_stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.contains("/modnet/LICENSE "));
            write!(
                license_stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            license_stream.write_all(b"model").unwrap();
            drop(license_stream);

            let (mut artifact_stream, _) = listener.accept().unwrap();
            let mut artifact_request = [0_u8; 4096];
            let count = artifact_stream.read(&mut artifact_request).unwrap();
            let request = String::from_utf8_lossy(&artifact_request[..count]);
            assert!(request.contains("/modnet/tiny.onnx "));
            write!(
                artifact_stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            artifact_stream.write_all(b"truncated").unwrap();
            artifact_stream.flush().unwrap();
            artifact_stream.shutdown(Shutdown::Write).unwrap();
        });

        let (fetched, release) = tiny_release();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache = env::temp_dir().join(format!(
            "valle-model-store-truncated-test-{}-{unique}",
            std::process::id()
        ));
        let hub = HubClient::new(&format!("http://{address}"), None).unwrap();
        let error = hub
            .install_artifact(&release, &fetched, "onnx-fp32", &cache)
            .unwrap_err();
        server.join().unwrap();
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("failed while downloading")),
            "unexpected error: {error:#}"
        );

        let root = artifact_install_root(&cache);
        assert!(!root.exists());
        assert!(staging_slots(&root).is_empty());
        fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn partial_downloads_are_unique_within_one_process() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "valle-model-store-partial-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("tiny.onnx");

        let (first_path, first_file) = create_download_partial(&target).unwrap();
        let (second_path, second_file) = create_download_partial(&target).unwrap();
        assert_ne!(first_path, second_path);
        assert!(first_path.is_file());
        assert!(second_path.is_file());

        drop((first_file, second_file));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_id_is_part_of_the_immutable_install_slot() {
        let base = ArtifactKey {
            model: "modnet".into(),
            version: "1.0.0".into(),
            revision: "0123456789012345678901234567890123456789".into(),
            artifact: "onnx-fp32".into(),
        };
        let mut coreml = base.clone();
        coreml.artifact = "coreml-fp16".into();

        let models_root = Path::new("cache/models");
        assert_ne!(
            base.install_root(models_root).unwrap(),
            coreml.install_root(models_root).unwrap()
        );
        assert!(
            base.install_root(models_root)
                .unwrap()
                .ends_with("modnet/1.0.0/0123456789012345678901234567890123456789/onnx-fp32")
        );
    }

    fn staging_slots(root: &Path) -> Vec<PathBuf> {
        let prefix = staging_slot_prefix(root).unwrap();
        fs::read_dir(root.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .map(|entry| entry.path())
            .collect()
    }

    fn unique_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "valle-model-store-{label}-test-{}-{unique}",
            std::process::id()
        ))
    }

    fn artifact_install_root(models_root: &Path) -> PathBuf {
        models_root
            .join("modnet/1.0.0")
            .join("0123456789012345678901234567890123456789")
            .join("onnx-fp32")
    }

    fn expected_test_lock(fetched: &FetchedManifest, release: &CatalogRelease) -> InstallLock {
        InstallLock {
            schema_version: 1,
            model: fetched.manifest.model.id.clone(),
            version: fetched.manifest.model.version.clone(),
            repository: release.repository.clone(),
            revision: release.revision.clone(),
            manifest_path: release.manifest_path.clone(),
            manifest_sha256: sha256_bytes(&fetched.bytes),
            artifact: "onnx-fp32".into(),
            license_files: fetched.manifest.license.files.clone(),
            files: fetched.manifest.artifacts[0].files.clone(),
        }
    }

    fn test_artifact_key(fetched: &FetchedManifest, release: &CatalogRelease) -> ArtifactKey {
        ArtifactKey {
            model: fetched.manifest.model.id.clone(),
            version: fetched.manifest.model.version.clone(),
            revision: release.revision.clone(),
            artifact: fetched.manifest.artifacts[0].id.clone(),
        }
    }

    fn write_test_install_slot(models_root: &Path) -> PathBuf {
        let (fetched, release) = tiny_release();
        let key = test_artifact_key(&fetched, &release);
        let slot = key.install_root(models_root).unwrap();
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join("LICENSE"), b"model").unwrap();
        fs::write(slot.join("tiny.onnx"), b"model").unwrap();
        fs::write(slot.join(INSTALLED_MANIFEST_PATH), &fetched.bytes).unwrap();
        fs::write(
            slot.join(INSTALL_LOCK_PATH),
            serde_json::to_vec_pretty(&expected_test_lock(&fetched, &release)).unwrap(),
        )
        .unwrap();
        slot
    }

    fn read_http_request(stream: &mut impl Read, expected_path: &str) {
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(
            request.contains(expected_path),
            "unexpected request: {request}"
        );
    }

    fn write_model_response(stream: &mut impl Write) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.write_all(b"model").unwrap();
    }

    fn assert_child_succeeded(label: &str, output: &Output) {
        assert!(
            output.status.success(),
            "{label} child failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn tiny_release() -> (FetchedManifest, CatalogRelease) {
        let mut manifest: ModelManifest =
            serde_json::from_str(include_str!("catalog/release-modnet-1.0.0.json")).unwrap();
        manifest.release.state = ReleaseReadiness::Ready;
        manifest.release.repository = Some("valle/modnet".into());
        manifest.license.files = vec![ArtifactFile {
            path: "LICENSE".into(),
            bytes: 5,
            sha256: sha256_bytes(b"model"),
        }];
        manifest.artifacts.truncate(1);
        manifest.artifacts[0].entrypoint = "tiny.onnx".into();
        manifest.artifacts[0].files = vec![ArtifactFile {
            path: "tiny.onnx".into(),
            bytes: 5,
            sha256: sha256_bytes(b"model"),
        }];
        manifest
            .routes
            .retain(|route| route.artifact == "onnx-fp32");
        manifest.validate().unwrap();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        (
            FetchedManifest { manifest, bytes },
            CatalogRelease {
                version: "1.0.0".into(),
                repository: "valle/modnet".into(),
                revision: "0123456789012345678901234567890123456789".into(),
                manifest_path: "modnet/release.v1.json".into(),
            },
        )
    }
}
