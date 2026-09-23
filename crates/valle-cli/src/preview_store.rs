//! Immutable preview resources. Large media stays on disk; in-flight requests own their files.
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, Weak},
};
use valle_motion::ContentDigest;

#[derive(Clone)]
pub(crate) enum PreviewFile {
    Bytes(Arc<[u8]>),
    File {
        path: PathBuf,
        _directory: Arc<tempfile::TempDir>,
    },
}

#[derive(PartialEq, Eq)]
struct SourceStamp {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl SourceStamp {
    fn read(metadata: std::fs::Metadata) -> Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

struct CachedMedia {
    stamp: SourceStamp,
    digest: ContentDigest,
    path: PathBuf,
    directory: Weak<tempfile::TempDir>,
}

/// Reuse unchanged source snapshots while preview generations or HTTP requests own them.
/// Weak owners let retired files be deleted without another cache eviction policy.
#[derive(Default)]
pub(crate) struct FrozenMediaCache {
    entries: BTreeMap<PathBuf, CachedMedia>,
}

impl FrozenMediaCache {
    pub fn get(&mut self, source: &Path) -> Result<(ContentDigest, PreviewFile)> {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};
        self.entries
            .retain(|_, entry| entry.directory.strong_count() > 0);
        let source = source.canonicalize()?;
        let mut input = std::fs::File::open(&source)?;
        let stamp = SourceStamp::read(input.metadata()?)?;
        if let Some(entry) = self.entries.get(&source) {
            if entry.stamp == stamp {
                if let Some(directory) = entry.directory.upgrade() {
                    return Ok((
                        entry.digest,
                        PreviewFile::File {
                            path: entry.path.clone(),
                            _directory: directory,
                        },
                    ));
                }
            }
        }
        let directory = Arc::new(tempfile::tempdir().context("freezing preview media")?);
        let mut output = tempfile::NamedTempFile::new_in(directory.path())?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        if stamp != SourceStamp::read(input.metadata()?)?
            || stamp != SourceStamp::read(std::fs::metadata(&source)?)?
        {
            bail!(
                "media changed while preparing preview: {}",
                source.display()
            );
        }
        let digest = ContentDigest::from_bytes(hash.finalize().into());
        let path = directory.path().join(digest.as_hex());
        output.persist(&path).map_err(|error| error.error)?;
        self.entries.insert(
            source,
            CachedMedia {
                stamp,
                digest,
                path: path.clone(),
                directory: Arc::downgrade(&directory),
            },
        );
        Ok((
            digest,
            PreviewFile::File {
                path,
                _directory: directory,
            },
        ))
    }
}

#[derive(Default)]
pub(crate) struct PreviewFiles {
    current: BTreeMap<String, PreviewFile>,
    previous: BTreeMap<String, PreviewFile>,
}

impl PreviewFiles {
    pub fn get(&self, digest: &str) -> Option<&PreviewFile> {
        self.current
            .get(digest)
            .or_else(|| self.previous.get(digest))
    }

    /// Keep the last successful preview alive while the browser admits its replacement.
    pub fn replace(&mut self, next: BTreeMap<String, PreviewFile>) {
        self.previous = std::mem::replace(&mut self.current, next);
    }
}

#[derive(Default)]
pub(crate) struct PreviewStore {
    pub files: RwLock<PreviewFiles>,
    // A reload/edit must not prepare several multi-gigabyte packages simultaneously.
    pub prepare: Mutex<FrozenMediaCache>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_media_reuses_snapshot_and_edits_preserve_old_generation() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let bytes = vec![42; 2 * 1024 * 1024 + 17];
        std::fs::write(source.path(), &bytes).unwrap();
        let mut cache = FrozenMediaCache::default();
        let (digest, first) = cache.get(source.path()).unwrap();
        assert_eq!(digest, ContentDigest::of_bytes(&bytes));
        let (_, second) = cache.get(source.path()).unwrap();
        let path = |file: &PreviewFile| match file {
            PreviewFile::File { path, .. } => path.clone(),
            _ => unreachable!(),
        };
        assert_eq!(path(&first), path(&second));
        // Same-size replacement must also invalidate the cache.
        let replacement = vec![43; bytes.len()];
        let new_source = tempfile::NamedTempFile::new_in(source.path().parent().unwrap()).unwrap();
        std::fs::write(new_source.path(), &replacement).unwrap();
        new_source.persist(source.path()).unwrap();
        let (updated, third) = cache.get(source.path()).unwrap();
        assert_ne!(digest, updated);
        assert_eq!(std::fs::read(path(&first)).unwrap(), bytes);
        assert_eq!(std::fs::read(path(&third)).unwrap(), replacement);
        let old_path = path(&first);
        drop(first);
        drop(second);
        assert!(!old_path.exists()); // Cache does not keep retired media alive.
    }

    #[test]
    fn generations_release_files_but_in_flight_requests_keep_them_alive() {
        let directory = Arc::new(tempfile::tempdir().unwrap());
        let path = directory.path().join("asset");
        std::fs::write(&path, b"frozen").unwrap();
        let mut store = PreviewFiles::default();
        store.replace(BTreeMap::from([(
            "a".into(),
            PreviewFile::File {
                path: path.clone(),
                _directory: directory,
            },
        )]));
        let request = store.get("a").unwrap().clone();
        store.replace(BTreeMap::new());
        assert!(store.get("a").is_some());
        store.replace(BTreeMap::new());
        assert!(store.get("a").is_none());
        assert!(path.exists());
        drop(request);
        assert!(!path.exists());
    }
}
