//! Atomic publication through temporary files, fsync, and rename. Crash checkpoints use
//! `<label>-before-rename` and `<label>-after-rename`.

use std::{ffi::OsString, io::Write, path::Path};

use crate::assets::crash::maybe_crash;
use crate::assets::report::{AssetsError, Result};

/// Atomically write bytes, fsync the file, rename, then fsync the directory.
pub fn write_atomic(path: &Path, bytes: &[u8], crash_label: &str) -> Result<()> {
    stage_atomic(path, crash_label, |staged| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staged)?;
        file.write_all(bytes)?;
        Ok(())
    })
}

/// Stage a unique file in the destination directory before atomic publication. The callback must
/// use exclusive creation, supporting both byte writes and clonefile/reflink. A race after
/// releasing the temporary reservation must fail rather than overwrite another file.
pub(crate) fn stage_atomic(
    path: &Path,
    crash_label: &str,
    populate: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| AssetsError::io(format!("no parent directory: {}", path.display())))?;
    std::fs::create_dir_all(dir)?;
    let mut prefix = OsString::from(".");
    prefix.push(path.file_name().unwrap_or_default());
    prefix.push(".tmp-");
    let reservation = tempfile::Builder::new().prefix(&prefix).tempfile_in(dir)?;
    let (reservation_file, temporary_path) = reservation.into_parts();
    drop(reservation_file);
    std::fs::remove_file(&temporary_path)?;
    populate(&temporary_path)?;

    let metadata = std::fs::symlink_metadata(&temporary_path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(AssetsError::io(format!(
            "staged result is not a regular file: {}",
            temporary_path.display()
        )));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary_path)?;
    file.sync_all()?;
    maybe_crash(&format!("{crash_label}-before-rename"));
    let temporary = tempfile::NamedTempFile::from_parts(file, temporary_path);
    temporary
        .persist(path)
        .map_err(|error| AssetsError::from(error.error))?;
    sync_dir(dir)?;
    maybe_crash(&format!("{crash_label}-after-rename"));
    Ok(())
}

fn sync_dir(dir: &Path) -> Result<()> {
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn write_atomic_leaves_no_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("deep").join("a.json");
        write_atomic(&target, b"{}", "meta").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_does_not_follow_preexisting_or_destination_symlinks() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("data");
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("meta.json");

        let destination_target = temporary.path().join("destination-target");
        std::fs::write(&destination_target, b"destination sentinel").unwrap();
        symlink(&destination_target, &target).unwrap();

        let preexisting_target = temporary.path().join("preexisting-target");
        std::fs::write(&preexisting_target, b"preexisting sentinel").unwrap();
        let preexisting_temp =
            target.with_file_name(format!("meta.json.tmp{}", std::process::id()));
        symlink(&preexisting_target, preexisting_temp).unwrap();

        write_atomic(&target, b"new truth", "meta").unwrap();

        assert_eq!(
            std::fs::read(&destination_target).unwrap(),
            b"destination sentinel"
        );
        assert_eq!(
            std::fs::read(&preexisting_target).unwrap(),
            b"preexisting sentinel"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new truth");
        assert!(
            !std::fs::symlink_metadata(target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn concurrent_writers_publish_only_complete_files() {
        const WRITERS: usize = 12;

        let temporary = tempfile::tempdir().unwrap();
        let target = Arc::new(temporary.path().join("state.json"));
        let barrier = Arc::new(Barrier::new(WRITERS + 1));
        let payloads = (0..WRITERS)
            .map(|index| vec![u8::try_from(index).unwrap(); 32 * 1024])
            .collect::<Vec<_>>();
        let mut workers = Vec::new();

        for payload in payloads.iter().cloned() {
            let target = Arc::clone(&target);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                write_atomic(&target, &payload, "concurrent")
            }));
        }

        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let published = std::fs::read(target.as_ref()).unwrap();
        assert!(payloads.contains(&published));
        assert!(
            std::fs::read_dir(temporary.path())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".state.json.tmp-"))
        );
    }
}
