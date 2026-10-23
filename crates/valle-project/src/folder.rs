//! File-backed projects. Source files remain authoritative; history contains source snapshots.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ContentDigest;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
    pub timeline: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: ProjectConfig,
}

/// An input captured by the compiler/render host. Binary media carries only its digest.
#[derive(Debug, Clone)]
pub struct CapturedInput {
    pub path: PathBuf,
    pub digest: ContentDigest,
    pub bytes: Option<Vec<u8>>,
}

impl CapturedInput {
    pub fn source(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            digest: ContentDigest::of_bytes(&bytes),
            bytes: Some(bytes),
        }
    }

    pub fn media(path: PathBuf, digest: ContentDigest) -> Self {
        Self {
            path,
            digest,
            bytes: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotFile {
    pub path: String,
    pub digest: Option<ContentDigest>,
    /// Relative to this snapshot's directory; absent for media and nonexistent preimages.
    pub stored: Option<String>,
    pub existed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub number: u64,
    pub created_at: u64,
    pub message: Option<String>,
    pub source: String,
    pub files: Vec<SnapshotFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSnapshot {
    pub snapshot: u64,
    pub unchanged: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub snapshot: u64,
    pub before_snapshot: u64,
    pub restored: Vec<String>,
    pub skipped: Vec<serde_json::Value>,
}

/// Kept only for the duration of an operation. Never remove the lock file.
pub struct ProjectLock(File);

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

impl Project {
    pub fn discover(start: &Path) -> Result<Self> {
        let start = absolute(start)?;
        let directory = if start.is_file() {
            start.parent().unwrap()
        } else {
            &start
        };
        for candidate in directory.ancestors() {
            if candidate.join("project.json").is_file() {
                return Self::open(candidate);
            }
        }
        bail!("no project.json found; create a project with: valle project create <directory>")
    }

    pub fn open(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("opening {}", root.display()))?;
        let config: ProjectConfig = serde_json::from_slice(&fs::read(root.join("project.json"))?)?;
        if config.name.trim().is_empty() || config.timeline.as_os_str().is_empty() {
            bail!("project.json needs a name and an entry Timeline");
        }
        if config.timeline.is_absolute()
            || config
                .timeline
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            bail!("project entry Timeline must be a path inside the project");
        }
        Ok(Self { root, config })
    }

    pub fn create(root: &Path, width: u32, height: u32, fps: &str) -> Result<Self> {
        let root = absolute(root)?;
        if root.join("project.json").exists() {
            bail!(
                "already a project; inspect it with: valle project show -C {}",
                root.display()
            );
        }
        let fps = serde_json::from_str::<serde_json::Value>(fps).unwrap_or_else(|_| json!(fps));
        let timeline = json!({"canvas":{"width":width,"height":height,"fps":fps},"tracks":{}});
        // Validate before creating anything, including dimensions and rational frame rates.
        valle_timeline::decode_timeline(&timeline.to_string())?;
        fs::create_dir_all(&root)?;
        if root.join("timeline.json").exists() {
            bail!("timeline.json already exists; choose another project directory");
        }
        for dir in ["components", "media", "renders", ".valle/history"] {
            fs::create_dir_all(root.join(dir))?;
        }
        let name = root
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("project")
            .to_owned();
        let config = ProjectConfig {
            name,
            timeline: "timeline.json".into(),
        };
        write_json(&root.join("timeline.json"), &timeline, false)?;
        // Publishing the marker last makes a failed create safe to inspect and retry.
        write_json(&root.join("project.json"), &config, false)?;
        Self::open(&root)
    }

    pub fn timeline_path(&self) -> PathBuf {
        self.root.join(&self.config.timeline)
    }

    pub fn lock(&self) -> Result<ProjectLock> {
        let directory = self.root.join(".valle");
        fs::create_dir_all(&directory)?;
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            bail!("project management directory must not be a symbolic link");
        }
        let path = directory.join("lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            bail!("project lock must be a regular file");
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(ProjectLock(file))
    }

    pub fn history(&self) -> Result<Vec<Snapshot>> {
        let directory = self.root.join(".valle/history");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let Some(number) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if number > 0 {
                result.push(self.snapshot(number)?);
            }
        }
        result.sort_by_key(|s| s.number);
        Ok(result)
    }

    pub fn snapshot(&self, number: u64) -> Result<Snapshot> {
        let snapshot: Snapshot = serde_json::from_slice(
            &fs::read(self.snapshot_dir(number).join("snapshot.json"))
                .with_context(|| format!("reading snapshot {number}"))?,
        )?;
        if snapshot.number != number {
            bail!("snapshot number does not match its directory");
        }
        Ok(snapshot)
    }

    pub fn snapshot_dir(&self, number: u64) -> PathBuf {
        self.root.join(".valle/history").join(number.to_string())
    }

    pub fn path_key(&self, path: &Path) -> Result<String> {
        let path = absolute(path)?;
        Ok(path
            .strip_prefix(&self.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/"))
    }

    /// The caller holds the project lock while capturing and saving these exact inputs.
    pub fn save_captured(
        &self,
        mut inputs: Vec<CapturedInput>,
        message: Option<&str>,
        source: &str,
    ) -> Result<SavedSnapshot> {
        inputs.push(CapturedInput::source(
            self.root.join("project.json"),
            fs::read(self.root.join("project.json"))?,
        ));
        self.save_inputs(inputs, BTreeMap::new(), message, source)
    }

    fn save_inputs(
        &self,
        inputs: Vec<CapturedInput>,
        missing: BTreeMap<String, ()>,
        message: Option<&str>,
        source: &str,
    ) -> Result<SavedSnapshot> {
        let mut entries = BTreeMap::new();
        for input in inputs {
            let key = self.path_key(&input.path)?;
            if let Some(previous) = entries.insert(key.clone(), input.clone()) {
                if previous.digest != input.digest {
                    bail!("input changed while capturing: {key}");
                }
            }
        }
        let files = entries
            .iter()
            .enumerate()
            .map(|(i, (path, input))| SnapshotFile {
                path: path.clone(),
                digest: Some(input.digest),
                stored: input.bytes.as_ref().map(|_| format!("sources/{i}")),
                existed: true,
            })
            .chain(missing.keys().map(|path| SnapshotFile {
                path: path.clone(),
                digest: None,
                stored: None,
                existed: false,
            }))
            .collect::<Vec<_>>();
        let history = self.history()?;
        if let Some(last) = history.last() {
            if last.files == files {
                return Ok(SavedSnapshot {
                    snapshot: last.number,
                    unchanged: true,
                });
            }
        }
        let number = history.last().map_or(1, |s| s.number + 1);
        let directory = self.root.join(".valle/history");
        fs::create_dir_all(&directory)?;
        let temp = tempfile::Builder::new()
            .prefix(".snapshot-")
            .tempdir_in(&directory)?;
        fs::create_dir(temp.path().join("sources"))?;
        for (i, input) in entries.values().enumerate() {
            if let Some(bytes) = &input.bytes {
                write_bytes(&temp.path().join(format!("sources/{i}")), bytes, false)?;
            }
        }
        let snapshot = Snapshot {
            number,
            created_at: now_millis(),
            message: message.map(str::to_owned),
            source: source.to_owned(),
            files,
        };
        write_json(&temp.path().join("snapshot.json"), &snapshot, false)?;
        crate::sync_directory(temp.path())?;
        fs::rename(temp.path(), self.snapshot_dir(number))?;
        crate::sync_directory(&directory)?;
        Ok(SavedSnapshot {
            snapshot: number,
            unchanged: false,
        })
    }

    /// Capture every preimage that will be overwritten, even when the current Timeline is broken.
    /// The caller holds the project lock for this entire operation.
    pub fn restore(&self, number: u64, mut current: Vec<CapturedInput>) -> Result<RestoreResult> {
        let snapshot = self.snapshot(number)?;
        let mut missing = BTreeMap::new();
        let mut writes = Vec::new();
        let mut skipped = Vec::new();
        for file in &snapshot.files {
            let relative = Path::new(&file.path);
            let path = self.root.join(relative);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| matches!(c, Component::ParentDir))
            {
                skipped.push(json!({"path":file.path,"reason":"outside_project"}));
                continue;
            }
            if !safe_destination(&self.root, relative)? {
                skipped.push(json!({"path":file.path,"reason":"symbolic_link_or_nonregular"}));
                continue;
            }
            let Some(stored) = &file.stored else {
                let matches = file
                    .digest
                    .is_some_and(|digest| digest_file(&path).ok() == Some(digest));
                if !matches {
                    skipped.push(json!({"path":file.path,"reason":if file.existed {"media_changed_or_missing"} else {"previously_absent"}}));
                }
                continue;
            };
            let stored_path = Path::new(stored);
            if stored_path.is_absolute()
                || stored_path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir))
            {
                bail!("invalid snapshot source path");
            }
            let bytes = fs::read(self.snapshot_dir(number).join(stored_path))?;
            if Some(ContentDigest::of_bytes(&bytes)) != file.digest {
                bail!("snapshot source digest mismatch: {}", file.path);
            }
            match fs::read(&path) {
                Ok(before) => current.push(CapturedInput::source(path.clone(), before)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing.insert(file.path.clone(), ());
                }
                Err(error) => return Err(error.into()),
            }
            writes.push((file.path.clone(), path, bytes));
        }
        // A target's preimage takes precedence over any earlier capture of the same file.
        let mut preimages = BTreeMap::new();
        for input in current {
            preimages.insert(input.path.clone(), input);
        }
        let before = self.save_inputs(
            preimages.into_values().collect(),
            missing,
            Some(&format!("before restore {number}")),
            "restore",
        )?;
        let mut restored = Vec::new();
        for (key, path, bytes) in writes {
            if !safe_destination(&self.root, Path::new(&key))? {
                bail!("restore destination changed: {key}");
            }
            write_bytes(&path, &bytes, true)?;
            restored.push(key);
        }
        Ok(RestoreResult {
            snapshot: number,
            before_snapshot: before.snapshot,
            restored,
            skipped,
        })
    }
}

