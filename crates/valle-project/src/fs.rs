//! Durable directory-entry publication shared by project and asset stores.

pub(crate) fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    let directory = std::fs::File::open(path)?;
    #[cfg(windows)]
    let directory = {
        use std::os::windows::fs::OpenOptionsExt;

        // Windows requires backup semantics to open a directory and write
        // access for FlushFileBuffers, which implements File::sync_all.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
    };
    #[cfg(any(unix, windows))]
    return directory.sync_all();
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "durable directory publication is unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn directory_publication_is_synced_and_missing_directories_fail() {
        // Keep this module std-only so CI can probe the native filesystem before
        // compiling the workspace's dependencies.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "valle-directory-sync-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("staging");
        std::fs::write(&source, b"complete").unwrap();
        std::fs::rename(source, directory.join("published")).unwrap();
        let synced = super::sync_directory(&directory);
        let missing = super::sync_directory(&directory.join("missing"));
        std::fs::remove_dir_all(&directory).unwrap();
        synced.unwrap();
        assert!(missing.is_err());
    }
}
