//! Native CoreML graph loading and inference for macOS.
//!
//! Published `.mlpackage` directories are compiled on the target Mac and the resulting
//! `.mlmodelc` directory is cached. This avoids shipping a machine-compiled bundle and avoids
//! ONNX Runtime's separate CoreML execution-provider conversion path.

// MLMultiArray's pointer API is deprecated in favor of block-based access, but it remains the
// only binding here that exposes strides and lets us correctly read ANE-padded output tensors.
#![allow(deprecated)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use block2::RcBlock;
use fs2::FileExt;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLFeatureValue, MLModel,
    MLModelConfiguration, MLMultiArray, MLMultiArrayDataType,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSProcessInfo, NSString, NSURL};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::runtime::{TensorInput, TensorOutput};

/// Hash the normalized artifact file manifest used as the source identity of a compiled model.
///
/// Records are sorted by path and encoded with explicit lengths before hashing, so input order or
/// ambiguous string concatenation cannot change/collide with the identity. The returned lowercase
/// SHA-256 is intentionally complete: truncating individual file hashes weakens cache invalidation
/// for multi-file packages.
pub fn artifact_files_digest<'a>(
    files: impl IntoIterator<Item = (&'a str, u64, &'a str)>,
) -> Result<String> {
    let mut files: Vec<_> = files.into_iter().collect();
    files.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let mut digest = Sha256::new();
    digest.update(b"valle.artifact-files.v1\0");
    let mut previous_path = None;
    for (path, bytes, sha256) in files {
        ensure!(!path.is_empty(), "artifact file path must not be empty");
        ensure!(
            previous_path != Some(path),
            "artifact file manifest contains duplicate path {path:?}"
        );
        ensure!(
            sha256.len() == 64
                && sha256
                    .as_bytes()
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)),
            "artifact file {path:?} has a non-canonical SHA-256"
        );
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update(bytes.to_be_bytes());
        digest.update(sha256.as_bytes());
        previous_path = Some(path);
    }
    let digest = digest.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Clone, Copy)]
pub enum CoreMlComputeUnits {
    CpuOnly,
    CpuAndGpu,
    CpuAndNeuralEngine,
    All,
}

impl CoreMlComputeUnits {
    fn native(self) -> MLComputeUnits {
        match self {
            Self::CpuOnly => MLComputeUnits::CPUOnly,
            Self::CpuAndGpu => MLComputeUnits::CPUAndGPU,
            Self::CpuAndNeuralEngine => MLComputeUnits::CPUAndNeuralEngine,
            Self::All => MLComputeUnits::All,
        }
    }

    const fn cache_identity(self) -> &'static str {
        match self {
            Self::CpuOnly => "cpu-only",
            Self::CpuAndGpu => "cpu-and-gpu",
            Self::CpuAndNeuralEngine => "cpu-and-neural-engine",
            Self::All => "all",
        }
    }
}

pub struct CoreMlSession {
    model: Retained<MLModel>,
    compiled_path: PathBuf,
}

impl CoreMlSession {
    pub fn load(
        source: &Path,
        compile_cache: &Path,
        cache_key: &str,
        compute_units: CoreMlComputeUnits,
    ) -> Result<Self> {
        let compiled_path = compile_if_needed(source, compile_cache, cache_key, compute_units)?;
        let canonical = fs::canonicalize(&compiled_path).with_context(|| {
            format!(
                "compiled CoreML model is missing: {}",
                compiled_path.display()
            )
        })?;
        let model = unsafe {
            let configuration = MLModelConfiguration::new();
            configuration.setComputeUnits(compute_units.native());
            let url = NSURL::fileURLWithPath(&NSString::from_str(&canonical.to_string_lossy()));
            MLModel::modelWithContentsOfURL_configuration_error(&url, &configuration)
                .map_err(|error| anyhow!("failed to load {}: {error:?}", canonical.display()))?
        };
        Ok(Self {
            model,
            compiled_path: canonical,
        })
    }

    pub fn compiled_path(&self) -> &Path {
        &self.compiled_path
    }

