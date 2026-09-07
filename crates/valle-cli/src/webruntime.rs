//! Pack, verify, install, and resolve web runtime assets separately from the CLI binary. An
//! explicit directory takes precedence over the versioned cache. Manifests define the complete
//! asset allowlist, hashes, and protocol version; reject missing or inconsistent assets and report
//! recovery commands. Never upgrade implicitly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Shared Rust/Web runtime handshake version.
pub const PROTOCOL_VERSION: u32 =
    parse_protocol_version(include_str!("../../../web/runtime-protocol-version.txt"));
/// Runtime and engine versions are released together.
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Dependency metadata shares the Web package's exact CanvasKit pin.
fn canvaskit_version() -> String {
    let package: serde_json::Value = serde_json::from_str(include_str!(
        "../../../web/packages/player-core/package.json"
    ))
    .expect("checked-in player-core package.json must be valid JSON");
    package["dependencies"]["canvaskit-wasm"]
        .as_str()
        .expect("player-core must pin canvaskit-wasm")
        .to_owned()
}
/// Aggregate manifest relative to the web build root.
pub const BUILD_MANIFEST_FILE: &str = "runtime/manifest.json";
pub const ENGINE_GLUE_PATH: &str = "runtime/engine/valle_engine.js";
pub const ENGINE_WASM_PATH: &str = "runtime/engine/valle_engine_bg.wasm";
pub const CANVASKIT_BASE_GLUE_PATH: &str = "runtime/canvaskit/base/canvaskit.js";
pub const CANVASKIT_BASE_WASM_PATH: &str = "runtime/canvaskit/base/canvaskit.wasm";
pub const CANVASKIT_FULL_GLUE_PATH: &str = "runtime/canvaskit/full/canvaskit.js";
pub const CANVASKIT_FULL_WASM_PATH: &str = "runtime/canvaskit/full/canvaskit.wasm";
pub const DEFAULT_SANS_FONT_PATH: &str = "runtime/fonts/NotoSans-Regular.ttf";
pub const PRODUCT_FRAME_WORKER_PATH: &str = "runtime/workers/product-frame.js";

