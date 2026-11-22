//! Snapshots of a project's sources, numbered from 1 in `.valle/history/<n>/`: `snapshot.json`
//! lists every source file with its SHA-256, and `files/` keeps the text ones as they were:
//! `project.json`, the timelines, everything in `components/`, and the data and shaders in
//! `media/`, with each timeline a note is on and what it plays, and each component a timeline
//! plays or a note is on and the modules it imports, wherever they are in the project.
//! Videos, sounds, images, fonts and models are only listed: a snapshot says whether
//! they changed, and restoring never writes them. A full render saves one and keeps its record
//! beside it, in `renders.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::{Error, FILE, Project, digest, digest_bytes, inside, pretty, write};

/// Extensions of the files in `media/` a snapshot keeps: data, captions and shaders.
const KEPT: [&str; 9] = [
    "json", "csv", "tsv", "txt", "srt", "vtt", "lrc", "wgsl", "md",
];

/// A source file: its path in the project, its SHA-256, and whether a snapshot keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: String,
    pub sha256: String,
    pub kept: bool,
}

pub struct Snapshot {
    pub number: u64,
    /// When it was saved, milliseconds since 1970.
    pub time: u64,
    pub message: Option<String>,
    /// What saved it: `save`, `render`, `restore` or `note`.
    pub by: String,
    pub files: Vec<Source>,
}

impl Snapshot {
    pub fn summary(&self) -> Value {
        json!({
            "snapshot": self.number,
            "time": self.time,
            "message": self.message,
            "by": self.by,
            "files": self.files.len(),
        })
    }
}

impl Project {
    fn history_dir(&self) -> PathBuf {
        self.root.join(".valle/history")
    }

    /// Every source file now, by path.
    pub fn sources(&self) -> Result<Vec<Source>, Error> {
        self.sources_with(&[])
    }