    pub fn run_f32(
        &self,
        inputs: &[TensorInput<'_>],
        output_names: &[&str],
    ) -> Result<Vec<TensorOutput>> {
        unsafe {
            let mut values: Vec<Retained<MLFeatureValue>> = Vec::with_capacity(inputs.len());
            let mut keys: Vec<Retained<NSString>> = Vec::with_capacity(inputs.len());
            for input in inputs {
                let expected: usize = input.shape.iter().product();
                ensure!(
                    input.data.len() == expected,
                    "input {:?} has {} values, shape {:?} requires {expected}",
                    input.name,
                    input.data.len(),
                    input.shape
                );
                let dimensions: Vec<Retained<NSNumber>> = input
                    .shape
                    .iter()
                    .map(|&dimension| NSNumber::new_usize(dimension))
                    .collect();
                let dimension_refs: Vec<&NSNumber> =
                    dimensions.iter().map(|dimension| &**dimension).collect();
                let array = MLMultiArray::initWithShape_dataType_error(
                    MLMultiArray::alloc(),
                    &NSArray::from_slice(&dimension_refs),
                    MLMultiArrayDataType::Float32,
                )
                .map_err(|error| {
                    anyhow!(
                        "failed to allocate CoreML input {:?}: {error:?}",
                        input.name
                    )
                })?;
                std::ptr::copy_nonoverlapping(
                    input.data.as_ptr(),
                    array.dataPointer().as_ptr() as *mut f32,
                    expected,
                );
                values.push(MLFeatureValue::featureValueWithMultiArray(&array));
                keys.push(NSString::from_str(&input.name));
            }
            let key_refs: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
            let value_refs: Vec<&AnyObject> =
                values.iter().map(|value| &**value as &AnyObject).collect();
            let dictionary: Retained<NSDictionary<NSString, AnyObject>> =
                NSDictionary::from_slices(&key_refs, &value_refs);
            let provider = MLDictionaryFeatureProvider::initWithDictionary_error(
                MLDictionaryFeatureProvider::alloc(),
                &dictionary,
            )
            .map_err(|error| anyhow!("failed to construct CoreML input provider: {error:?}"))?;
            let outputs = self
                .model
                .predictionFromFeatures_error(ProtocolObject::from_ref(&*provider))
                .map_err(|error| anyhow!("CoreML inference failed: {error:?}"))?;

            output_names
                .iter()
                .map(|name| {
                    let value = outputs
                        .featureValueForName(&NSString::from_str(name))
                        .ok_or_else(|| anyhow!("CoreML output {name:?} is missing"))?
                        .multiArrayValue()
                        .ok_or_else(|| anyhow!("CoreML output {name:?} is not a multi-array"))?;
                    read_output(&value, name)
                })
                .collect()
        }
    }
}

const CACHE_LAYOUT_VERSION: u32 = 1;
const CACHE_MARKER_FILE: &str = ".valle-coreml-cache.json";
const RENAME_EXCL: u32 = 0x0000_0004;
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" {
    fn renamex_np(from: *const c_char, to: *const c_char, flags: u32) -> c_int;
    fn sysctlbyname(
        name: *const c_char,
        old_value: *mut c_void,
        old_len: *mut usize,
        new_value: *mut c_void,
        new_len: usize,
    ) -> c_int;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheMarker {
    layout_version: u32,
    derived_key: String,
    files: Vec<CacheFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct CacheFileRecord {
    path: String,
    size: u64,
}

struct StagingDirectory {
    path: PathBuf,
    published: bool,
}

impl StagingDirectory {
    fn create(parent: &Path, cache_key: &str) -> Result<Self> {
        for _ in 0..1024 {
            let sequence = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                "{cache_key}.partial.{}.{}",
                std::process::id(),
                sequence
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        published: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to create CoreML staging directory {}",
                            path.display()
                        )
                    });
                }
            }
        }
        Err(anyhow!(
            "failed to allocate a unique CoreML staging directory for {cache_key:?}"
        ))
    }

    fn publish(&mut self, target: &Path) -> io::Result<()> {
        atomic_publish_no_replace(&self.path, target)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn compile_if_needed(
    source: &Path,
    cache: &Path,
    source_cache_key: &str,
    compute_units: CoreMlComputeUnits,
) -> Result<PathBuf> {
    ensure!(
        source.exists(),
        "CoreML model is missing: {}",
        source.display()
    );
    if source
        .extension()
        .is_some_and(|extension| extension == "mlmodelc")
    {
        return Ok(source.to_path_buf());
    }

    // CoreML has no public compiler build identifier. The compiler and runtime ship with macOS,
    // so the exact OS build is the narrowest stable public invalidation boundary. Architecture
    // and compute units are included separately because generated execution plans can differ.
    let platform_identity = coreml_platform_identity()?;
    compile_cached_with(
        source,
        cache,
        source_cache_key,
        compute_units.cache_identity(),
        &platform_identity,
        compile_with_coreml,
    )
}

fn compile_cached_with<F>(
    source: &Path,
    cache: &Path,
    source_cache_key: &str,
    compute_identity: &str,
    platform_identity: &str,
    compile: F,
) -> Result<PathBuf>
where
    F: FnOnce(&Path) -> Result<PathBuf>,
{
    ensure!(
        source_cache_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte)),
        "unsafe CoreML source cache key {source_cache_key:?}"
    );
    ensure!(source.is_dir(), "CoreML source must be a package directory");

    let cache_key =
        derive_runtime_cache_key(source_cache_key, compute_identity, platform_identity)?;
    let directory = cache.join("coreml-compiled");
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create CoreML compile cache {}",
            directory.display()
        )
    })?;
    let target = directory.join(format!("{cache_key}.mlmodelc"));
    if cache_entry_is_valid(&target, &cache_key)? {
        return Ok(target);
    }

    let lock_directory = directory.join(".locks");
    fs::create_dir_all(&lock_directory).with_context(|| {
        format!(
            "failed to create CoreML lock directory {}",
            lock_directory.display()
        )
    })?;
    let lock_path = lock_directory.join(format!("{cache_key}.lock"));
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open CoreML compile lock {}", lock_path.display()))?;
    FileExt::lock_exclusive(&lock_file)
        .with_context(|| format!("failed to lock CoreML cache entry {cache_key:?}"))?;

    // A process can exit after creating its staging directory. The advisory lock is released by
    // the OS, so the next lock owner can safely remove same-key staging left by a dead process.
    remove_stale_staging_directories(&directory, &cache_key)?;

    // Another process may have completed the entry while this process waited for the lock.
    if cache_entry_is_valid(&target, &cache_key)? {
        return Ok(target);
    }
    remove_cache_entry_if_present(&target)?;

    let compiled = compile(source)?;
    ensure!(
        compiled.is_dir(),
        "CoreML compiler did not return a compiled package directory: {}",
        compiled.display()
    );
    let mut staging = StagingDirectory::create(&directory, &cache_key)?;
    copy_directory_contents(&compiled, &staging.path)?;
    write_cache_marker(&staging.path, &cache_key)?;
    sync_directory(&staging.path)?;

    match staging.publish(&target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if cache_entry_is_valid(&target, &cache_key)? {
                return Ok(target);
            }
            return Err(error).with_context(|| {
                format!(
                    "CoreML cache target {} appeared during publication but is invalid",
                    target.display()
                )
            });
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to atomically publish CoreML model {} to {}",
                    staging.path.display(),
                    target.display()
                )
            });
        }
    }
    sync_directory(&directory)?;
    ensure!(
        cache_entry_is_valid(&target, &cache_key)?,
        "published CoreML cache entry failed completeness validation: {}",
        target.display()
    );
    Ok(target)
}

