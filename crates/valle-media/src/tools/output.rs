use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use super::{ToolError, ToolErrorCode};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One staged single-file output. Dropping an uncommitted transaction removes its private staging
/// directory and every partial artifact inside it.
pub(super) struct FileOutputTransaction {
    target: PathBuf,
    staging_root: PathBuf,
    staging: PathBuf,
    target_existed_at_prepare: bool,
    committed: bool,
}

/// One staged directory output. Multi-artifact tools publish the complete directory with one
/// same-filesystem rename; an existing target is never overwritten.
#[cfg(any(test, feature = "tool-separate"))]
#[derive(Debug)]
pub(super) struct DirectoryOutputTransaction {
    target: PathBuf,
    staging: PathBuf,
    committed: bool,
}

/// Return whether two paths identify the same filesystem entry, including through symlinks.
///
/// A not-yet-created path is normalized through its existing parent so callers can reject an
/// output that would overwrite an input before any staging file is created.
pub(super) fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (path_identity(left), path_identity(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn path_identity(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let parent = path.parent().filter(|path| !path.as_os_str().is_empty())?;
    let file_name = path.file_name()?;
    std::fs::canonicalize(parent)
        .ok()
        .map(|parent| parent.join(file_name))
}

#[cfg(any(test, feature = "tool-separate"))]
fn entry_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn output_metadata(path: &Path) -> Result<Option<std::fs::Metadata>, ToolError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("inspect output {}: {error}", path.display()),
        )),
    }
}

fn create_private_staging_directory(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

impl FileOutputTransaction {
    #[allow(dead_code)] // Used only by model-tool builds; output transactions also serve core tests.
    pub fn validate_target(target: &Path, overwrite: bool) -> Result<(), ToolError> {
        validate_file_output_target(target, overwrite).map(drop)
    }

    pub fn new(target: &Path, overwrite: bool) -> Result<Self, ToolError> {
        let target_existed_at_prepare = validate_file_output_target(target, overwrite)?;
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stem = target
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("output");
        let mut staging_root = None;
        for _ in 0..64 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{stem}.valle-staging-{}-{sequence}",
                std::process::id()
            ));
            match create_private_staging_directory(&candidate) {
                Ok(()) => {
                    staging_root = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ToolError::new(
                        ToolErrorCode::OutputValidationFailed,
                        format!("reserve staging directory {}: {error}", candidate.display()),
                    ));
                }
            }
        }
        let staging_root = staging_root.ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!(
                    "could not reserve a staging directory for {}",
                    target.display()
                ),
            )
        })?;
        let staging = staging_root.join(
            target
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("output")),
        );
        Ok(Self {
            target: target.to_owned(),
            staging_root,
            staging,
            target_existed_at_prepare,
            committed: false,
        })
    }

    pub fn staging_path(&self) -> &Path {
        &self.staging
    }

    pub fn commit(mut self) -> Result<PathBuf, ToolError> {
        if !self.staging.is_file() {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("staged output is missing: {}", self.staging.display()),
            ));
        }

        if self.target_existed_at_prepare {
            let target_metadata = output_metadata(&self.target)?.ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "output disappeared while the job was running: {}",
                        self.target.display()
                    ),
                )
            })?;
            if !target_metadata.file_type().is_file() {
                return Err(ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "refusing to replace a non-regular output: {}",
                        self.target.display()
                    ),
                ));
            }
            replace_file(&self.staging, &self.target)?;
        } else {
            move_file_no_replace(&self.staging, &self.target).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    ToolError::new(
                        ToolErrorCode::OutputValidationFailed,
                        format!(
                            "output appeared while the job was running: {}",
                            self.target.display()
                        ),
                    )
                } else {
                    ToolError::new(
                        ToolErrorCode::OutputValidationFailed,
                        format!(
                            "atomically commit {} to {} without replacing an existing output: {error}",
                            self.staging.display(),
                            self.target.display()
                        ),
                    )
                }
            })?;
        }
        self.committed = true;
        let _ = std::fs::remove_dir(&self.staging_root);
        Ok(self.target.clone())
    }
}

