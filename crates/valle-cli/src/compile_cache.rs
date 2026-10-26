//! Disposable Motion compiler results. Keys include compiler bytes and captured author inputs.
use anyhow::Result;
use std::{io::Write, path::Path, sync::Mutex};

pub(crate) const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
static WRITER: Mutex<()> = Mutex::new(());

pub(crate) fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn read(root: &Path, key: &str) -> Option<Vec<u8>> {
    if !valid_key(key) {
        return None;
    }
    let path = root.join(format!("{key}.json"));
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES as u64 {
        return None;
    }
    std::fs::read(path).ok()
}

pub(crate) fn write(root: &Path, key: &str, bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        valid_key(key) && bytes.len() <= MAX_ENTRY_BYTES,
        "invalid compiler cache entry"
    );
    let _guard = WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!("compiler cache poisoned"))?;
    std::fs::create_dir_all(root)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(bytes)?;
    temporary
        .persist(root.join(format!("{key}.json")))
        .map_err(|error| error.error)?;
    let mut entries = std::fs::read_dir(root)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let key = name.to_str()?.strip_suffix(".json")?;
            if !valid_key(key) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.0);
    let mut size: u64 = entries.iter().map(|entry| entry.1).sum();
    let mut count = entries.len();
    for (_, bytes, path) in entries {
        if size <= MAX_CACHE_BYTES && count <= MAX_ENTRIES {
            break;
        }
        std::fs::remove_file(path)?;
        size = size.saturating_sub(bytes);
        count -= 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_is_atomic_bounded_and_rejects_paths() {
        let directory = tempfile::tempdir().unwrap();
        assert!(write(directory.path(), "../outside", b"x").is_err());
        assert!(read(directory.path(), "../outside").is_none());
        for index in 0..MAX_ENTRIES + 2 {
            write(
                directory.path(),
                &format!("{index:064x}"),
                b"{\"artifact\":1}",
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            MAX_ENTRIES
        );
        let key = format!("{:064x}", MAX_ENTRIES + 1);
        assert_eq!(read(directory.path(), &key).unwrap(), b"{\"artifact\":1}");
        write(directory.path(), &key, b"replacement").unwrap();
        assert_eq!(read(directory.path(), &key).unwrap(), b"replacement");
        assert!(write(directory.path(), &key, &vec![0; MAX_ENTRY_BYTES + 1]).is_err());
    }
}
