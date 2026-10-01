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
    audio: Mutex<BTreeMap<String, Arc<[u8]>>>,
    // A reload/edit must not prepare several multi-gigabyte packages simultaneously.
    pub prepare: Mutex<FrozenMediaCache>,
}

impl PreviewStore {
    /// Canonical analysis input: the same FFmpeg 48 kHz mono decode used by CLI.
    /// Only already-frozen preview resources can be decoded through this route.
    pub fn audio_pcm(&self, digest: &str) -> Result<Option<Arc<[u8]>>> {
        let file = self
            .files
            .read()
            .map_err(|_| anyhow::anyhow!("preview files poisoned"))?
            .get(digest)
            .cloned();
        let Some(file) = file else {
            return Ok(None);
        };
        let mut cache = self
            .audio
            .lock()
            .map_err(|_| anyhow::anyhow!("audio cache poisoned"))?;
        if let Some(bytes) = cache.get(digest) {
            return Ok(Some(Arc::clone(bytes)));
        }
        let mut temporary = None;
        let path = match &file {
            PreviewFile::File { path, .. } => path.clone(),
            PreviewFile::Bytes(bytes) => {
                let temp = tempfile::NamedTempFile::new()?;
                std::fs::write(temp.path(), bytes)?;
                let path = temp.path().to_owned();
                temporary = Some(temp);
                path
            }
        };
        let samples = valle_media::codec::decode_audio_mono_f32(&path, 48_000)?;
        drop(temporary);
        let bytes: Arc<[u8]> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>()
            .into();
        const MAX_CACHED_BYTES: usize = 128 * 1024 * 1024;
        while !cache.is_empty()
            && cache.values().map(|v| v.len()).sum::<usize>() + bytes.len() > MAX_CACHED_BYTES
        {
            cache.pop_first();
        }
        if bytes.len() <= MAX_CACHED_BYTES {
            cache.insert(digest.to_owned(), Arc::clone(&bytes));
        }
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_pcm_uses_cli_decoder_and_reuses_frozen_bytes() {
        let rate = 48_000_u32;
        let samples: Vec<i16> = (0..4800).map(|i| ((i % 31) as i16 - 15) * 1000).collect();
        let payload: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + payload.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((payload.len() as u32).to_le_bytes());
        wav.extend(payload);
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), &wav).unwrap();
        let expected = valle_media::codec::decode_audio_mono_f32(source.path(), rate).unwrap();
        let digest = ContentDigest::of_bytes(&wav).to_wire();
        let store = PreviewStore::default();
        store.files.write().unwrap().replace(BTreeMap::from([(
            digest.clone(),
            PreviewFile::Bytes(wav.into()),
        )]));
        let first = store.audio_pcm(&digest).unwrap().unwrap();
        let second = store.audio_pcm(&digest).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            &*first,
            expected
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert!(store.audio_pcm("unknown").unwrap().is_none());
        // Eviction from the frozen resource closure also revokes cached PCM access.
        store.files.write().unwrap().replace(BTreeMap::new());
        store.files.write().unwrap().replace(BTreeMap::new());
        assert!(store.audio_pcm(&digest).unwrap().is_none());
    }

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
