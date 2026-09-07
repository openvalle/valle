//! Serialize writes with `assets/.lock`. Readers do not lock. Acquisition times out after five
//! seconds; long-running analysis releases the lock between state updates.

use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

use fs2::FileExt;

use crate::assets::home::Home;
use crate::assets::report::{AssetsError, Result};

pub struct WriteLock {
    file: File,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

/// Acquire the write lock with a five-second timeout. Keep the lock file to avoid unlink/recreate
/// races.
pub fn acquire(home: &Home) -> Result<WriteLock> {
    acquire_path(&home.lock_path(), "asset write lock")
}

/// Shared flock acquisition. When nested, the database initialization lock is always innermost.
pub fn acquire_path(path: &std::path::Path, what: &str) -> Result<WriteLock> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        let metadata = std::fs::symlink_metadata(dir)?;
        if !metadata.file_type().is_dir() {
            return Err(AssetsError::io(format!(
                "{what} parent must be a real directory: {}",
                dir.display()
            )));
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(AssetsError::io(format!(
                "{what} path must be a regular file: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AssetsError::from(error)),
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(AssetsError::io(format!(
            "{what} path must be a regular file: {}",
            path.display()
        )));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(WriteLock { file }),
            Err(error) if lock_is_contended(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) if lock_is_contended(&error) => {
                return Err(AssetsError::locked(format!("another process held {what} beyond the five-second timeout"))
                    .with_hint("retry later; keep the persistent lock file in place while processes are running"));
            }
            Err(error) => return Err(AssetsError::from(error)),
        }
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}