fn validate_file_output_target(target: &Path, overwrite: bool) -> Result<bool, ToolError> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(ToolError::invalid_input(format!(
            "output parent does not exist: {}",
            parent.display()
        )));
    }
    let target_metadata = output_metadata(target)?;
    if target_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(ToolError::invalid_input(format!(
            "output must not be a symbolic link: {}",
            target.display()
        )));
    }
    if target_metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.file_type().is_file())
    {
        return Err(ToolError::invalid_input(format!(
            "single-file output must be a regular file: {}",
            target.display()
        )));
    }
    let target_existed_at_prepare = target_metadata.is_some();
    if target_existed_at_prepare && !overwrite {
        return Err(ToolError::invalid_input(format!(
            "output already exists: {} (enable overwrite or choose a new path)",
            target.display()
        )));
    }

    Ok(target_existed_at_prepare)
}

#[cfg(any(test, feature = "tool-separate"))]
fn validate_directory_output_target(target: &Path) -> Result<(), ToolError> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(ToolError::invalid_input(format!(
            "output parent does not exist: {}",
            parent.display()
        )));
    }
    if entry_exists(target) {
        return Err(ToolError::invalid_input(format!(
            "output directory already exists: {} (choose a new directory)",
            target.display()
        )));
    }
    Ok(())
}

#[cfg(any(test, feature = "tool-separate"))]
impl DirectoryOutputTransaction {
    #[allow(dead_code)] // Used by the separately feature-gated source-separation workflow.
    pub fn validate_target(target: &Path) -> Result<(), ToolError> {
        validate_directory_output_target(target)
    }

    pub fn new(target: &Path) -> Result<Self, ToolError> {
        validate_directory_output_target(target)?;
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("output");
        let mut staging = None;
        for _ in 0..64 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{name}.valle-staging-{}-{sequence}",
                std::process::id()
            ));
            match create_private_staging_directory(&candidate) {
                Ok(()) => {
                    staging = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ToolError::new(
                        ToolErrorCode::OutputValidationFailed,
                        format!("create staging directory {}: {error}", candidate.display()),
                    ));
                }
            }
        }
        let staging = staging.ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!(
                    "could not reserve a staging directory for {}",
                    target.display()
                ),
            )
        })?;
        Ok(Self {
            target: target.to_owned(),
            staging,
            committed: false,
        })
    }

    pub fn staging_path(&self) -> &Path {
        &self.staging
    }

    pub fn commit(mut self) -> Result<PathBuf, ToolError> {
        if !self.staging.is_dir() {
            return Err(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!(
                    "staged output directory is missing: {}",
                    self.staging.display()
                ),
            ));
        }
        move_file_no_replace(&self.staging, &self.target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "output directory appeared while the job was running: {}",
                        self.target.display()
                    ),
                )
            } else {
                ToolError::new(
                    ToolErrorCode::OutputValidationFailed,
                    format!(
                        "atomically commit output directory {} to {} without replacing an existing output: {error}",
                        self.staging.display(),
                        self.target.display()
                    ),
                )
            }
        })?;
        self.committed = true;
        Ok(self.target.clone())
    }
}

#[cfg(any(test, feature = "tool-separate"))]
impl Drop for DirectoryOutputTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_to_c_string(path: &Path) -> Result<std::ffi::CString, std::io::Error> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path contains an interior NUL byte: {}", path.display()),
        )
    })
}