pub fn app_home() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("VALLE_HOME") {
        return Ok(root.into());
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("set VALLE_HOME because no user home directory is available")?;
    Ok(PathBuf::from(home).join(".valle"))
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}

fn safe_destination(root: &Path, relative: &Path) -> Result<bool> {
    let mut path = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, part) in components.iter().enumerate() {
        if !matches!(part, Component::Normal(_)) {
            return Ok(false);
        }
        path.push(part.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(false),
            Ok(meta) if index + 1 < components.len() && !meta.is_dir() => return Ok(false),
            Ok(meta) if index + 1 == components.len() && !meta.is_file() => return Ok(false),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

pub fn digest_file(path: &Path) -> Result<ContentDigest> {
    use sha2::{Digest, Sha256};
    let mut input = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(ContentDigest::from_bytes(digest.finalize().into()))
}

pub fn write_json(path: &Path, value: &impl Serialize, replace: bool) -> Result<()> {
    write_bytes(
        path,
        format!("{}\n", serde_json::to_string_pretty(value)?).as_bytes(),
        replace,
    )
}

pub fn write_bytes(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().context("file has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(meta) = fs::metadata(path) {
        temp.as_file().set_permissions(meta.permissions())?;
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    }
    .map_err(|error| error.error)?;
    crate::sync_directory(parent)?;
    Ok(())
}