/// Rust hosts and the admitted root manifest use one canonical runtime URL map.
pub fn runtime_assets_json() -> serde_json::Value {
    serde_json::json!({
        "engine": { "glue": ENGINE_GLUE_PATH, "wasm": ENGINE_WASM_PATH },
        "canvasKit": {
            "base": { "glue": CANVASKIT_BASE_GLUE_PATH, "wasm": CANVASKIT_BASE_WASM_PATH },
            "full": { "glue": CANVASKIT_FULL_GLUE_PATH, "wasm": CANVASKIT_FULL_WASM_PATH }
        },
        "fonts": { "defaultSans": DEFAULT_SANS_FONT_PATH },
        "workers": { "productFrame": PRODUCT_FRAME_WORKER_PATH }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeManifest {
    pub name: String,
    pub runtime_version: String,
    pub protocol_version: u32,
    pub canvaskit_version: String,
    /// Sort by path for byte-deterministic manifests.
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildManifest {
    schema_version: u32,
    runtime_version: String,
    protocol_version: u32,
    packages: Vec<BuildPackageRef>,
    assets: Vec<BuildManifestFile>,
    asset_groups: Vec<BuildAssetGroup>,
    workers: Vec<BuildEntryRef>,
    apps: Vec<BuildAppRef>,
    routes: BTreeMap<String, String>,
    runtime_assets: BuildRuntimeAssets,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildRuntimeAssets {
    engine: BuildGlueWasmBinding,
    canvas_kit: BuildCanvasKitBindings,
    fonts: BuildFontBindings,
    workers: BuildWorkerBindings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildGlueWasmBinding {
    glue: String,
    wasm: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildCanvasKitBindings {
    base: BuildGlueWasmBinding,
    full: BuildGlueWasmBinding,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildFontBindings {
    default_sans: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildWorkerBindings {
    product_frame: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildManifestFile {
    id: String,
    owner: String,
    role: String,
    path: String,
    sha256: String,
    bytes: u64,
    license: String,
    group: Option<String>,
    #[serde(rename = "source")]
    _source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildPackageRef {
    id: String,
    manifest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildAssetGroup {
    id: String,
    glue: String,
    wasm: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildEntryRef {
    #[serde(rename = "id")]
    _id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildAppRef {
    #[serde(rename = "id")]
    _id: String,
    html: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildPackageManifest {
    schema_version: u32,
    package: String,
    assets: Vec<BuildManifestFile>,
}

/// Runtime resolution sources in priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSource {
    /// Explicit runtime directory; validation failure does not fall back to the cache.
    DevDir,
    /// Installed runtime under the versioned user cache.
    Cache,
    Distribution,
}

impl RuntimeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeSource::DevDir => "dev-dir",
            RuntimeSource::Cache => "cache",
            RuntimeSource::Distribution => "distribution",
        }
    }
}

pub struct ResolvedRuntime {
    pub dir: PathBuf,
    pub source: RuntimeSource,
    pub manifest: RuntimeManifest,
    /// Map public URLs to runtime files within the build tree.
    aliases: BTreeMap<String, String>,
    /// Bytes verified and frozen during resolution; never reopen live runtime paths when serving.
    frozen_files: BTreeMap<String, Arc<[u8]>>,
}

impl std::fmt::Debug for ResolvedRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRuntime")
            .field("dir", &self.dir)
            .field("source", &self.source)
            .field("file_count", &self.frozen_files.len())
            .finish()
    }
}

/// Verified manifest and asset bytes captured once. Staging consumes this snapshot to prevent
/// source replacement between verification and publication.
#[derive(Debug, PartialEq, Eq)]
struct VerifiedBuild {
    manifest: RuntimeManifest,
    aliases: BTreeMap<String, String>,
    files: Vec<FrozenRuntimeFile>,
}

#[derive(Debug, PartialEq, Eq)]
struct FrozenRuntimeFile {
    relative: String,
    bytes: Vec<u8>,
}

/// Serve runtime assets from verified bytes and project assets from explicit local paths.
#[derive(Debug, Clone)]
pub enum HostedFile {
    VerifiedRuntime(Arc<[u8]>),
    LocalPath(PathBuf),
}

struct PublishLock(File);

impl Drop for PublishLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

struct StagingDir(PathBuf);

impl StagingDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl ResolvedRuntime {
    /// Map manifest assets and public routes to hosted content.
    pub fn serving_map(&self) -> BTreeMap<String, HostedFile> {
        let mut files: BTreeMap<_, _> = self
            .manifest
            .files
            .iter()
            .map(|file| {
                let bytes = self
                    .frozen_files
                    .get(&file.path)
                    .expect("verified runtime must freeze every manifest asset");
                (
                    file.path.clone(),
                    HostedFile::VerifiedRuntime(Arc::clone(bytes)),
                )
            })
            .collect();
        for (route, target) in &self.aliases {
            let bytes = self
                .frozen_files
                .get(target)
                .expect("verified route target must be a frozen manifest asset");
            files.insert(
                route.clone(),
                HostedFile::VerifiedRuntime(Arc::clone(bytes)),
            );
        }
        files
    }
}

/// Package the verified web/dist manifest and all referenced assets.
pub fn pack(source: &Path, out: &Path) -> Result<RuntimeManifest> {
    if !source.join(BUILD_MANIFEST_FILE).is_file() {
        bail!(
            "{} has no {}; build it first with `cd web && bun run build`",
            source.display(),
            BUILD_MANIFEST_FILE
        );
    }
    let verified = verify_build(source)
        .with_context(|| format!("verifying web build at {}", source.display()))?;
    let parent = publish_parent(out)?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let _lock = lock_publish_target(out)?;
    recover_interrupted_publish(out)?;
    ensure_empty_or_missing_output(out)?;
    let staging = unique_staging_dir(parent, "pack")?;
    write_verified_build(&verified, staging.path())?;
    verify_staged_build(staging.path(), &verified)?;
    replace_installed_runtime(staging.path(), out)?;
    Ok(verified.manifest)
}

/// Verify the full asset graph rooted at dist/runtime/manifest.json.
fn verify_build(dir: &Path) -> Result<VerifiedBuild> {
    let root = std::fs::canonicalize(dir)
        .with_context(|| format!("canonicalizing web runtime root {}", dir.display()))?;
    if !std::fs::metadata(&root)?.is_dir() {
        bail!("web runtime root is not a directory: {}", dir.display());
    }
    let manifest_path = root.join(BUILD_MANIFEST_FILE);
    let root_bytes = read_regular_runtime_file(&root, BUILD_MANIFEST_FILE)?;
    let build: BuildManifest = serde_json::from_slice(&root_bytes)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    if build.schema_version != 1 {
        bail!(
            "unsupported web runtime manifest schema {}",
            build.schema_version
        );
    }
    ensure_safe_runtime_version(&build.runtime_version)?;
    if build.protocol_version != PROTOCOL_VERSION {
        bail!(
            "web runtime protocol mismatch: bundle is {}, host requires {PROTOCOL_VERSION}",
            build.protocol_version
        );
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeMap::new();
    let mut assets_by_id = BTreeMap::new();
    let mut frozen_files = Vec::new();
    for file in &build.assets {
        ensure_safe_runtime_path(&file.path)?;
        ensure_not_host_reserved_route(&file.path)?;
        if !ids.insert(file.id.clone()) || assets_by_id.insert(file.id.clone(), file).is_some() {
            bail!("duplicate web runtime asset id '{}'", file.id);
        }
        if paths.insert(file.path.clone(), file).is_some() {
            bail!("duplicate web runtime asset path '{}'", file.path);
        }
        if file.owner.is_empty() || file.license.is_empty() {
            bail!(
                "web runtime asset '{}' requires owner and license",
                file.path
            );
        }
        let asset = root.join(&file.path);
        let data = read_regular_runtime_file(&root, &file.path)?;
        if data.len() as u64 != file.bytes || sha256_hex(&data) != file.sha256 {
            bail!(
                "web runtime file corrupted: {} (sha256 mismatch)",
                asset.display()
            );
        }
        frozen_files.push(FrozenRuntimeFile {
            relative: file.path.clone(),
            bytes: data,
        });
    }
    let mut group_ids = BTreeSet::new();
    let mut grouped_paths = BTreeSet::new();
    for group in &build.asset_groups {
        if group.id.is_empty() || !group_ids.insert(group.id.clone()) {
            bail!(
                "duplicate or empty web runtime asset group id '{}'",
                group.id
            );
        }
        if group.wasm.is_empty() {
            bail!("web runtime asset group '{}' has no wasm", group.id);
        }
        let mut members = BTreeSet::new();
        for member in std::iter::once(&group.glue).chain(group.wasm.iter()) {
            if !members.insert(member) {
                bail!(
                    "web runtime asset group '{}' repeats member '{}'",
                    group.id,
                    member
                );
            }
            let asset = paths.get(member).ok_or_else(|| {
                anyhow::anyhow!(
                    "web asset group '{}' member '{}' is outside manifest",
                    group.id,
                    member
                )
            })?;
            if asset.group.as_deref() != Some(group.id.as_str()) {
                bail!("web asset group '{}' does not own '{}'", group.id, member);
            }
            if !grouped_paths.insert(member.clone()) {
                bail!("web runtime asset '{}' appears in multiple groups", member);
            }
        }
    }
    for (path, asset) in &paths {
        if let Some(group) = &asset.group {
            if !group_ids.contains(group) || !grouped_paths.contains(path.as_str()) {
                bail!(
                    "web runtime asset '{}' claims undeclared or incomplete group '{}'",
                    path,
                    group
                );
            }
        }
    }
    let mut worker_ids = BTreeSet::new();
    for worker in &build.workers {
        if worker._id.is_empty() || !worker_ids.insert(worker._id.clone()) {
            bail!("duplicate or empty web worker id '{}'", worker._id);
        }
        if paths.get(&worker.path).map(|asset| asset.role.as_str()) != Some("worker") {
            bail!("web worker '{}' is outside the worker closure", worker.path);
        }
    }
    let mut app_ids = BTreeSet::new();
    for app in &build.apps {
        if app._id.is_empty() || !app_ids.insert(app._id.clone()) {
            bail!("duplicate or empty web app id '{}'", app._id);
        }
        if paths.get(&app.html).map(|asset| asset.role.as_str()) != Some("html") {
            bail!("web app HTML '{}' is outside the html closure", app.html);
        }
    }
    for (route, target) in &build.routes {
        ensure_safe_runtime_path(route)?;
        ensure_not_host_reserved_route(route)?;
        ensure_safe_runtime_path(target)?;
        if !paths.contains_key(target) {
            bail!("web route '{route}' targets asset outside manifest: '{target}'");
        }
        if paths.contains_key(route) && route != target {
            bail!("web route '{route}' would shadow a manifest asset with target '{target}'");
        }
    }
    for (runtime_path, expected_path, expected_role, expected_group) in [
        (
            build.runtime_assets.engine.glue.as_str(),
            ENGINE_GLUE_PATH,
            "glue",
            Some("engine-core"),
        ),
        (
            build.runtime_assets.engine.wasm.as_str(),
            ENGINE_WASM_PATH,
            "wasm",
            Some("engine-core"),
        ),
        (
            build.runtime_assets.canvas_kit.base.glue.as_str(),
            CANVASKIT_BASE_GLUE_PATH,
            "glue",
            Some("canvaskit-base"),
        ),
        (
            build.runtime_assets.canvas_kit.base.wasm.as_str(),
            CANVASKIT_BASE_WASM_PATH,
            "wasm",
            Some("canvaskit-base"),
        ),
        (
            build.runtime_assets.canvas_kit.full.glue.as_str(),
            CANVASKIT_FULL_GLUE_PATH,
            "glue",
            Some("canvaskit-full"),
        ),
        (
            build.runtime_assets.canvas_kit.full.wasm.as_str(),
            CANVASKIT_FULL_WASM_PATH,
            "wasm",
            Some("canvaskit-full"),
        ),
        (
            build.runtime_assets.fonts.default_sans.as_str(),
            DEFAULT_SANS_FONT_PATH,
            "font",
            None,
        ),
        (
            build.runtime_assets.workers.product_frame.as_str(),
            PRODUCT_FRAME_WORKER_PATH,
            "worker",
            None,
        ),
    ] {
        ensure_safe_runtime_path(runtime_path)?;
        if runtime_path != expected_path {
            bail!(
                "web injected runtime URL '{}' must use canonical path '{}'",
                runtime_path,
                expected_path
            );
        }
        let asset = paths.get(runtime_path).ok_or_else(|| {
            anyhow::anyhow!("web injected runtime URL '{runtime_path}' is outside manifest")
        })?;
        if asset.role != expected_role || asset.group.as_deref() != expected_group {
            bail!(
                "web injected runtime URL '{}' requires role '{}' and group {:?}",
                runtime_path,
                expected_role,
                expected_group
            );
        }
    }
    let mut package_ids = BTreeSet::new();
    let mut packaged_ids = BTreeSet::new();
    let mut frozen_paths: BTreeSet<String> = frozen_files
        .iter()
        .map(|file| file.relative.clone())
        .collect();
    for package_ref in &build.packages {
        if package_ref.id.is_empty() || !package_ids.insert(package_ref.id.clone()) {
            bail!(
                "duplicate or empty web runtime package id '{}'",
                package_ref.id
            );
        }
        ensure_safe_runtime_path(&package_ref.manifest)?;
        if package_ref.manifest == BUILD_MANIFEST_FILE
            || !frozen_paths.insert(package_ref.manifest.clone())
        {
            bail!(
                "duplicate web runtime package manifest path '{}'",
                package_ref.manifest
            );
        }
        let package_path = root.join(&package_ref.manifest);
        let package_bytes = read_regular_runtime_file(&root, &package_ref.manifest)?;
        let package: BuildPackageManifest = serde_json::from_slice(&package_bytes)
            .with_context(|| format!("parsing {}", package_path.display()))?;
        if package.schema_version != build.schema_version || package.package != package_ref.id {
            bail!(
                "web package manifest identity drift: {}",
                package_ref.manifest
            );
        }
        if package.assets.is_empty() {
            bail!(
                "web package manifest '{}' has no assets",
                package_ref.manifest
            );
        }
        for asset in package.assets {
            if asset.owner != package.package {
                bail!(
                    "web package manifest '{}' asset '{}' is owned by '{}'",
                    package_ref.manifest,
                    asset.id,
                    asset.owner
                );
            }
            if assets_by_id.get(&asset.id).copied() != Some(&asset) {
                bail!(
                    "web package manifest '{}' asset '{}' drifts from the root manifest",
                    package_ref.manifest,
                    asset.id
                );
            }
            if !packaged_ids.insert(asset.id.clone()) {
                bail!(
                    "web runtime asset '{}' appears in multiple packages",
                    asset.id
                );
            }
        }
        frozen_files.push(FrozenRuntimeFile {
            relative: package_ref.manifest.clone(),
            bytes: package_bytes,
        });
    }
    if packaged_ids != ids {
        bail!("web root assets do not equal the per-package manifest union");
    }
    frozen_files.push(FrozenRuntimeFile {
        relative: BUILD_MANIFEST_FILE.to_owned(),
        bytes: root_bytes,
    });
    Ok(VerifiedBuild {
        manifest: RuntimeManifest {
            name: "valle-web-runtime".to_owned(),
            runtime_version: build.runtime_version,
            protocol_version: build.protocol_version,
            canvaskit_version: canvaskit_version(),
            files: build
                .assets
                .into_iter()
                .map(|file| ManifestFile {
                    path: file.path,
                    sha256: file.sha256,
                    bytes: file.bytes,
                })
                .collect(),
        },
        aliases: build.routes,
        files: frozen_files,
    })
}

fn ensure_safe_runtime_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.contains('\\')
        || value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        || path.components().any(|part| {
            matches!(
                part,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        bail!("unsafe web runtime path '{value}'");
    }
    Ok(())
}

fn ensure_not_host_reserved_route(value: &str) -> Result<()> {
    const EXACT: &[&str] = &[
        "result",
        "library/execute",
        "timeline/edit",
        "events",
        "smoke-report",
        "studio",
        "studio/boot.json",
        "timeline/get",
        "config.json",
    ];
    const PREFIXES: &[&str] = &["assets/", "library/blob/", "library/thumb/"];
    if EXACT.contains(&value) || PREFIXES.iter().any(|prefix| value.starts_with(prefix)) {
        bail!("web runtime path conflicts with a host route: '{value}'");
    }
    Ok(())
}

fn read_regular_runtime_file(root: &Path, relative: &str) -> Result<Vec<u8>> {
    ensure_safe_runtime_path(relative)?;
    let relative_path = Path::new(relative);
    let component_count = relative_path.components().count();
    let mut current = root.to_path_buf();
    for (index, component) in relative_path.components().enumerate() {
        let Component::Normal(component) = component else {
            bail!("unsafe web runtime path '{relative}'");
        };
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)
            .with_context(|| format!("inspecting web runtime file {}", current.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("web runtime path contains a symlink: {}", current.display());
        }
        if index + 1 == component_count {
            if !metadata.file_type().is_file() {
                bail!(
                    "web runtime asset is not a regular file: {}",
                    current.display()
                );
            }
        } else if !metadata.file_type().is_dir() {
            bail!(
                "web runtime path parent is not a directory: {}",
                current.display()
            );
        }
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(&current)
        .with_context(|| format!("opening web runtime file {}", current.display()))?;
    if !file.metadata()?.file_type().is_file() {
        bail!(
            "web runtime asset is not a regular file: {}",
            current.display()
        );
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("reading web runtime file {}", current.display()))?;
    Ok(bytes)
}

/// Require a safe single path component in Cargo SemVer form. Reject aliases such as v1.2.3 to keep
/// one cache identity per version.
fn ensure_safe_runtime_version(value: &str) -> Result<()> {
    let path = Path::new(value);
    let mut components = path.components();
    let is_single_normal_component = matches!(components.next(), Some(Component::Normal(part)) if part == value)
        && components.next().is_none();
    if !is_single_normal_component
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
        || !is_semver(value)
    {
        bail!("unsafe web runtime version '{value}'");
    }
    Ok(())
}

fn is_semver(value: &str) -> bool {
    let (without_build, build) = match value.split_once('+') {
        Some((left, right)) if !right.contains('+') => (left, Some(right)),
        Some(_) => return false,
        None => (value, None),
    };
    let (core, prerelease) = match without_build.split_once('-') {
        Some((left, right)) => (left, Some(right)),
        None => (without_build, None),
    };
    let mut core_parts = core.split('.');
    if !(is_semver_number(core_parts.next())
        && is_semver_number(core_parts.next())
        && is_semver_number(core_parts.next())
        && core_parts.next().is_none())
    {
        return false;
    }
    prerelease.is_none_or(|part| is_semver_identifiers(part, true))
        && build.is_none_or(|part| is_semver_identifiers(part, false))
}

fn is_semver_number(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && (value == "0" || !value.starts_with('0'))
    })
}

fn is_semver_identifiers(value: &str, reject_numeric_leading_zero: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!reject_numeric_leading_zero
                    || !identifier.bytes().all(|byte| byte.is_ascii_digit())
                    || identifier == "0"
                    || !identifier.starts_with('0'))
        })
}

const fn parse_protocol_version(source: &str) -> u32 {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut value = 0_u32;
    let mut saw_digit = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte >= b'0' && byte <= b'9' {
            saw_digit = true;
            value = match value.checked_mul(10) {
                Some(value) => value,
                None => panic!("runtime protocol version overflows u32"),
            };
            value = match value.checked_add((byte - b'0') as u32) {
                Some(value) => value,
                None => panic!("runtime protocol version overflows u32"),
            };
        } else if byte == b'\n' && index + 1 == bytes.len() {
            break;
        } else {
            panic!("runtime protocol version must be decimal digits with an optional newline");
        }
        index += 1;
    }
    if !saw_digit {
        panic!("runtime protocol version must not be empty");
    }
    value
}

fn runtime_slot(cache_root: &Path, runtime_version: &str) -> Result<PathBuf> {
    ensure_safe_runtime_version(runtime_version)?;
    Ok(cache_root.join(runtime_version))
}

/// Verify and install a packed bundle into its version slot using staging and atomic rename.
pub fn install(from: &Path, cache_root: &Path) -> Result<PathBuf> {
    let verified = verify_build(from)
        .with_context(|| format!("verifying web bundle at {}", from.display()))?;
    let target = runtime_slot(cache_root, &verified.manifest.runtime_version)?;
    std::fs::create_dir_all(cache_root)
        .with_context(|| format!("creating {}", cache_root.display()))?;
    let _lock = lock_publish_target(&target)?;
    recover_interrupted_publish(&target)?;
    if path_exists(&target)? {
        if let Ok(installed) = verify_build(&target) {
            if installed == verified {
                return Ok(target);
            }
            bail!(
                "web runtime version collision: {} already contains a different verified bundle",
                verified.manifest.runtime_version
            );
        }
    }
    let staging = unique_staging_dir(cache_root, &verified.manifest.runtime_version)?;
    write_verified_build(&verified, staging.path())?;
    verify_staged_build(staging.path(), &verified)?;
    replace_installed_runtime(staging.path(), &target)?;
    Ok(target)
}

fn publish_parent(target: &Path) -> Result<&Path> {
    if target.file_name().is_none() {
        bail!("web runtime output has no file name: {}", target.display());
    }
    target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .or(Some(Path::new(".")))
        .ok_or_else(|| anyhow::anyhow!("web runtime output has no parent: {}", target.display()))
}

fn lock_publish_target(target: &Path) -> Result<PublishLock> {
    let parent = publish_parent(target)?;
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing publish parent {}", parent.display()))?;
    let identity_path = canonical_parent.join(
        target
            .file_name()
            .expect("publish_parent rejected paths without a file name"),
    );
    let identity = sha256_hex(identity_path.as_os_str().to_string_lossy().as_bytes());
    let lock_path = parent.join(format!(".valle-runtime-publish-{identity}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening web runtime publish lock {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("locking web runtime publish target {}", target.display()))?;
    Ok(PublishLock(file))
}

fn publish_target_identity(target: &Path) -> Result<String> {
    let parent = publish_parent(target)?;
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing publish parent {}", parent.display()))?;
    let identity_path = canonical_parent.join(
        target
            .file_name()
            .expect("publish_parent rejected paths without a file name"),
    );
    Ok(sha256_hex(
        identity_path.as_os_str().to_string_lossy().as_bytes(),
    ))
}

fn backup_path(target: &Path) -> Result<PathBuf> {
    Ok(publish_parent(target)?.join(format!(
        ".valle-runtime-backup-{}",
        publish_target_identity(target)?
    )))
}

fn path_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn ensure_empty_or_missing_output(out: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(out) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", out.display())),
    };
    if !metadata.file_type().is_dir() {
        bail!("web runtime output is not a directory: {}", out.display());
    }
    if std::fs::read_dir(out)
        .with_context(|| format!("reading {}", out.display()))?
        .next()
        .is_some()
    {
        bail!(
            "web runtime output must be empty or absent: {}",
            out.display()
        );
    }
    Ok(())
}

fn unique_staging_dir(parent: &Path, tag: &str) -> Result<StagingDir> {
    ensure_safe_runtime_path(tag)?;
    for _ in 0..128 {
        let path = unique_sibling_path(parent, &format!("staging-{tag}"));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(StagingDir(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
    bail!("could not allocate a unique web runtime staging directory")
}

fn unique_sibling_path(parent: &Path, tag: &str) -> PathBuf {
    static NEXT_UNIQUE: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_UNIQUE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    parent.join(format!(
        ".valle-{tag}-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

fn replace_installed_runtime(staging: &Path, target: &Path) -> Result<()> {
    recover_interrupted_publish(target)?;
    let parent = publish_parent(target)?;
    if !path_exists(target)? {
        std::fs::rename(staging, target)
            .with_context(|| format!("moving {} → {}", staging.display(), target.display()))?;
        sync_directory(parent)?;
        return Ok(());
    }
    if !std::fs::symlink_metadata(target)?.file_type().is_dir() {
        bail!(
            "web runtime target is not a directory: {}",
            target.display()
        );
    }
    let backup = backup_path(target)?;
    if path_exists(&backup)? {
        bail!(
            "web runtime backup still exists after recovery: {}",
            backup.display()
        );
    }
    std::fs::rename(target, &backup).with_context(|| {
        format!(
            "moving last-good web runtime {} → {}",
            target.display(),
            backup.display()
        )
    })?;
    sync_directory(parent)?;
    if let Err(publish_error) = std::fs::rename(staging, target) {
        if let Err(rollback_error) = std::fs::rename(&backup, target) {
            return Err(anyhow::anyhow!(publish_error)).context(format!(
                "publishing {} failed and restoring last-good {} also failed: {rollback_error}",
                staging.display(),
                target.display()
            ));
        }
        sync_directory(parent).with_context(|| {
            format!(
                "restored last-good {} but failed to sync its parent",
                target.display()
            )
        })?;
        return Err(anyhow::anyhow!(publish_error)).with_context(|| {
            format!(
                "publishing {} failed; restored last-good {}",
                staging.display(),
                target.display()
            )
        });
    }
    sync_directory(parent)?;
    remove_runtime_tree(&backup)?;
    sync_directory(parent)?;
    Ok(())
}

/// Recover interrupted publication under the target lock. The backup name uniquely identifies the
/// publication target.
fn recover_interrupted_publish(target: &Path) -> Result<()> {
    let backup = backup_path(target)?;
    let target_exists = path_exists(target)?;
    let backup_exists = path_exists(&backup)?;
    if !backup_exists {
        return Ok(());
    }
    if !std::fs::symlink_metadata(&backup)?.file_type().is_dir() {
        bail!(
            "web runtime recovery backup is not a directory: {}",
            backup.display()
        );
    }
    let parent = publish_parent(target)?;
    if !target_exists {
        std::fs::rename(&backup, target).with_context(|| {
            format!(
                "restoring interrupted web runtime publish {} → {}",
                backup.display(),
                target.display()
            )
        })?;
        sync_directory(parent)?;
        return Ok(());
    }
    if !std::fs::symlink_metadata(target)?.file_type().is_dir() {
        bail!(
            "web runtime target is not a directory during recovery: {}",
            target.display()
        );
    }

    if let Ok(target_build) = verify_build(target) {
        if let Ok(backup_build) = verify_build(&backup) {
            if target_build != backup_build {
                bail!(
                    "ambiguous web runtime recovery: target and backup are different verified bundles ({}, {})",
                    target.display(),
                    backup.display()
                );
            }
        }
        remove_runtime_tree(&backup)?;
        sync_directory(parent)?;
        return Ok(());
    }

    // A published-but-invalid target cannot displace the durable last-good backup.
    remove_runtime_tree(target)?;
    sync_directory(parent)?;
    std::fs::rename(&backup, target).with_context(|| {
        format!(
            "restoring last-good web runtime {} → {}",
            backup.display(),
            target.display()
        )
    })?;
    sync_directory(parent)?;
    Ok(())
}

fn remove_runtime_tree(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting web runtime tree {}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "refusing to remove non-directory runtime tree: {}",
            path.display()
        );
    }
    std::fs::remove_dir_all(path)
        .with_context(|| format!("removing web runtime tree {}", path.display()))
}

fn write_verified_build(verified: &VerifiedBuild, to: &Path) -> Result<()> {
    for file in &verified.files {
        write_frozen_runtime_file(to, &file.relative, &file.bytes)?;
    }
    sync_directory_tree(to)?;
    Ok(())
}

fn write_frozen_runtime_file(to: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    ensure_safe_runtime_path(relative)?;
    let dst = to.join(relative);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dst)
        .with_context(|| format!("creating frozen web runtime file {}", dst.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing frozen web runtime file {}", dst.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing frozen web runtime file {}", dst.display()))?;
    Ok(())
}

fn verify_staged_build(staging: &Path, expected: &VerifiedBuild) -> Result<()> {
    let actual = verify_build(staging)
        .with_context(|| format!("verifying staged web runtime {}", staging.display()))?;
    if &actual != expected {
        bail!("staged web runtime identity drifted before publish")
    }
    Ok(())
}

fn sync_directory_tree(root: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(root).with_context(|| format!("reading directory {}", root.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "web runtime staging contains a symlink: {}",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            sync_directory_tree(&entry.path())?;
        }
    }
    sync_directory(root)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .with_context(|| format!("opening directory {} for sync", path.display()))?
            .sync_all()
            .with_context(|| format!("syncing directory {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Resolve explicit development assets or resources shipped beside the executable.
pub fn resolve(explicit: Option<&Path>, _cache_root: &Path) -> Result<ResolvedRuntime> {
    let (dir, source) = if let Some(dir) = explicit {
        (dir.to_path_buf(), RuntimeSource::DevDir)
    } else {
        let exe = std::env::current_exe().context("locating Valle executable")?;
        let shipped = exe
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("share/valle/web"));
        if let Some(dir) = shipped.filter(|dir| dir.is_dir()) {
            (dir, RuntimeSource::Distribution)
        } else {
            let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
            // Repository fallback is only for a binary built into this checkout's target tree.
            let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
            let in_checkout = target
                .canonicalize()
                .ok()
                .is_some_and(|target| exe.starts_with(target));
            if in_checkout && dev.is_dir() {
                (dev, RuntimeSource::DevDir)
            } else {
                bail!(
                    "Studio resources are missing. Build the distribution with `cargo xtask build`; for development use --web-assets-dir with a built web/dist directory"
                );
            }
        }
    };
    if !dir.join(BUILD_MANIFEST_FILE).is_file() {
        bail!(
            "Studio resource manifest missing in {}; build Studio resources first",
            dir.display()
        );
    }
    let verified = verify_build(&dir)
        .with_context(|| format!("verifying Studio resources {}", dir.display()))?;
    if verified.manifest.runtime_version != RUNTIME_VERSION {
        bail!(
            "Studio resource version {} does not match CLI {RUNTIME_VERSION}; rebuild them together",
            verified.manifest.runtime_version
        );
    }
    resolved_runtime(dir, source, verified)
}

fn resolved_runtime(
    dir: PathBuf,
    source: RuntimeSource,
    verified: VerifiedBuild,
) -> Result<ResolvedRuntime> {
    let mut all_files = BTreeMap::new();
    for file in verified.files {
        if all_files
            .insert(file.relative.clone(), Arc::<[u8]>::from(file.bytes))
            .is_some()
        {
            bail!("verified web runtime repeats file '{}'", file.relative);
        }
    }
    let mut frozen_files = BTreeMap::new();
    for file in &verified.manifest.files {
        let bytes = all_files.remove(&file.path).ok_or_else(|| {
            anyhow::anyhow!("verified web runtime did not freeze '{}'", file.path)
        })?;
        frozen_files.insert(file.path.clone(), bytes);
    }
    Ok(ResolvedRuntime {
        dir,
        source,
        manifest: verified.manifest,
        aliases: verified.aliases,
        frozen_files,
    })
}

/// Runtime cache under `$VALLE_CACHE_DIR` or `~/.cache/valle`.
pub fn default_cache_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("VALLE_CACHE_DIR") {
        return Ok(PathBuf::from(root).join("web-runtime"));
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow::anyhow!("HOME not set; pass VALLE_CACHE_DIR"))?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("valle")
        .join("web-runtime"))
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        // Use an atomic counter to make temporary paths unique across concurrent tests.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("{prefix}_{}_{nanos}_{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_runtime() -> PathBuf {
        let dist = temp_dir("valle_webruntime_source");
        let html_path = "apps/preview/index.html";
        let chunk_path = "apps/preview/chunk-test.js";
        let specs: [(&str, &str, &str, &[u8], Option<&str>); 10] = [
            (
                "html",
                "html",
                html_path,
                b"<script src=\"./chunk-test.js\"></script>",
                None,
            ),
            ("chunk", "app", chunk_path, b"console.log('runtime')", None),
            (
                "engine-glue",
                "glue",
                ENGINE_GLUE_PATH,
                b"engine glue",
                Some("engine-core"),
            ),
            (
                "engine-wasm",
                "wasm",
                ENGINE_WASM_PATH,
                b"engine wasm",
                Some("engine-core"),
            ),
            (
                "canvas-base-glue",
                "glue",
                CANVASKIT_BASE_GLUE_PATH,
                b"canvas base glue",
                Some("canvaskit-base"),
            ),
            (
                "canvas-base-wasm",
                "wasm",
                CANVASKIT_BASE_WASM_PATH,
                b"canvas base wasm",
                Some("canvaskit-base"),
            ),
            (
                "canvas-full-glue",
                "glue",
                CANVASKIT_FULL_GLUE_PATH,
                b"canvas full glue",
                Some("canvaskit-full"),
            ),
            (
                "canvas-full-wasm",
                "wasm",
                CANVASKIT_FULL_WASM_PATH,
                b"canvas full wasm",
                Some("canvaskit-full"),
            ),
            ("font", "font", DEFAULT_SANS_FONT_PATH, b"font", None),
            (
                "worker",
                "worker",
                PRODUCT_FRAME_WORKER_PATH,
                b"worker",
                None,
            ),
        ];
        let mut assets = Vec::new();
        for (id, role, name, bytes, group) in specs {
            let target = dist.join(name);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
            let mut asset = serde_json::json!({
                "id": id,
                "owner": "fixture",
                "role": role,
                "path": name,
                "bytes": bytes.len(),
                "sha256": sha256_hex(bytes),
                "license": "Apache-2.0"
            });
            if let Some(group) = group {
                asset["group"] = serde_json::json!(group);
            }
            assets.push(asset);
        }
        let package_path = dist.join("runtime/manifests/fixture.json");
        std::fs::create_dir_all(package_path.parent().unwrap()).unwrap();
        std::fs::write(
            package_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schemaVersion": 1,
                "package": "fixture",
                "assets": assets.clone(),
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "runtimeVersion": RUNTIME_VERSION,
            "protocolVersion": PROTOCOL_VERSION,
            "packages": [{ "id": "fixture", "manifest": "runtime/manifests/fixture.json" }],
            "assets": assets,
            "assetGroups": [
                { "id": "engine-core", "glue": ENGINE_GLUE_PATH, "wasm": [ENGINE_WASM_PATH] },
                { "id": "canvaskit-base", "glue": CANVASKIT_BASE_GLUE_PATH, "wasm": [CANVASKIT_BASE_WASM_PATH] },
                { "id": "canvaskit-full", "glue": CANVASKIT_FULL_GLUE_PATH, "wasm": [CANVASKIT_FULL_WASM_PATH] },
            ],
            "workers": [{ "id": "product-frame", "path": PRODUCT_FRAME_WORKER_PATH }],
            "apps": [{ "id": "preview", "html": html_path }],
            "routes": { "preview.html": html_path, "chunk-test.js": chunk_path },
            "runtimeAssets": runtime_assets_json(),
        });
        let manifest_path = dist.join(BUILD_MANIFEST_FILE);
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        std::fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        dist
    }

    fn rewrite_fake_chunk(source: &Path, changed: &[u8]) {
        std::fs::write(source.join("apps/preview/chunk-test.js"), changed).unwrap();
        let root_path = source.join(BUILD_MANIFEST_FILE);
        let package_path = source.join("runtime/manifests/fixture.json");
        for path in [&root_path, &package_path] {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let asset = value["assets"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|asset| asset["id"] == "chunk")
                .unwrap();
            asset["bytes"] = serde_json::json!(changed.len());
            asset["sha256"] = serde_json::json!(sha256_hex(changed));
            std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
    }

    fn spawn_install_helper(source: &Path, cache_root: &Path) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("webruntime::tests::install_process_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env("VALLE_WEBRUNTIME_TEST_INSTALL_SOURCE", source)
            .env("VALLE_WEBRUNTIME_TEST_INSTALL_CACHE", cache_root)
            .spawn()
            .unwrap()
    }

    #[test]
    fn install_process_helper() {
        let Some(source) = std::env::var_os("VALLE_WEBRUNTIME_TEST_INSTALL_SOURCE") else {
            return;
        };
        let cache_root = std::env::var_os("VALLE_WEBRUNTIME_TEST_INSTALL_CACHE").unwrap();
        install(Path::new(&source), Path::new(&cache_root)).unwrap();
    }

    #[test]
    fn manifest_pack_install_and_cache_roundtrip() {
        let source = fake_runtime();
        let bundle = temp_dir("valle_webruntime_bundle");
        let cache_root = temp_dir("valle_webruntime_cache");

        let packed = pack(&source, &bundle).unwrap();
        assert_eq!(packed.name, "valle-web-runtime");
        assert!(bundle.join(BUILD_MANIFEST_FILE).is_file());
        assert_eq!(
            resolve(Some(&bundle), &cache_root).unwrap().source,
            RuntimeSource::DevDir
        );

        install(&bundle, &cache_root).unwrap();
        let cached = resolve(Some(&cache_root.join(RUNTIME_VERSION)), &cache_root).unwrap();
        assert_eq!(cached.source, RuntimeSource::DevDir);
        let serving = cached.serving_map();
        let HostedFile::VerifiedRuntime(demo) = serving.get("preview.html").unwrap() else {
            panic!("runtime route was not frozen at resolve")
        };
        assert_eq!(demo.as_ref(), b"<script src=\"./chunk-test.js\"></script>");

        std::fs::write(cached.dir.join("apps/preview/chunk-test.js"), "tampered").unwrap();
        let HostedFile::VerifiedRuntime(chunk) = serving.get("chunk-test.js").unwrap() else {
            panic!("runtime route was not frozen at resolve")
        };
        assert_eq!(chunk.as_ref(), b"console.log('runtime')");
        let err = resolve(Some(&cache_root.join(RUNTIME_VERSION)), &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("sha256 mismatch"), "{err:#}");
    }

    #[test]
    fn install_is_cross_process_serialized_and_converges_on_one_valid_slot() {
        let source = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_process_lock_cache");
        let target = runtime_slot(&cache_root, RUNTIME_VERSION).unwrap();
        let held_lock = lock_publish_target(&target).unwrap();
        let mut children: Vec<_> = (0..3)
            .map(|_| spawn_install_helper(&source, &cache_root))
            .collect();

        std::thread::sleep(std::time::Duration::from_millis(200));
        for child in &mut children {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child install bypassed the slot lock"
            );
        }
        drop(held_lock);

        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let resolved = resolve(Some(&cache_root.join(RUNTIME_VERSION)), &cache_root).unwrap();
        assert_eq!(resolved.source, RuntimeSource::DevDir);
        let leftovers: Vec<_> = std::fs::read_dir(&cache_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("staging") || name.contains("backup"))
            .collect();
        assert!(leftovers.is_empty(), "stale publish dirs: {leftovers:?}");
    }

    #[test]
    fn verified_copy_closure_does_not_reread_swapped_source() {
        let source = fake_runtime();
        let verified = verify_build(&source).unwrap();
        let original_chunk = std::fs::read(source.join("apps/preview/chunk-test.js")).unwrap();

        std::fs::write(
            source.join("apps/preview/chunk-test.js"),
            b"swapped-after-verify",
        )
        .unwrap();
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let mut swapped_manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        swapped_manifest["engineVersion"] = serde_json::json!("untrusted");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&swapped_manifest).unwrap(),
        )
        .unwrap();

        let staging = temp_dir("valle_webruntime_frozen_staging");
        write_verified_build(&verified, &staging).unwrap();
        verify_staged_build(&staging, &verified).unwrap();
        assert_eq!(
            std::fs::read(staging.join("apps/preview/chunk-test.js")).unwrap(),
            original_chunk
        );
        let staged_root: serde_json::Value =
            serde_json::from_slice(&std::fs::read(staging.join(BUILD_MANIFEST_FILE)).unwrap())
                .unwrap();
        assert!(staged_root.get("engineVersion").is_none());
    }

    #[test]
    fn failed_publish_restores_last_good_target() {
        let parent = temp_dir("valle_webruntime_rollback");
        let target = parent.join("runtime");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("last-good"), b"preserved").unwrap();
        let missing_staging = parent.join("missing-staging");

        let err = replace_installed_runtime(&missing_staging, &target).unwrap_err();
        assert!(format!("{err:#}").contains("restored last-good"), "{err:#}");
        assert_eq!(
            std::fs::read(target.join("last-good")).unwrap(),
            b"preserved"
        );
        assert!(!missing_staging.exists());
    }

    #[test]
    fn pack_rejects_nonempty_output_without_touching_it() {
        let source = fake_runtime();
        let out = temp_dir("valle_webruntime_nonempty_pack");
        std::fs::write(out.join("keep-me"), b"last-good").unwrap();

        let err = pack(&source, &out).unwrap_err();
        assert!(format!("{err:#}").contains("must be empty or absent"));
        assert_eq!(std::fs::read(out.join("keep-me")).unwrap(), b"last-good");
        assert!(!out.join(BUILD_MANIFEST_FILE).exists());
    }

    #[test]
    fn root_manifest_rejects_unsupported_engine_version_field() {
        let source = fake_runtime();
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["engineVersion"] = serde_json::json!("0.1.0");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = verify_build(&source).unwrap_err();
        assert!(format!("{err:#}").contains("unknown field `engineVersion`"));
    }

    #[test]
    fn runtime_assets_are_exact_and_role_bound() {
        let missing = fake_runtime();
        let missing_manifest_path = missing.join(BUILD_MANIFEST_FILE);
        let mut missing_manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&missing_manifest_path).unwrap()).unwrap();
        missing_manifest["runtimeAssets"] = serde_json::json!({});
        std::fs::write(
            &missing_manifest_path,
            serde_json::to_vec_pretty(&missing_manifest).unwrap(),
        )
        .unwrap();
        let err = verify_build(&missing).unwrap_err();
        assert!(
            format!("{err:#}").contains("missing field `engine`"),
            "{err:#}"
        );

        let wrong_role = fake_runtime();
        let wrong_manifest_path = wrong_role.join(BUILD_MANIFEST_FILE);
        let wrong_package_path = wrong_role.join("runtime/manifests/fixture.json");
        for path in [&wrong_manifest_path, &wrong_package_path] {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            value["assets"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|asset| asset["id"] == "engine-glue")
                .unwrap()["role"] = serde_json::json!("app");
            std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
        let err = verify_build(&wrong_role).unwrap_err();
        assert!(
            format!("{err:#}").contains("requires role 'glue'"),
            "{err:#}"
        );

        let wrong_path = fake_runtime();
        let wrong_path_manifest = wrong_path.join(BUILD_MANIFEST_FILE);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&wrong_path_manifest).unwrap()).unwrap();
        value["runtimeAssets"]["engine"]["glue"] = serde_json::json!(CANVASKIT_BASE_GLUE_PATH);
        std::fs::write(
            &wrong_path_manifest,
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        let err = verify_build(&wrong_path).unwrap_err();
        assert!(
            format!("{err:#}").contains("must use canonical path"),
            "{err:#}"
        );
    }

    #[test]
    fn package_identity_binds_unique_id_owner_and_exact_assets() {
        let source = fake_runtime();
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let package_path = source.join("runtime/manifests/fixture.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        let mut package: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&package_path).unwrap()).unwrap();
        manifest["assets"][0]["owner"] = serde_json::json!("other-owner");
        package["assets"][0]["owner"] = serde_json::json!("other-owner");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(&package_path, serde_json::to_vec_pretty(&package).unwrap()).unwrap();
        let err = verify_build(&source).unwrap_err();
        assert!(
            format!("{err:#}").contains("is owned by 'other-owner'"),
            "{err:#}"
        );

        let duplicate = fake_runtime();
        let duplicate_manifest_path = duplicate.join(BUILD_MANIFEST_FILE);
        let mut duplicate_manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&duplicate_manifest_path).unwrap()).unwrap();
        let package_ref = duplicate_manifest["packages"][0].clone();
        duplicate_manifest["packages"]
            .as_array_mut()
            .unwrap()
            .push(package_ref);
        std::fs::write(
            &duplicate_manifest_path,
            serde_json::to_vec_pretty(&duplicate_manifest).unwrap(),
        )
        .unwrap();
        let err = verify_build(&duplicate).unwrap_err();
        assert!(format!("{err:#}").contains("duplicate or empty web runtime package id"));
    }

    #[test]
    fn route_cannot_shadow_a_canonical_asset_path() {
        let source = fake_runtime();
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["routes"]["apps/preview/index.html"] =
            serde_json::json!("apps/preview/chunk-test.js");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let err = verify_build(&source).unwrap_err();
        assert!(format!("{err:#}").contains("would shadow a manifest asset"));

        let reserved = fake_runtime();
        let reserved_manifest_path = reserved.join(BUILD_MANIFEST_FILE);
        let mut reserved_manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&reserved_manifest_path).unwrap()).unwrap();
        reserved_manifest["routes"]["config.json"] = serde_json::json!("apps/preview/index.html");
        std::fs::write(
            &reserved_manifest_path,
            serde_json::to_vec_pretty(&reserved_manifest).unwrap(),
        )
        .unwrap();
        let err = verify_build(&reserved).unwrap_err();
        assert!(format!("{err:#}").contains("conflicts with a host route"));
    }

    #[test]
    fn one_runtime_version_cannot_be_replaced_by_different_verified_bytes() {
        let first = fake_runtime();
        let second = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_version_collision");
        let target = install(&first, &cache_root).unwrap();
        let original = std::fs::read(target.join("apps/preview/chunk-test.js")).unwrap();

        let changed = b"console.log('different release')";
        rewrite_fake_chunk(&second, changed);

        let err = install(&second, &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("version collision"), "{err:#}");
        assert_eq!(
            std::fs::read(target.join("apps/preview/chunk-test.js")).unwrap(),
            original
        );
    }

    #[test]
    fn resolve_recovers_the_deterministic_last_good_backup() {
        let source = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_recovery");
        let target = install(&source, &cache_root).unwrap();
        let backup = backup_path(&target).unwrap();
        std::fs::rename(&target, &backup).unwrap();
        sync_directory(&cache_root).unwrap();

        recover_interrupted_publish(&target).unwrap();
        let resolved = resolve(Some(&target), &cache_root).unwrap();
        assert_eq!(resolved.source, RuntimeSource::DevDir);
        assert!(target.is_dir());
        assert!(!backup.exists());
    }

    #[test]
    fn recovery_keeps_both_different_verified_bundles_for_explicit_resolution() {
        let target_source = fake_runtime();
        let backup_source = fake_runtime();
        rewrite_fake_chunk(&backup_source, b"console.log('other verified release')");
        let cache_root = temp_dir("valle_webruntime_ambiguous_recovery");
        let target = install(&target_source, &cache_root).unwrap();
        let backup = backup_path(&target).unwrap();
        std::fs::create_dir(&backup).unwrap();
        let backup_build = verify_build(&backup_source).unwrap();
        write_verified_build(&backup_build, &backup).unwrap();

        let err = recover_interrupted_publish(&target).unwrap_err();
        assert!(format!("{err:#}").contains("ambiguous web runtime recovery"));
        assert!(target.is_dir());
        assert!(backup.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_admission_rejects_symlinks_and_noncanonical_paths() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let source = fake_runtime();
        let chunk_path = source.join("apps/preview/chunk-test.js");
        let outside_root = temp_dir("valle_webruntime_outside");
        let outside = outside_root.join("chunk.js");
        std::fs::write(&outside, b"console.log('runtime')").unwrap();
        std::fs::remove_file(&chunk_path).unwrap();
        symlink(&outside, &chunk_path).unwrap();
        let err = verify_build(&source).unwrap_err();
        assert!(format!("{err:#}").contains("contains a symlink"), "{err:#}");
        std::fs::remove_file(outside).unwrap();

        let noncanonical = fake_runtime();
        let manifest_path = noncanonical.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["assets"][0]["path"] = serde_json::json!("apps//preview/index.html");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let err = verify_build(&noncanonical).unwrap_err();
        assert!(
            format!("{err:#}").contains("unsafe web runtime path"),
            "{err:#}"
        );

        let fifo_source = fake_runtime();
        let fifo = fifo_source.join("apps/preview/chunk-test.js");
        std::fs::remove_file(&fifo).unwrap();
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `fifo_c` is a valid NUL-terminated path owned for the duration of the call.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        let err = verify_build(&fifo_source).unwrap_err();
        assert!(format!("{err:#}").contains("not a regular file"), "{err:#}");
    }

    #[test]
    fn resolves_explicit_valle_web_manifest_with_route_aliases() {
        let dist = fake_runtime();
        let chunk_path = "apps/preview/chunk-test.js";
        let chunk = b"console.log('runtime')";
        let manifest_path = dist.join(BUILD_MANIFEST_FILE);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();

        let cache_root = temp_dir("valle_webruntime_cache");
        let hit = resolve(Some(&dist), &cache_root).unwrap();
        assert_eq!(hit.source, RuntimeSource::DevDir);
        assert_eq!(hit.manifest.name, "valle-web-runtime");
        let map = hit.serving_map();
        let HostedFile::VerifiedRuntime(html) = map.get("preview.html").unwrap() else {
            panic!("demo route is not frozen")
        };
        let HostedFile::VerifiedRuntime(chunk_bytes) = map.get("chunk-test.js").unwrap() else {
            panic!("chunk route is not frozen")
        };
        assert_eq!(html.as_ref(), b"<script src=\"./chunk-test.js\"></script>");
        assert_eq!(chunk_bytes.as_ref(), chunk);

        std::fs::write(dist.join(chunk_path), "tampered").unwrap();
        let err = resolve(Some(&dist), &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("sha256 mismatch"), "{err:#}");

        std::fs::write(dist.join(chunk_path), chunk).unwrap();
        let mut invalid = manifest.clone();
        invalid["assets"][1]["license"] = serde_json::json!("");
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let err = resolve(Some(&dist), &cache_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("requires owner and license"),
            "{err:#}"
        );

        let mut invalid = manifest.clone();
        invalid["workers"] = serde_json::json!([{ "id": "bad", "path": chunk_path }]);
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let err = resolve(Some(&dist), &cache_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("outside the worker closure"),
            "{err:#}"
        );

        let mut invalid = manifest;
        invalid["assetGroups"] = serde_json::json!([{
            "id": "bad-group",
            "glue": chunk_path,
            "wasm": [chunk_path],
        }]);
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let err = resolve(Some(&dist), &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("does not own"), "{err:#}");
    }

    #[test]
    fn resolve_rejects_dir_without_manifest() {
        let junk = temp_dir("valle_webruntime_junk");
        std::fs::write(junk.join("random.txt"), "x").unwrap();
        let cache_root = temp_dir("valle_webruntime_cache_junk");
        let err = resolve(Some(&junk), &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("manifest"), "{err:#}");
        assert!(format!("{err:#}").contains("manifest"), "{err:#}");
    }

    #[test]
    fn runtime_version_accepts_only_a_semver_safe_path_component() {
        for accepted in [
            "0.1.0",
            "1.2.3-alpha.1",
            "1.2.3+build.5",
            "1.2.3-rc.1+sha-abc",
        ] {
            ensure_safe_runtime_version(accepted).unwrap();
        }

        for rejected in [
            "",
            ".",
            "..",
            "../escape",
            "1.2.3/escape",
            r"1.2.3\escape",
            "/tmp/1.2.3",
            r"C:\runtime\1.2.3",
            "1.2.3\nnext",
            "v1.2.3",
            "1.2",
            "01.2.3",
            "1.2.3-01",
        ] {
            let err = ensure_safe_runtime_version(rejected).unwrap_err();
            assert!(format!("{err:#}").contains("unsafe web runtime version"));
        }
    }

    #[test]
    fn install_rejects_unsafe_runtime_version_before_creating_a_slot() {
        let source = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_unsafe_version_cache");
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["runtimeVersion"] = serde_json::json!("../escaped");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = install(&source, &cache_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("unsafe web runtime version"),
            "{err:#}"
        );
        assert!(std::fs::read_dir(&cache_root).unwrap().next().is_none());
    }

    #[test]
    fn resolve_rejects_cached_slot_identity_mismatch() {
        let source = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_slot_mismatch_cache");
        let installed = install(&source, &cache_root).unwrap();
        let manifest_path = installed.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["runtimeVersion"] = serde_json::json!("9.9.9");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = resolve(Some(&cache_root.join(RUNTIME_VERSION)), &cache_root).unwrap_err();
        assert!(format!("{err:#}").contains("does not match CLI"), "{err:#}");
    }

    #[test]
    fn resolve_rejects_wrong_protocol_even_when_semver_matches() {
        let source = fake_runtime();
        let cache_root = temp_dir("valle_webruntime_protocol_mismatch_cache");
        let installed = install(&source, &cache_root).unwrap();
        let manifest_path = source.join(BUILD_MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["runtimeVersion"], RUNTIME_VERSION);
        manifest["protocolVersion"] = serde_json::json!(PROTOCOL_VERSION + 1);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = resolve(Some(&source), &cache_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("web runtime protocol mismatch"),
            "{err:#}"
        );

        std::fs::write(
            installed.join(BUILD_MANIFEST_FILE),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let err = resolve(Some(&cache_root.join(RUNTIME_VERSION)), &cache_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("web runtime protocol mismatch"),
            "{err:#}"
        );
    }
}