fn derive_runtime_cache_key(
    source_cache_key: &str,
    compute_identity: &str,
    platform_identity: &str,
) -> Result<String> {
    ensure!(
        source_cache_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte)),
        "unsafe CoreML source cache key {source_cache_key:?}"
    );
    ensure!(
        !compute_identity.is_empty(),
        "empty CoreML compute identity"
    );
    ensure!(
        compute_identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "unsafe CoreML compute identity {compute_identity:?}"
    );
    ensure!(
        !platform_identity.is_empty(),
        "empty CoreML platform identity"
    );
    let platform_digest = Sha256::digest(platform_identity.as_bytes());
    let platform_digest = platform_digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(
        "{source_cache_key}.coreml-v{CACHE_LAYOUT_VERSION}.{compute_identity}.{platform_digest}"
    ))
}

fn coreml_platform_identity() -> Result<String> {
    let process_info = NSProcessInfo::processInfo();
    let version = process_info.operatingSystemVersion();
    let build_name = c"kern.osversion";
    let build = sysctl_string(build_name).context("failed to read the macOS build identity")?;
    ensure!(!build.is_empty(), "macOS build identity is empty");
    Ok(format!(
        "macos-{}.{}.{}-build-{build}-arch-{}",
        version.majorVersion,
        version.minorVersion,
        version.patchVersion,
        std::env::consts::ARCH
    ))
}

