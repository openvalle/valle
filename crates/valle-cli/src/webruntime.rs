//! Verify explicitly supplied development assets or serve the Web runtime embedded in Valle.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Shared Rust/Web runtime handshake version.
pub const PROTOCOL_VERSION: u32 =
    parse_protocol_version(include_str!("../../../web/runtime-protocol-version.txt"));
/// Runtime and engine versions are released together.
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Dependency metadata shares the Web package's exact CanvasKit pin.
fn canvaskit_version() -> String {
    let package: serde_json::Value =
        serde_json::from_str(include_str!("../../../web/package.json"))
            .expect("checked-in Web package.json must be valid JSON");
    package["workspaces"]["catalog"]["canvaskit-wasm"]
        .as_str()
        .expect("Web catalog must pin canvaskit-wasm")
        .to_owned()
}
/// Aggregate manifest relative to the web build root.
pub const BUILD_MANIFEST_FILE: &str = "runtime/manifest.json";
pub const ENGINE_GLUE_PATH: &str = "runtime/engine/valle_engine.js";
pub const ENGINE_WASM_PATH: &str = "runtime/engine/valle_engine_bg.wasm";
pub const CANVASKIT_FULL_GLUE_PATH: &str = "runtime/canvaskit/canvaskit.js";
pub const CANVASKIT_FULL_WASM_PATH: &str = "runtime/canvaskit/canvaskit.wasm";
pub const DEFAULT_SANS_FONT_PATH: &str = "runtime/fonts/NotoSans-Regular.ttf";
pub const PRODUCT_FRAME_WORKER_PATH: &str = "runtime/workers/product-frame.js";

/// Rust hosts and the admitted root manifest use one canonical runtime URL map.
pub fn runtime_assets_json() -> serde_json::Value {
    serde_json::json!({
        "engine": { "glue": ENGINE_GLUE_PATH, "wasm": ENGINE_WASM_PATH },
        "canvasKit": {
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
    /// Explicit development directory; invalid assets never fall back to the embedded runtime.
    DevDir,
    /// Assets compiled into the executable.
    Embedded,
}

impl RuntimeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeSource::DevDir => "dev-dir",
            RuntimeSource::Embedded => "embedded",
        }
    }
}

pub struct ResolvedRuntime {
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
            .field("source", &self.source)
            .field("file_count", &self.frozen_files.len())
            .finish()
    }
}

/// Verified manifest and asset bytes captured once before serving.
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

