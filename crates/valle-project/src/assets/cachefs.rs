//! Disposable derived files under `~/.valle/cache/<area>/<hash>-<key>.<ext>`. Content and parameter
//! keys let projects, the library, and Studio share deterministic thumbnails, waveforms, and
//! proxies without using analysis slots.

use std::path::PathBuf;

use crate::assets::home::Home;
use crate::assets::report::{AssetsError, Result};

/// Derived-file path; omit the parameter suffix when the key is empty.
pub fn cache_path(home: &Home, area: &str, hash: &str, key: &str, ext: &str) -> PathBuf {
    let name = if key.is_empty() {
        format!("{hash}.{ext}")
    } else {
        format!("{hash}-{key}.{ext}")
    };
    home.cache_dir().join(area).join(name)
}

/// Create the parent directory and return a derived-file path.
pub fn ensure_cache_path(
    home: &Home,
    area: &str,
    hash: &str,
    key: &str,
    ext: &str,
) -> Result<PathBuf> {
    let p = cache_path(home, area, hash, key, ext);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(p)
}

/// Delete derived files whose leading content hash has no live metadata. Return the deletion count.
pub fn gc_cache(home: &Home, live: &std::collections::HashSet<String>) -> Result<usize> {
    remove_gc_candidates(collect_gc_candidates(home, live)?)
}

/// Build a complete cache deletion plan without mutating the filesystem.
/// Every traversed component must be a real directory or regular file; a
/// symlink or unreadable entry aborts the whole plan.
pub(crate) fn collect_gc_candidates(
    home: &Home,
    live: &std::collections::HashSet<String>,
) -> Result<Vec<PathBuf>> {
    let root = home.cache_dir();
    let mut candidates = Vec::new();
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(AssetsError::io(format!(
                "cache GC root must be a real directory: {}",
                root.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidates),
        Err(error) => return Err(AssetsError::from(error)),
    }
    for area in std::fs::read_dir(&root)? {
        let area = area?;
        let kind = area.file_type()?;
        if kind.is_symlink() {
            return Err(AssetsError::io(format!(
                "refusing cache GC traversal through symlink {}",
                area.path().display()
            )));
        }
        if !kind.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(area.path())? {
            let f = f?;
            let kind = f.file_type()?;
            if kind.is_symlink() {
                return Err(AssetsError::io(format!(
                    "refusing cache GC traversal through symlink {}",
                    f.path().display()
                )));
            }
            if !kind.is_file() {
                continue;
            }
            let name = f.file_name().to_string_lossy().into_owned();
            let hash_part: String = name.chars().take(64).collect();
            let is_hash = hash_part.len() == 64 && hash_part.bytes().all(|b| b.is_ascii_hexdigit());
            if !is_hash || !live.contains(&hash_part) {
                candidates.push(f.path());
            }
        }
    }
    Ok(candidates)
}

pub(crate) fn remove_gc_candidates(candidates: Vec<PathBuf>) -> Result<usize> {
    let mut removed = 0usize;
    for path in candidates {
        std::fs::remove_file(path)?;
        removed += 1;
    }
    Ok(removed)
}