fn sysctl_string(name: &CStr) -> io::Result<String> {
    let mut length = 0_usize;
    let status = unsafe {
        sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sysctl returned an empty value",
        ));
    }
    let mut bytes = vec![0_u8; length];
    let status = unsafe {
        sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    bytes.truncate(length);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn cache_entry_is_valid(target: &Path, cache_key: &str) -> Result<bool> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect CoreML cache {}", target.display()));
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }

    let marker_path = target.join(CACHE_MARKER_FILE);
    let marker_metadata = match fs::symlink_metadata(&marker_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect CoreML marker {}", marker_path.display())
            });
        }
    };
    if !marker_metadata.file_type().is_file() || marker_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let marker: CacheMarker = match fs::read(&marker_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(marker) => marker,
        None => return Ok(false),
    };
    if marker.layout_version != CACHE_LAYOUT_VERSION || marker.derived_key != cache_key {
        return Ok(false);
    }
    let files = match collect_cache_files(target) {
        Ok(files) => files,
        Err(_) => return Ok(false),
    };
    Ok(!files.is_empty() && files == marker.files)
}

fn write_cache_marker(target: &Path, cache_key: &str) -> Result<()> {
    let files = collect_cache_files(target)?;
    ensure!(
        !files.is_empty(),
        "compiled CoreML package {} contains no files",
        target.display()
    );
    let marker = CacheMarker {
        layout_version: CACHE_LAYOUT_VERSION,
        derived_key: cache_key.to_owned(),
        files,
    };
    let marker_path = target.join(CACHE_MARKER_FILE);
    let bytes = serde_json::to_vec(&marker).context("failed to encode CoreML cache marker")?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker_path)
        .with_context(|| format!("failed to create CoreML marker {}", marker_path.display()))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn collect_cache_files(root: &Path) -> Result<Vec<CacheFileRecord>> {
    let mut files = Vec::new();
    collect_cache_files_from(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_cache_files_from(
    root: &Path,
    directory: &Path,
    files: &mut Vec<CacheFileRecord>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read CoreML directory {}", directory.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let relative = path
            .strip_prefix(root)
            .expect("cache walk must stay under root");
        if relative == Path::new(CACHE_MARKER_FILE) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "compiled CoreML bundle contains a symbolic link {}",
                path.display()
            ));
        }
        if metadata.file_type().is_dir() {
            collect_cache_files_from(root, &path, files)?;
        } else if metadata.file_type().is_file() {
            let relative = relative.to_str().with_context(|| {
                format!(
                    "compiled CoreML bundle path is not valid UTF-8: {}",
                    path.display()
                )
            })?;
            files.push(CacheFileRecord {
                path: relative.to_owned(),
                size: metadata.len(),
            });
        } else {
            return Err(anyhow!(
                "compiled CoreML bundle contains unsupported filesystem entry {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn remove_cache_entry_if_present(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| {
        format!(
            "failed to remove invalid CoreML cache entry {}",
            path.display()
        )
    })
}