impl ResolvedRuntime {
    /// Map manifest assets and public routes to hosted content.
    pub fn serving_map(&self) -> BTreeMap<String, HostedFile> {
        let mut files: BTreeMap<_, _> = self
            .frozen_files
            .iter()
            .map(|(path, bytes)| (path.clone(), HostedFile::VerifiedRuntime(Arc::clone(bytes))))
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

/// Verify the full asset graph rooted at dist/runtime/manifest.json.
fn verify_build(dir: &Path) -> Result<VerifiedBuild> {
    let root = std::fs::canonicalize(dir)
        .with_context(|| format!("canonicalizing web runtime root {}", dir.display()))?;
    if !std::fs::metadata(&root)?.is_dir() {
        bail!("web runtime root is not a directory: {}", dir.display());
    }
    verify_files(|relative| read_regular_runtime_file(&root, relative))
}

fn verify_files(read: impl Fn(&str) -> Result<Vec<u8>>) -> Result<VerifiedBuild> {
    let root_bytes = read(BUILD_MANIFEST_FILE).context("reading Web runtime manifest")?;
    let build: BuildManifest =
        serde_json::from_slice(&root_bytes).context("parsing Web runtime manifest")?;
    if build.schema_version != 2 {
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
        let data = read(&file.path)?;
        if data.len() as u64 != file.bytes || sha256_hex(&data) != file.sha256 {
            bail!(
                "web runtime file corrupted: {} (sha256 mismatch)",
                file.path
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
        let package_bytes = read(&package_ref.manifest)?;
        let package: BuildPackageManifest = serde_json::from_slice(&package_bytes)
            .with_context(|| format!("parsing {}", package_ref.manifest))?;
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

/// Require a canonical Cargo SemVer identity for the host/runtime handshake.
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
        } else if byte == b'\r' && index + 2 == bytes.len() && bytes[index + 1] == b'\n' {
            // Git may check out the shared version file with CRLF on Windows.
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

/// Explicit development assets override embedded assets. No cache or installation is needed.
pub fn resolve(explicit: Option<&Path>) -> Result<ResolvedRuntime> {
    let (source, verified) = if let Some(dir) = explicit {
        (
            RuntimeSource::DevDir,
            verify_build(dir)
                .with_context(|| format!("verifying Studio resources {}", dir.display()))?,
        )
    } else {
        if crate::embedded::WEB_FILES.is_empty() {
            bail!(
                "This development build has no embedded Studio resources. Run `cargo xtask build` or pass --web-assets-dir with a built web/dist directory"
            );
        }
        (
            RuntimeSource::Embedded,
            verify_files(|relative| {
                crate::embedded::WEB_FILES
                    .iter()
                    .find(|(path, _)| *path == relative)
                    .map(|(_, bytes)| bytes.to_vec())
                    .ok_or_else(|| anyhow::anyhow!("embedded Web asset missing: {relative}"))
            })?,
        )
    };
    if verified.manifest.runtime_version != RUNTIME_VERSION {
        bail!(
            "Studio resource version {} does not match CLI {RUNTIME_VERSION}; rebuild them together",
            verified.manifest.runtime_version
        );
    }
    resolved_runtime(source, verified)
}

fn resolved_runtime(source: RuntimeSource, verified: VerifiedBuild) -> Result<ResolvedRuntime> {
    let mut all_files = BTreeMap::new();
    for file in verified.files {
        if all_files
            .insert(file.relative.clone(), Arc::<[u8]>::from(file.bytes))
            .is_some()
        {
            bail!("verified web runtime repeats file '{}'", file.relative);
        }
    }
    Ok(ResolvedRuntime {
        source,
        manifest: verified.manifest,
        aliases: verified.aliases,
        frozen_files: all_files,
    })
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_accepts_lf_and_crlf_in_const_evaluation() {
        const VERSIONS: [u32; 3] = [
            parse_protocol_version("42"),
            parse_protocol_version("42\n"),
            parse_protocol_version("42\r\n"),
        ];
        assert_eq!(VERSIONS, [42; 3]);
        assert_eq!(parse_protocol_version("4294967295\r\n"), u32::MAX);
    }

    #[test]
    fn protocol_version_rejects_invalid_content() {
        for source in [
            "",
            "\n",
            "\r\n",
            "1\r",
            "1\r\n2",
            "1\n\n",
            " 1",
            "1 ",
            "a",
            "4294967296",
        ] {
            assert!(
                std::panic::catch_unwind(|| parse_protocol_version(source)).is_err(),
                "accepted {source:?}"
            );
        }
    }

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
        let specs: [(&str, &str, &str, &[u8], Option<&str>); 8] = [
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
                "schemaVersion": 2,
                "package": "fixture",
                "assets": assets.clone(),
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "runtimeVersion": RUNTIME_VERSION,
            "protocolVersion": PROTOCOL_VERSION,
            "packages": [{ "id": "fixture", "manifest": "runtime/manifests/fixture.json" }],
            "assets": assets,
            "assetGroups": [
                { "id": "engine-core", "glue": ENGINE_GLUE_PATH, "wasm": [ENGINE_WASM_PATH] },
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
        value["runtimeAssets"]["engine"]["glue"] = serde_json::json!(CANVASKIT_FULL_GLUE_PATH);
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

        let hit = resolve(Some(&dist)).unwrap();
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
        let err = resolve(Some(&dist)).unwrap_err();
        assert!(format!("{err:#}").contains("sha256 mismatch"), "{err:#}");

        std::fs::write(dist.join(chunk_path), chunk).unwrap();
        let mut invalid = manifest.clone();
        invalid["assets"][1]["license"] = serde_json::json!("");
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let err = resolve(Some(&dist)).unwrap_err();
        assert!(
            format!("{err:#}").contains("requires owner and license"),
            "{err:#}"
        );

        let mut invalid = manifest.clone();
        invalid["workers"] = serde_json::json!([{ "id": "bad", "path": chunk_path }]);
        std::fs::write(&manifest_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let err = resolve(Some(&dist)).unwrap_err();
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
        let err = resolve(Some(&dist)).unwrap_err();
        assert!(format!("{err:#}").contains("does not own"), "{err:#}");
    }

    #[test]
    fn resolve_rejects_dir_without_manifest() {
        let junk = temp_dir("valle_webruntime_junk");
        std::fs::write(junk.join("random.txt"), "x").unwrap();
        let err = resolve(Some(&junk)).unwrap_err();
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
    fn verified_assets_remain_frozen_after_development_files_change() {
        let dir = fake_runtime();
        let runtime = resolve(Some(&dir)).unwrap();
        std::fs::write(dir.join("apps/preview/chunk-test.js"), "tampered").unwrap();
        let files = runtime.serving_map();
        let HostedFile::VerifiedRuntime(bytes) = &files["chunk-test.js"] else {
            panic!()
        };
        assert_eq!(bytes.as_ref(), b"console.log('runtime')");
        assert!(resolve(Some(&dir)).is_err());
    }

    #[test]
    fn rejects_mismatched_runtime_and_protocol_versions() {
        for (field, value) in [
            ("runtimeVersion", serde_json::json!("9.9.9")),
            ("protocolVersion", serde_json::json!(PROTOCOL_VERSION + 1)),
        ] {
            let dir = fake_runtime();
            let path = dir.join(BUILD_MANIFEST_FILE);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            manifest[field] = value;
            std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(resolve(Some(&dir)).is_err());
        }
    }

    #[cfg(feature = "embedded-runtime")]
    #[test]
    fn embedded_runtime_is_complete_and_explicit_errors_do_not_fall_back() {
        let runtime = resolve(None).unwrap();
        assert_eq!(runtime.source, RuntimeSource::Embedded);
        let files = runtime.serving_map();
        for path in [
            ENGINE_WASM_PATH,
            CANVASKIT_FULL_WASM_PATH,
            DEFAULT_SANS_FONT_PATH,
        ] {
            let HostedFile::VerifiedRuntime(bytes) = &files[path] else {
                panic!()
            };
            assert!(!bytes.is_empty());
        }
        assert!(resolve(Some(Path::new("/nonexistent-valle-web-directory"))).is_err());
    }
}