    /// The sources, with the timeline or component at each of `reviewed`, a path in the project,
    /// and what it plays or imports, as those notes are on.
    fn sources_with(&self, reviewed: &[String]) -> Result<Vec<Source>, Error> {
        let mut paths = vec![FILE.to_string()];
        let mut on = self.noted();
        on.extend(reviewed.iter().cloned());
        let (mut components, noted): (Vec<String>, Vec<String>) =
            on.into_iter().partition(|p| p.ends_with(".motion.tsx"));
        let mut timelines = self.timelines();
        timelines.extend(noted.into_iter().filter(|p| self.root.join(p).is_file()));
        timelines.sort();
        timelines.dedup();
        for rel in timelines.clone() {
            if let Ok(doc) = self.document(&rel) {
                let named = self
                    .dependencies(&rel, &doc)
                    .into_iter()
                    .filter_map(|n| n.path);
                timelines.extend(named.filter(|p| p.ends_with(".timeline.json")));
            }
        }
        timelines.sort();
        timelines.dedup();
        paths.extend(timelines.iter().cloned());
        for dir in ["components", "media"] {
            walk(&self.root, dir, &mut paths);
        }
        for rel in &timelines {
            let Ok(doc) = self.document(rel) else {
                continue;
            };
            let named = self.dependencies(rel, &doc).into_iter();
            components.extend(named.filter_map(|n| n.path.filter(|p| p.ends_with(".motion.tsx"))));
        }
        components.sort();
        components.dedup();
        for component in &components {
            paths.extend(self.modules(component));
        }
        // What the timelines name in the project is listed, as a preview reads it.
        for rel in &timelines {
            if let Ok(doc) = self.document(rel) {
                paths.extend(
                    self.dependencies(rel, &doc)
                        .into_iter()
                        .filter_map(|n| n.path),
                );
            }
        }
        paths.sort();
        paths.dedup();
        let mut cache = Digests::open(self);
        let sources = (paths.into_iter())
            .filter(|p| self.root.join(p).is_file())
            .map(|path| {
                let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
                let ext = ext.as_deref().unwrap_or("");
                let text = KEPT.contains(&ext) || CODE.contains(&ext);
                // Out of media/, a file is kept but a picture, a sound or a video a timeline names.
                let kept = match path.split_once('/').map(|(dir, _)| dir) {
                    Some("media") => KEPT.contains(&ext),
                    Some("components") => true,
                    _ => text || path == FILE || path.ends_with(".timeline.json"),
                };
                let sha256 = cache.of(&self.root, &path)?;
                Ok(Source { path, sha256, kept })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        cache.save(self);
        Ok(sources)
    }

    /// The snapshots, oldest first.
    pub fn history(&self) -> Result<Vec<Snapshot>, Error> {
        let mut numbers: Vec<u64> = (fs::read_dir(self.history_dir()).into_iter())
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .filter(|n| *n > 0)
            .collect();
        numbers.sort();
        numbers.into_iter().map(|n| self.snapshot(n)).collect()
    }

    pub fn snapshot(&self, number: u64) -> Result<Snapshot, Error> {
        let path = self
            .history_dir()
            .join(number.to_string())
            .join("snapshot.json");
        let text = fs::read_to_string(&path).map_err(|_| {
            format!("no snapshot {number}: valle project history lists the snapshots")
        })?;
        let bad = || format!("{}: not a snapshot", path.display());
        let j: Value = serde_json::from_str(&text).map_err(|_| bad())?;
        let files = (j["files"].as_array().ok_or_else(bad)?.iter())
            .map(|f| {
                Some(Source {
                    path: inside(f["path"].as_str()?)?,
                    sha256: f["sha256"].as_str()?.to_string(),
                    kept: f["kept"].as_bool()?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(bad)?;
        Ok(Snapshot {
            number,
            time: j["time"].as_u64().unwrap_or(0),
            message: j["message"].as_str().map(str::to_string),
            by: j["by"].as_str().unwrap_or("save").to_string(),
            files,
        })
    }

    /// Saves the sources as a new snapshot, or gives the last one if none of them changed since:
    /// its number, and whether it is new.
    pub fn save(&self, message: Option<&str>, by: &str) -> Result<(u64, bool), Error> {
        self.save_with(message, by, &[])
    }

    /// As `save`, keeping the component at each of `reviewed` too.
    pub(crate) fn save_with(
        &self,
        message: Option<&str>,
        by: &str,
        reviewed: &[String],
    ) -> Result<(u64, bool), Error> {
        let _lock = self.lock()?;
        let files = self.sources_with(reviewed)?;
        let history = self.history()?;
        if let Some(last) = history.last()
            && last.files == files
        {
            return Ok((last.number, false));
        }
        let number = history.last().map_or(1, |s| s.number + 1);
        let dir = self.history_dir();
        let temp = dir.join(format!(".new-{number}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        let saved = (|| {
            for f in files.iter().filter(|f| f.kept) {
                let bytes =
                    (fs::read(self.root.join(&f.path))).map_err(|e| format!("{}: {e}", f.path))?;
                // A file written while it was read is saved again on the next try.
                if digest_bytes(&bytes) != f.sha256 {
                    return Err(format!("{} changed while it was saved: save again", f.path));
                }
                write(&temp.join("files").join(&f.path), &bytes, true)?;
            }
            let listed: Vec<Value> = (files.iter())
                .map(|f| json!({ "path": f.path, "sha256": f.sha256, "kept": f.kept }))
                .collect();
            let snapshot = json!({
                "number": number,
                "time": now(),
                "message": message,
                "by": by,
                "files": listed,
            });
            write(&temp.join("snapshot.json"), &pretty(&snapshot), true)?;
            fs::rename(&temp, dir.join(number.to_string())).map_err(|e| e.to_string())
        })();
        if saved.is_err() {
            let _ = fs::remove_dir_all(&temp);
        }
        saved.map(|_| (number, true))
    }

    /// Puts the kept files of snapshot `number` back, after saving what they replace as a
    /// snapshot of its own. It writes no media and nothing out of the project, deletes no file
    /// added since, and follows no symbolic link: what it leaves is reported.
    pub fn restore(&self, number: u64) -> Result<Value, Error> {
        let snapshot = self.snapshot(number)?;
        let (backup, _) = self.save(Some(&format!("before restore {number}")), "restore")?;
        let _lock = self.lock()?;
        let stored = self.history_dir().join(number.to_string()).join("files");
        let (mut restored, mut same, mut skipped) = (Vec::new(), 0, Vec::new());
        for f in &snapshot.files {
            let path = self.root.join(&f.path);
            let now = digest(&path).ok();
            if now.as_deref() == Some(f.sha256.as_str()) {
                same += 1;
                continue;
            }
            if !f.kept {
                let why = if now.is_some() {
                    "media changed"
                } else {
                    "media missing"
                };
                skipped.push(json!({ "path": f.path, "reason": why }));
                continue;
            }
            if linked(&self.root, &f.path) {
                skipped.push(json!({ "path": f.path, "reason": "a symbolic link or not a file" }));
                continue;
            }
            let bytes = fs::read(stored.join(&f.path)).map_err(|e| format!("{}: {e}", f.path))?;
            if digest_bytes(&bytes) != f.sha256 {
                return Err(format!("snapshot {number}: {} is damaged", f.path));
            }
            write(&path, &bytes, true)?;
            restored.push(f.path.clone());
        }
        let listed: Vec<&str> = snapshot.files.iter().map(|f| f.path.as_str()).collect();
        let added: Vec<String> = (self.sources()?.into_iter())
            .filter(|s| !listed.contains(&s.path.as_str()))
            .map(|s| s.path)
            .collect();
        Ok(json!({
            "snapshot": number,
            "backup": backup,
            "restored": restored,
            "unchanged": same,
            "skipped": skipped,
            "added": added,
        }))
    }

    /// Adds a render's record to snapshot `number`'s.
    pub fn record(&self, number: u64, record: Value) -> Result<(), Error> {
        let _lock = self.lock()?;
        let path = self
            .history_dir()
            .join(number.to_string())
            .join("renders.json");
        let mut records = self.records(number);
        records.push(record);
        write(&path, &pretty(&Value::Array(records)), true)
    }

    /// The records of the renders of snapshot `number`, oldest first.
    pub fn records(&self, number: u64) -> Vec<Value> {
        let path = self
            .history_dir()
            .join(number.to_string())
            .join("renders.json");
        (fs::read_to_string(path).ok())
            .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
            .unwrap_or_default()
    }

    /// The newest render's record, if any.
    pub fn last_render(&self) -> Option<Value> {
        let history = self.history().ok()?;
        (history.iter().rev()).find_map(|s| self.records(s.number).pop())
    }
}

impl Project {
    /// The component at `path` in the project and the files it reads there: the modules it
    /// imports, and its resources' defaults. One that does not compile is itself.
    fn modules(&self, path: &str) -> Vec<String> {
        let root = self.root.to_string_lossy().replace('\\', "/");
        let read = |p: &str| fs::read_to_string(p).ok();
        let Ok(files) = valle_engine::front::files(&format!("{root}/{path}"), &read) else {
            return vec![path.to_string()];
        };
        (files.into_iter())
            .filter_map(|f| self.relative(Path::new(&f)))
            .collect()
    }

    /// Whether `version`, as `crate::version` gives it, is the version of `path` whose SHA-256
    /// the cache of digests knows as `sha256`.
    pub(crate) fn is_version(&self, path: &str, version: &str, sha256: &str) -> bool {
        let cache = Digests::open(self);
        (cache.known.get(path))
            .is_some_and(|(len, time, sha)| sha == sha256 && format!("{len}:{time}") == version)
    }
}

/// Extensions of the modules a component imports.
const CODE: [&str; 8] = ["tsx", "ts", "jsx", "js", "mjs", "json", "css", "wgsl"];

/// Milliseconds since 1970.
pub fn now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map_or(0, |d| d.as_millis() as u64)
}

/// Adds the files under `dir` of `root` to `paths`, but hidden ones and symbolic links.
pub(crate) fn walk(root: &Path, dir: &str, paths: &mut Vec<String>) {
    for entry in fs::read_dir(root.join(dir)).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if name.starts_with('.') || kind.is_symlink() {
            continue;
        }
        let path = format!("{dir}/{name}");
        if kind.is_dir() {
            walk(root, &path, paths);
        } else if kind.is_file() {
            paths.push(path);
        }
    }
}

/// Whether a symbolic link, or something but a folder, is on the way to `path`, or `path` is
/// there and not a file.
pub(crate) fn linked(root: &Path, path: &str) -> bool {
    let mut at = root.to_path_buf();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(m) if i + 1 < parts.len() && !m.is_dir() => return true,
            Ok(m) if i + 1 == parts.len() && !m.is_file() => return true,
            _ => {}
        }
    }
    false
}

/// The SHA-256 of files by their size and time of change, in `.valle/digests.json`, so a large
/// video is read once, not at every save.
struct Digests {
    known: BTreeMap<String, (u64, u128, String)>,
    changed: bool,
}

impl Digests {
    fn open(project: &Project) -> Digests {
        let path = project.root.join(".valle/digests.json");
        let known = (fs::read_to_string(path).ok())
            .and_then(|t| serde_json::from_str::<BTreeMap<String, (u64, u128, String)>>(&t).ok())
            .unwrap_or_default();
        Digests {
            known,
            changed: false,
        }
    }

    fn of(&mut self, root: &Path, path: &str) -> Result<String, Error> {
        let file = root.join(path);
        let meta = fs::metadata(&file).map_err(|e| format!("{path}: {e}"))?;
        let time = (meta.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        if let Some((len, t, sha)) = self.known.get(path)
            && *len == meta.len()
            && *t == time
        {
            return Ok(sha.clone());
        }
        let sha = digest(&file)?;
        self.known
            .insert(path.to_string(), (meta.len(), time, sha.clone()));
        self.changed = true;
        Ok(sha)
    }

    fn save(&self, project: &Project) {
        if self.changed
            && let Ok(text) = serde_json::to_vec(&self.known)
        {
            let _ = write(&project.root.join(".valle/digests.json"), &text, true);
        }
    }
}
