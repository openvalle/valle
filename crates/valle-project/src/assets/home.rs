//! Asset storage paths under `~/.valle/assets/`. Metadata, analysis, and annotations are
//! authoritative; inexpensive derived files live in the shared cache. `VALLE_HOME` overrides the
//! application root.

use std::path::{Path, PathBuf};

use crate::assets::report::{AssetsError, Result};

/// Split a full hash into a two-character shard and suffix, distributing files across 256
/// directories without changing asset identity.
pub fn fanout(hash: &str) -> (&str, &str) {
    debug_assert!(
        hash.len() > 2,
        "fanout requires at least three hash characters"
    );
    hash.split_at(2)
}

/// Application home from `VALLE_HOME` or `~/.valle`.
#[derive(Debug, Clone)]
pub struct Home {
    root: PathBuf,
}

impl Home {
    /// Resolve the root from the environment without creating directories.
    pub fn resolve() -> Result<Home> {
        if let Some(v) = std::env::var_os("VALLE_HOME") {
            return Ok(Home {
                root: PathBuf::from(v),
            });
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            AssetsError::io("HOME is not set; set VALLE_HOME to locate the application directory")
        })?;
        Ok(Home {
            root: home.join(".valle"),
        })
    }

    /// Use an explicit root, typically for tests.
    pub fn at(root: impl Into<PathBuf>) -> Home {
        Home { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Asset library root.
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    /// Content-addressed blob directory.
    pub fn objects_dir(&self) -> PathBuf {
        self.assets_dir().join("objects")
    }

    /// Blob path retaining its extension for preview and decoding tools.
    pub fn object_path(&self, hash: &str, ext: Option<&str>) -> PathBuf {
        let (d, rest) = fanout(hash);
        let name = match ext {
            Some(e) => format!("{rest}.{e}"),
            None => rest.to_owned(),
        };
        self.objects_dir().join(d).join(name)
    }

    /// Authoritative asset metadata path.
    pub fn meta_path(&self, hash: &str) -> PathBuf {
        let (d, rest) = fanout(hash);
        self.assets_dir()
            .join("meta")
            .join(d)
            .join(format!("{rest}.json"))
    }

    pub fn meta_dir(&self) -> PathBuf {
        self.assets_dir().join("meta")
    }

    /// Analysis directory containing `<analyzer>@<version>.json` slots.
    pub fn analysis_dir(&self, hash: &str) -> PathBuf {
        let (d, rest) = fanout(hash);
        self.assets_dir().join("analysis").join(d).join(rest)
    }

    /// Annotation directory containing numbered JSON files.
    pub fn annotations_dir(&self, hash: &str) -> PathBuf {
        let (d, rest) = fanout(hash);
        self.assets_dir().join("annotations").join(d).join(rest)
    }

    /// Authoritative entity table.
    pub fn entities_path(&self) -> PathBuf {
        self.assets_dir().join("entities.json")
    }

    /// Disposable SQLite projection, recoverable by reindexing.
    pub fn index_db_path(&self) -> PathBuf {
        self.assets_dir().join("index.db")
    }

    /// Lock serializing write commands.
    pub fn lock_path(&self) -> PathBuf {
        self.assets_dir().join(".lock")
    }

    /// Separate database initialization lock, always acquired inside the write lock when both are
    /// needed.
    pub fn init_lock_path(&self) -> PathBuf {
        self.assets_dir().join(".init.lock")
    }

    /// Disposable cache shared by projects, the library, and Studio.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "3f8ac2e1aabbccdd0011223344556677aabbccdd0011223344556677aabbccdd";

    #[test]
    fn fanout_splits_at_two() {
        let (d, rest) = fanout(H);
        assert_eq!(d, "3f");
        assert_eq!(rest, &H[2..]);
    }

    #[test]
    fn paths_shape() {
        let home = Home::at("/tmp/vh");
        assert_eq!(
            home.object_path(H, Some("mp4")),
            PathBuf::from(format!("/tmp/vh/assets/objects/3f/{}.mp4", &H[2..]))
        );
        assert_eq!(
            home.object_path(H, None),
            PathBuf::from(format!("/tmp/vh/assets/objects/3f/{}", &H[2..]))
        );
        assert_eq!(
            home.meta_path(H),
            PathBuf::from(format!("/tmp/vh/assets/meta/3f/{}.json", &H[2..]))
        );
        assert_eq!(
            home.analysis_dir(H),
            PathBuf::from(format!("/tmp/vh/assets/analysis/3f/{}", &H[2..]))
        );
        assert_eq!(
            home.annotations_dir(H),
            PathBuf::from(format!("/tmp/vh/assets/annotations/3f/{}", &H[2..]))
        );
        assert_eq!(
            home.entities_path(),
            PathBuf::from("/tmp/vh/assets/entities.json")
        );
        assert_eq!(
            home.index_db_path(),
            PathBuf::from("/tmp/vh/assets/index.db")
        );
        assert_eq!(home.cache_dir(), PathBuf::from("/tmp/vh/cache"));
    }
}