fn remove_stale_staging_directories(directory: &Path, cache_key: &str) -> Result<()> {
    let prefix = format!("{cache_key}.partial.");
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to inspect CoreML cache {}", directory.display()))?
    {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            remove_cache_entry_if_present(&entry.path())?;
        }
    }
    Ok(())
}

fn atomic_publish_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let status = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), RENAME_EXCL) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn compile_with_coreml(source: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(source)
        .with_context(|| format!("failed to resolve CoreML source {}", source.display()))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&canonical.to_string_lossy()));
    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |compiled: *mut NSURL, error: *mut NSError| {
        let result = unsafe {
            if let Some(compiled) = compiled.as_ref() {
                compiled
                    .path()
                    .map(|path| PathBuf::from(path.to_string()))
                    .ok_or_else(|| "CoreML returned a compiled URL without a file path".to_string())
            } else {
                let message = error
                    .as_ref()
                    .map(|value| format!("{value:?}"))
                    .unwrap_or_else(|| "unknown CoreML compilation error".into());
                Err(message)
            }
        };
        let _ = sender.send(result);
    });
    unsafe {
        MLModel::compileModelAtURL_completionHandler(&url, &handler);
    }
    receiver
        .recv_timeout(Duration::from_secs(10 * 60))
        .context("timed out compiling CoreML package")?
        .map_err(|error| anyhow!("failed to compile {}: {error}", canonical.display()))
}