#[cfg(target_os = "macos")]
fn move_file_no_replace(staging: &Path, target: &Path) -> Result<(), std::io::Error> {
    use std::os::raw::{c_char, c_int, c_uint};

    const RENAME_EXCL: c_uint = 0x0000_0004;

    unsafe extern "C" {
        fn renamex_np(old: *const c_char, new: *const c_char, flags: c_uint) -> c_int;
    }

    let staging = path_to_c_string(staging)?;
    let target = path_to_c_string(target)?;
    // SAFETY: both paths are valid NUL-terminated byte strings that remain alive for the call.
    // RENAME_EXCL asks the kernel to fail atomically if the destination already exists.
    let renamed = unsafe { renamex_np(staging.as_ptr(), target.as_ptr(), RENAME_EXCL) };
    if renamed == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn move_file_no_replace(staging: &Path, target: &Path) -> Result<(), std::io::Error> {
    use std::os::raw::{c_char, c_int, c_uint};

    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;

    unsafe extern "C" {
        fn renameat2(
            old_dir_fd: c_int,
            old: *const c_char,
            new_dir_fd: c_int,
            new: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }

    let staging = path_to_c_string(staging)?;
    let target = path_to_c_string(target)?;
    // SAFETY: both paths are valid NUL-terminated byte strings that remain alive for the call.
    // RENAME_NOREPLACE asks the kernel to fail atomically if the destination already exists.
    let renamed = unsafe {
        renameat2(
            AT_FDCWD,
            staging.as_ptr(),
            AT_FDCWD,
            target.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn move_file_no_replace(staging: &Path, target: &Path) -> Result<(), std::io::Error> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let staging_wide: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain alive for the call.
    // MOVEFILE_REPLACE_EXISTING is deliberately absent, so an existing destination wins.
    let moved = unsafe {
        MoveFileExW(
            staging_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn move_file_no_replace(_staging: &Path, _target: &Path) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace move is unsupported on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn replace_file(staging: &Path, target: &Path) -> Result<(), ToolError> {
    std::fs::rename(staging, target).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "atomically replace {} with {}: {error}",
                target.display(),
                staging.display()
            ),
        )
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn replace_file(_staging: &Path, target: &Path) -> Result<(), ToolError> {
    Err(ToolError::new(
        ToolErrorCode::OutputValidationFailed,
        format!(
            "atomic single-file replacement is unsupported on this platform: {}",
            target.display()
        ),
    ))
}

#[cfg(windows)]
fn replace_file(staging: &Path, target: &Path) -> Result<(), ToolError> {
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

    let staging_wide: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain alive for the call;
    // optional backup/exclusion pointers are null as required by ReplaceFileW.
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            staging_wide.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "atomically replace {}: {}",
                target.display(),
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(())
}

impl Drop for FileOutputTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.staging_root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_namespace_is_reserved_for_the_transaction() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let transaction = FileOutputTransaction::new(&root.join("out.json"), false).unwrap();

        assert!(transaction.staging_root.is_dir());
        assert_eq!(transaction.staging_path().extension().unwrap(), "json");
        assert!(!transaction.staging_path().exists());
        let error = std::fs::create_dir(&transaction.staging_root).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

        let staging_root = transaction.staging_root.clone();
        drop(transaction);
        assert!(!staging_root.exists());
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn cancellation_drop_cleans_staging_and_preserves_an_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();
        let (staging_root, staging) = {
            let transaction = FileOutputTransaction::new(&target, true).unwrap();
            std::fs::write(transaction.staging_path(), b"partial").unwrap();
            (
                transaction.staging_root.clone(),
                transaction.staging_path().to_owned(),
            )
        };

        assert_eq!(std::fs::read(&target).unwrap(), b"old owner");
        assert!(!staging.exists());
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn drop_recursively_cleans_unexpected_staging_residue() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let transaction = FileOutputTransaction::new(&root.join("out.json"), false).unwrap();
        let staging_root = transaction.staging_root.clone();
        std::fs::write(transaction.staging_path(), b"partial").unwrap();
        std::fs::create_dir(staging_root.join("unexpected")).unwrap();
        std::fs::write(staging_root.join("unexpected/residue"), b"partial").unwrap();

        drop(transaction);

        assert!(!staging_root.exists());
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn commit_publishes_a_complete_file() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        let transaction = FileOutputTransaction::new(&target, false).unwrap();
        let staging_root = transaction.staging_root.clone();
        std::fs::write(transaction.staging_path(), b"complete").unwrap();
        assert_eq!(transaction.commit().unwrap(), target);
        assert_eq!(std::fs::read(&target).unwrap(), b"complete");
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn existing_target_without_overwrite_is_rejected_before_staging() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();

        let error = FileOutputTransaction::new(&target, false)
            .err()
            .expect("overwrite-disabled output must be rejected");

        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert_eq!(std::fs::read(&target).unwrap(), b"old owner");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn overwrite_atomically_replaces_a_target_that_existed_at_prepare() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();

        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        std::fs::write(transaction.staging_path(), b"new owner").unwrap();
        assert_eq!(transaction.commit().unwrap(), target);
        assert_eq!(std::fs::read(&target).unwrap(), b"new owner");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failed_commit_cleans_staging_and_preserves_an_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();
        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        let staging_root = transaction.staging_root.clone();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert_eq!(std::fs::read(&target).unwrap(), b"old owner");
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn overwrite_fails_closed_if_the_original_target_disappears() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();
        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        let staging_root = transaction.staging_root.clone();
        std::fs::write(transaction.staging_path(), b"staged owner").unwrap();
        std::fs::remove_file(&target).unwrap();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert!(!target.exists());
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn overwrite_rejects_a_preexisting_directory() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::create_dir(&target).unwrap();

        let error = FileOutputTransaction::new(&target, true)
            .err()
            .expect("a directory must not be accepted as a single-file output");

        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert!(target.is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn overwrite_fails_closed_if_the_target_changes_to_a_directory() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        std::fs::write(&target, b"old owner").unwrap();
        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        let staging_root = transaction.staging_root.clone();
        std::fs::write(transaction.staging_path(), b"staged owner").unwrap();
        std::fs::remove_file(&target).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("owner.txt"), b"concurrent owner").unwrap();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert_eq!(
            std::fs::read(target.join("owner.txt")).unwrap(),
            b"concurrent owner"
        );
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commit_never_overwrites_a_target_that_appeared_after_prepare() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("out.json");
        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        std::fs::write(transaction.staging_path(), b"staged").unwrap();
        std::fs::write(&target, b"new owner").unwrap();
        assert!(transaction.commit().is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"new owner");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_rejects_a_preexisting_target_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let referent = root.join("referent.json");
        let target = root.join("out.json");
        std::fs::write(&referent, b"protected").unwrap();
        symlink(&referent, &target).unwrap();

        let error = FileOutputTransaction::new(&target, true)
            .err()
            .expect("a symlink target must be rejected");

        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&referent).unwrap(), b"protected");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn commit_does_not_follow_or_replace_a_late_target_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let referent = root.join("referent.json");
        let target = root.join("out.json");
        std::fs::write(&referent, b"protected").unwrap();
        let transaction = FileOutputTransaction::new(&target, true).unwrap();
        let staging_root = transaction.staging_root.clone();
        std::fs::write(transaction.staging_path(), b"staged").unwrap();
        symlink(&referent, &target).unwrap();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&referent).unwrap(), b"protected");
        assert!(!staging_root.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn atomic_no_replace_resolves_concurrent_claims_without_data_loss() {
        use std::{
            io::Write as _,
            sync::{Arc, Barrier},
        };

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();

        for iteration in 0..64 {
            let target = root.join(format!("out-{iteration}.json"));
            let transaction = FileOutputTransaction::new(&target, false).unwrap();
            let staging_root = transaction.staging_root.clone();
            std::fs::write(transaction.staging_path(), b"staged owner").unwrap();
            let barrier = Arc::new(Barrier::new(2));

            let (commit_result, claim_result) = std::thread::scope(|scope| {
                let commit_barrier = Arc::clone(&barrier);
                let commit = scope.spawn(move || {
                    commit_barrier.wait();
                    transaction.commit()
                });
                let claim_barrier = Arc::clone(&barrier);
                let claim_target = target.clone();
                let claim = scope.spawn(move || {
                    claim_barrier.wait();
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(claim_target)?;
                    file.write_all(b"concurrent owner")
                });
                (commit.join().unwrap(), claim.join().unwrap())
            });

            match (commit_result, claim_result) {
                (Ok(committed), Err(error)) => {
                    assert_eq!(committed, target);
                    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                    assert_eq!(std::fs::read(&target).unwrap(), b"staged owner");
                }
                (Err(error), Ok(())) => {
                    assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
                    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent owner");
                }
                (commit, claim) => {
                    panic!("unexpected race result: commit={commit:?}, claim={claim:?}")
                }
            }
            assert!(!staging_root.exists());
            std::fs::remove_file(&target).unwrap();
        }

        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn directory_commit_publishes_the_complete_set_at_once() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("stems");
        let transaction = DirectoryOutputTransaction::new(&target).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(transaction.staging_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        std::fs::write(transaction.staging_path().join("vocals.wav"), b"vocals").unwrap();
        std::fs::write(
            transaction.staging_path().join("instrumental.wav"),
            b"instrumental",
        )
        .unwrap();
        assert!(!target.exists());
        assert_eq!(transaction.commit().unwrap(), target);
        assert_eq!(std::fs::read(target.join("vocals.wav")).unwrap(), b"vocals");
        assert_eq!(
            std::fs::read(target.join("instrumental.wav")).unwrap(),
            b"instrumental"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn directory_transaction_never_overwrites_an_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("stems");
        std::fs::create_dir(&target).unwrap();
        assert!(DirectoryOutputTransaction::new(&target).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn directory_commit_never_overwrites_a_target_that_appeared_after_prepare() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("stems");
        let transaction = DirectoryOutputTransaction::new(&target).unwrap();
        let staging = transaction.staging_path().to_owned();
        std::fs::write(staging.join("vocals.wav"), b"staged").unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("owner.txt"), b"late owner").unwrap();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert_eq!(
            std::fs::read(target.join("owner.txt")).unwrap(),
            b"late owner"
        );
        assert!(!staging.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_commit_does_not_follow_or_replace_a_late_target_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let referent = root.join("protected-stems");
        let target = root.join("stems");
        std::fs::create_dir(&referent).unwrap();
        std::fs::write(referent.join("owner.txt"), b"protected").unwrap();
        let transaction = DirectoryOutputTransaction::new(&target).unwrap();
        let staging = transaction.staging_path().to_owned();
        std::fs::write(staging.join("vocals.wav"), b"staged").unwrap();
        symlink(&referent, &target).unwrap();

        let error = transaction.commit().unwrap_err();

        assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read(referent.join("owner.txt")).unwrap(),
            b"protected"
        );
        assert!(!staging.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn directory_atomic_no_replace_resolves_concurrent_claims() {
        use std::sync::{Arc, Barrier};

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();

        for iteration in 0..32 {
            let target = root.join(format!("stems-{iteration}"));
            let transaction = DirectoryOutputTransaction::new(&target).unwrap();
            let staging = transaction.staging_path().to_owned();
            std::fs::write(transaction.staging_path().join("vocals.wav"), b"staged").unwrap();
            let barrier = Arc::new(Barrier::new(2));

            let (commit_result, claim_result) = std::thread::scope(|scope| {
                let commit_barrier = Arc::clone(&barrier);
                let commit = scope.spawn(move || {
                    commit_barrier.wait();
                    transaction.commit()
                });
                let claim_barrier = Arc::clone(&barrier);
                let claim_target = target.clone();
                let claim = scope.spawn(move || {
                    claim_barrier.wait();
                    std::fs::create_dir(&claim_target)?;
                    std::fs::write(claim_target.join("owner.txt"), b"concurrent owner")
                });
                (commit.join().unwrap(), claim.join().unwrap())
            });

            match (commit_result, claim_result) {
                (Ok(committed), Err(error)) => {
                    assert_eq!(committed, target);
                    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                    assert_eq!(std::fs::read(target.join("vocals.wav")).unwrap(), b"staged");
                }
                (Err(error), Ok(())) => {
                    assert_eq!(error.code, ToolErrorCode::OutputValidationFailed);
                    assert_eq!(
                        std::fs::read(target.join("owner.txt")).unwrap(),
                        b"concurrent owner"
                    );
                }
                (commit, claim) => {
                    panic!("unexpected directory race result: commit={commit:?}, claim={claim:?}")
                }
            }
            assert!(!staging.exists());
            std::fs::remove_dir_all(&target).unwrap();
        }

        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn dropping_directory_transaction_removes_partial_outputs() {
        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let staging = {
            let transaction = DirectoryOutputTransaction::new(&root.join("stems")).unwrap();
            std::fs::write(transaction.staging_path().join("vocals.wav"), b"partial").unwrap();
            transaction.staging_path().to_owned()
        };
        assert!(!staging.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn same_file_detection_follows_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "valle-media-output-{}-{}",
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.wav");
        let alias = root.join("alias.wav");
        std::fs::write(&source, b"audio").unwrap();
        symlink(&source, &alias).unwrap();
        assert!(paths_refer_to_same_file(&source, &alias));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
