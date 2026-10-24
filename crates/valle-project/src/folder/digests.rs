//! Disposable snapshot digest cache. Callers hold the project's operation lock.
use super::{CapturedInput, Project, absolute, digest_file, write_json};
use crate::{ContentDigest, files::FileStamp};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct CachedDigest {
    stamp: FileStamp,
    digest: ContentDigest,
}

pub struct DigestCache {
    path: PathBuf,
    entries: BTreeMap<String, CachedDigest>,
    changed: bool,
}

impl DigestCache {
    pub fn open(project: &Project) -> Self {
        let path = project.root.join(".valle/digests.json");
        let entries = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            changed: false,
        }
    }

    pub fn capture(&mut self, path: &Path) -> Result<CapturedInput> {
        let path = absolute(path)?;
        let key = path.to_string_lossy().into_owned();
        let stamp = FileStamp::read(&path)?;
        let digest = if let Some(cached) = self.entries.get(&key).filter(|e| e.stamp == stamp) {
            cached.digest
        } else {
            let digest = digest_file(&path)?;
            if FileStamp::read(&path)? != stamp {
                bail!(
                    "media changed while saving snapshot: {}; retry",
                    path.display()
                );
            }
            self.entries.insert(
                key,
                CachedDigest {
                    stamp: stamp.clone(),
                    digest,
                },
            );
            self.changed = true;
            digest
        };
        let mut input = CapturedInput::media(path, digest);
        input.stamp = Some(stamp);
        Ok(input)
    }

    pub fn persist(self) -> Result<()> {
        if self.changed {
            write_json(&self.path, &self.entries, true)?;
        }
        Ok(())
    }
}