fn copy_directory_contents(source: &Path, target: &Path) -> Result<()> {
    ensure!(
        target.is_dir(),
        "CoreML staging directory is missing: {}",
        target.display()
    );
    let mut entries = fs::read_dir(source)
        .with_context(|| format!("failed to read compiled CoreML model {}", source.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path())?;
        let destination = target.join(entry.file_name());
        ensure!(
            entry.file_name() != CACHE_MARKER_FILE,
            "compiled CoreML bundle uses reserved cache marker name {CACHE_MARKER_FILE:?}"
        );
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "compiled CoreML bundle contains a symbolic link {}",
                entry.path().display()
            ));
        }
        if metadata.file_type().is_dir() {
            fs::create_dir(&destination)?;
            copy_directory_contents(&entry.path(), &destination)?;
            sync_directory(&destination)?;
        } else if metadata.file_type().is_file() {
            fs::copy(entry.path(), &destination)?;
            File::open(&destination)?.sync_all()?;
        } else {
            return Err(anyhow!(
                "compiled CoreML bundle contains unsupported filesystem entry {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open directory {} for sync", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync directory {}", path.display()))
}

unsafe fn read_output(array: &MLMultiArray, name: &str) -> Result<TensorOutput> {
    unsafe {
        let count = array.count() as usize;
        let shape: Vec<usize> = array
            .shape()
            .iter()
            .map(|value| value.integerValue() as usize)
            .collect();
        let strides: Vec<usize> = array
            .strides()
            .iter()
            .map(|value| value.integerValue() as usize)
            .collect();
        let data_type = array.dataType();
        let base = array.dataPointer().as_ptr();
        ensure!(
            matches!(
                data_type,
                MLMultiArrayDataType::Float16
                    | MLMultiArrayDataType::Float32
                    | MLMultiArrayDataType::Double
            ),
            "CoreML output {name:?} has unsupported type {data_type:?}"
        );
        let read = |offset: usize| match data_type {
            MLMultiArrayDataType::Float16 => half_to_f32(*(base as *const u16).add(offset)),
            MLMultiArrayDataType::Float32 => *(base as *const f32).add(offset),
            MLMultiArrayDataType::Double => *(base as *const f64).add(offset) as f32,
            _ => unreachable!(),
        };

        let mut expected_stride = 1;
        let mut contiguous = true;
        for dimension in (0..shape.len()).rev() {
            if strides[dimension] != expected_stride {
                contiguous = false;
                break;
            }
            expected_stride *= shape[dimension];
        }
        let data = if contiguous {
            (0..count).map(read).collect()
        } else {
            let mut output = Vec::with_capacity(count);
            let mut index = vec![0_usize; shape.len()];
            for _ in 0..count {
                let offset = index
                    .iter()
                    .zip(&strides)
                    .map(|(index, stride)| index * stride)
                    .sum();
                output.push(read(offset));
                for dimension in (0..shape.len()).rev() {
                    index[dimension] += 1;
                    if index[dimension] < shape[dimension] {
                        break;
                    }
                    index[dimension] = 0;
                }
            }
            output
        };
        Ok(TensorOutput {
            name: name.into(),
            shape,
            data,
        })
    }
}

fn half_to_f32(value: u16) -> f32 {
    let sign = ((value >> 15) as u32) << 31;
    let exponent = ((value >> 10) & 0x1f) as u32;
    let mantissa = (value & 0x03ff) as u32;
    let bits = match exponent {
        0 if mantissa == 0 => sign,
        0 => {
            let mut normalized = mantissa;
            let mut shifts = 0_u32;
            while normalized & 0x0400 == 0 {
                normalized <<= 1;
                shifts += 1;
            }
            sign | ((113 - shifts) << 23) | ((normalized & 0x03ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            for _ in 0..1024 {
                let sequence = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "valle-coreml-cache-{label}-{}.{}",
                    std::process::id(),
                    sequence
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("failed to create test root: {error}"),
                }
            }
            panic!("failed to allocate unique test root");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake_source(root: &Path) -> PathBuf {
        let source = root.join("artifact").join("model.mlpackage");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Manifest.json"), b"source").unwrap();
        source
    }

    fn fake_compiled(root: &Path, name: &str, payload: &[u8]) -> PathBuf {
        let compiled = root.join(name);
        fs::create_dir_all(compiled.join("weights")).unwrap();
        fs::write(compiled.join("model.mil"), payload).unwrap();
        fs::write(compiled.join("weights").join("weight.bin"), b"weights").unwrap();
        compiled
    }

    #[test]
    fn converts_common_half_values() {
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert!(half_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn artifact_digest_is_order_independent_and_covers_every_file_field() {
        let sha_a = "11".repeat(32);
        let sha_b = "22".repeat(32);
        let first = artifact_files_digest([
            ("Data/weights.bin", 42, sha_a.as_str()),
            ("Manifest.json", 7, sha_b.as_str()),
        ])
        .unwrap();
        let reordered = artifact_files_digest([
            ("Manifest.json", 7, sha_b.as_str()),
            ("Data/weights.bin", 42, sha_a.as_str()),
        ])
        .unwrap();
        let changed_size = artifact_files_digest([
            ("Data/weights.bin", 43, sha_a.as_str()),
            ("Manifest.json", 7, sha_b.as_str()),
        ])
        .unwrap();

        assert_eq!(first.len(), 64);
        assert_eq!(first, reordered);
        assert_ne!(first, changed_size);
        assert!(artifact_files_digest([("Manifest.json", 7, "ABC")]).is_err());
        assert!(
            artifact_files_digest([
                ("Manifest.json", 7, sha_a.as_str()),
                ("Manifest.json", 7, sha_b.as_str()),
            ])
            .is_err()
        );
    }

    #[test]
    fn runtime_key_is_stable_and_scoped_by_compute_and_platform() {
        let base = "birefnet-1.0.0-coreml-01234567";
        let first = derive_runtime_cache_key(base, "cpu-only", "macos-build-a").unwrap();
        let repeat = derive_runtime_cache_key(base, "cpu-only", "macos-build-a").unwrap();
        let compute = derive_runtime_cache_key(base, "all", "macos-build-a").unwrap();
        let platform = derive_runtime_cache_key(base, "cpu-only", "macos-build-b").unwrap();

        assert_eq!(first, repeat);
        assert!(first.starts_with(base));
        assert_ne!(first, compute);
        assert_ne!(first, platform);
        assert!(derive_runtime_cache_key("../escape", "all", "macos-build-a").is_err());
    }

    #[test]
    fn platform_identity_contains_exact_build_scope() {
        let identity = coreml_platform_identity().unwrap();
        assert!(identity.starts_with("macos-"));
        assert!(identity.contains("-build-"));
        assert!(identity.ends_with(std::env::consts::ARCH));
    }

    #[test]
    fn concurrent_callers_compile_one_complete_entry() {
        let root = TestRoot::new("concurrent");
        let source = fake_source(root.path());
        let cache = root.path().join("cache");
        let compiler_root = root.path().join("compiler-output");
        fs::create_dir(&compiler_root).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();

        for index in 0..8 {
            let source = source.clone();
            let cache = cache.clone();
            let compiler_root = compiler_root.clone();
            let calls = Arc::clone(&calls);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                compile_cached_with(
                    &source,
                    &cache,
                    "model-1-artifact-deadbeef",
                    "all",
                    "macos-build-test",
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(fake_compiled(
                            &compiler_root,
                            &format!("compiled-{index}"),
                            b"complete-model",
                        ))
                    },
                )
                .unwrap()
            }));
        }

        let paths: BTreeSet<PathBuf> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(paths.len(), 1);
        let target = paths.into_iter().next().unwrap();
        let key = target.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(cache_entry_is_valid(&target, &key).unwrap());
    }

    #[test]
    fn incomplete_or_size_damaged_target_is_recompiled() {
        let root = TestRoot::new("damaged");
        let source = fake_source(root.path());
        let cache = root.path().join("cache");
        let compiler_root = root.path().join("compiler-output");
        fs::create_dir(&compiler_root).unwrap();
        let key = derive_runtime_cache_key("model-1-artifact-deadbeef", "all", "macos-build-test")
            .unwrap();
        let target = cache
            .join("coreml-compiled")
            .join(format!("{key}.mlmodelc"));
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("model.mil"), b"partial").unwrap();

        let first = compile_cached_with(
            &source,
            &cache,
            "model-1-artifact-deadbeef",
            "all",
            "macos-build-test",
            |_| {
                Ok(fake_compiled(
                    &compiler_root,
                    "compiled-1",
                    b"first-complete",
                ))
            },
        )
        .unwrap();
        assert_eq!(first, target);
        assert!(cache_entry_is_valid(&target, &key).unwrap());

        fs::write(target.join("model.mil"), b"x").unwrap();
        assert!(!cache_entry_is_valid(&target, &key).unwrap());
        compile_cached_with(
            &source,
            &cache,
            "model-1-artifact-deadbeef",
            "all",
            "macos-build-test",
            |_| {
                Ok(fake_compiled(
                    &compiler_root,
                    "compiled-2",
                    b"second-complete",
                ))
            },
        )
        .unwrap();
        assert_eq!(
            fs::read(target.join("model.mil")).unwrap(),
            b"second-complete"
        );
        assert!(cache_entry_is_valid(&target, &key).unwrap());
    }

    #[test]
    fn failed_copy_removes_unique_staging_directory() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("cleanup");
        let source = fake_source(root.path());
        let cache = root.path().join("cache");
        let compiled = fake_compiled(root.path(), "compiled", b"model");
        let cache_key =
            derive_runtime_cache_key("model-1-artifact-deadbeef", "all", "macos-build-test")
                .unwrap();
        let stale = cache
            .join("coreml-compiled")
            .join(format!("{cache_key}.partial.dead-process"));
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("partial"), b"incomplete").unwrap();
        symlink(
            root.path().join("missing"),
            compiled.join("unsupported-link"),
        )
        .unwrap();

        let error = compile_cached_with(
            &source,
            &cache,
            "model-1-artifact-deadbeef",
            "all",
            "macos-build-test",
            |_| Ok(compiled),
        )
        .unwrap_err();
        assert!(error.to_string().contains("symbolic link"));
        assert!(!stale.exists());
        let leftovers: Vec<_> = fs::read_dir(cache.join("coreml-compiled"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".partial."))
            .collect();
        assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
    }

    #[test]
    fn atomic_publication_never_replaces_existing_target() {
        let root = TestRoot::new("no-replace");
        let source = root.path().join("staging");
        let target = root.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(source.join("sentinel"), b"new").unwrap();
        fs::write(target.join("sentinel"), b"old").unwrap();

        let error = atomic_publish_no_replace(&source, &target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"old");
        assert_eq!(fs::read(source.join("sentinel")).unwrap(), b"new");
    }

    #[test]
    fn compilation_uses_a_separate_cache_when_artifact_tree_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreMode {
            path: PathBuf,
            mode: u32,
        }

        impl Drop for RestoreMode {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
            }
        }

        let root = TestRoot::new("read-only-artifact");
        let source = fake_source(root.path());
        let artifact_root = source.parent().unwrap().to_path_buf();
        let original_mode = fs::metadata(&artifact_root).unwrap().permissions().mode();
        fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o555)).unwrap();
        let _restore = RestoreMode {
            path: artifact_root.clone(),
            mode: original_mode,
        };
        let before: BTreeSet<_> = fs::read_dir(&artifact_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let compiler_root = root.path().join("compiler-output");
        fs::create_dir(&compiler_root).unwrap();

        let output = compile_cached_with(
            &source,
            &root.path().join("cache"),
            "model-1-artifact-deadbeef",
            "all",
            "macos-build-test",
            |_| Ok(fake_compiled(&compiler_root, "compiled", b"complete")),
        )
        .unwrap();
        let after: BTreeSet<_> = fs::read_dir(&artifact_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();

        assert_eq!(before, after);
        assert!(!output.starts_with(&artifact_root));
    }

    #[test]
    #[ignore = "requires VALLE_COREML_REAL_PACKAGE to point to a local .mlpackage"]
    fn real_coreml_package_compiles_loads_and_reuses_cache() {
        let Some(source) = std::env::var_os("VALLE_COREML_REAL_PACKAGE") else {
            return;
        };
        let root = TestRoot::new("real-coreml");
        let first = CoreMlSession::load(
            Path::new(&source),
            root.path(),
            "real-smoke-source-digest",
            CoreMlComputeUnits::CpuOnly,
        )
        .unwrap();
        let compiled = first.compiled_path().to_path_buf();
        assert!(compiled.is_dir());
        drop(first);

        let second = CoreMlSession::load(
            Path::new(&source),
            root.path(),
            "real-smoke-source-digest",
            CoreMlComputeUnits::CpuOnly,
        )
        .unwrap();
        assert_eq!(second.compiled_path(), compiled);
    }

    #[test]
    #[ignore = "helper process for the lock-release test"]
    fn cache_lock_child_process() {
        let Ok(lock_path) = std::env::var("VALLE_COREML_TEST_LOCK_PATH") else {
            return;
        };
        let ready_path = std::env::var("VALLE_COREML_TEST_READY_PATH").unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();
        fs::write(ready_path, b"locked").unwrap();
        thread::sleep(Duration::from_millis(500));
        std::process::exit(23);
    }

    #[test]
    fn file_lock_is_released_after_abrupt_process_exit() {
        static CHILD_TEST_NAME: OnceLock<String> = OnceLock::new();

        let root = TestRoot::new("crash-lock");
        let lock_path = root.path().join("entry.lock");
        let ready_path = root.path().join("ready");
        let current_exe = std::env::current_exe().unwrap();
        let child_test_name = CHILD_TEST_NAME.get_or_init(|| "cache_lock_child_process".into());
        let mut child = Command::new(current_exe)
            .arg("--ignored")
            .arg(child_test_name)
            .arg("--nocapture")
            .env("VALLE_COREML_TEST_LOCK_PATH", &lock_path)
            .env("VALLE_COREML_TEST_READY_PATH", &ready_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready_path.exists(), "child never acquired the cache lock");

        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        assert!(
            FileExt::try_lock_exclusive(&lock).is_err(),
            "second process acquired a lock while the child still held it"
        );
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(23));
        FileExt::try_lock_exclusive(&lock)
            .expect("OS must release the advisory lock when a process exits abruptly");
    }
}
