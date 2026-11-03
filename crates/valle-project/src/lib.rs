//! A project: a plain folder of an edit's sources, `project.json` naming its entry timeline,
//! more timelines in `timelines/`, components in `components/` and media in `media/`, with the
//! snapshots of its sources and the records of its renders under `.valle/`. Its files are edited
//! directly: nothing here is a database, and a lock is held only while something is written.

pub mod add;
pub mod history;
pub mod show;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use ring::digest::{Context, SHA256};
use serde_json::{Value, json};
use valle_timeline::time::Rate;

pub type Error = String;

/// The file that makes a folder a project.
pub const FILE: &str = "project.json";

/// The folders a new project has.
const FOLDERS: [&str; 5] = [
    "timelines",
    "components",
    "media",
    "renders",
    ".valle/history",
];

#[derive(Debug)]
pub struct Project {
    /// The folder, absolute.
    pub root: PathBuf,
    pub name: String,
    /// The entry timeline, a path in the project.
    pub timeline: String,
}

impl Project {
    /// The project `start` is in: the nearest folder up from it with a `project.json`.
    pub fn discover(start: &Path) -> Result<Project, Error> {
        let start = absolute(start);
        let found = (start.ancestors()).find(|dir| dir.join(FILE).is_file());
        match found {
            Some(root) => Project::open(root),
            None => Err(format!(
                "no {FILE} in {} or a folder above it: create a project with valle project \
                 create <folder>",
                start.display()
            )),
        }
    }

    pub fn open(root: &Path) -> Result<Project, Error> {
        let root = absolute(root);
        let path = root.join(FILE);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bad = |what: &str| format!("{}: {what}", path.display());
        let config: Value = serde_json::from_str(&text).map_err(|e| bad(&e.to_string()))?;
        let Some(fields) = config.as_object() else {
            return Err(bad("an object of name and timeline"));
        };
        if let Some(k) = fields.keys().find(|k| *k != "name" && *k != "timeline") {
            return Err(bad(&format!("no field {k}: it has name and timeline")));
        }
        let text = |k: &str| {
            (fields.get(k).and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| bad(&format!("{k} is a string")))
        };
        let (name, timeline) = (text("name")?, text("timeline")?);
        if inside(&timeline).is_none() {
            return Err(bad("timeline is a path in the project"));
        }
        Ok(Project {
            root,
            name,
            timeline,
        })
    }

    /// A new project in `root`, which may exist but not be a project: its folders, an entry
    /// timeline of the canvas with no clips yet, and `project.json` last, so a create that
    /// stops partway can run again.
    pub fn create(root: &Path, width: u32, height: u32, fps: &str) -> Result<Project, Error> {
        let root = absolute(root);
        if root.join(FILE).exists() {
            return Err(format!("{} is a project already", root.display()));
        }
        if width == 0 || height == 0 {
            return Err(format!("a canvas of {width}×{height} px: both above 0"));
        }
        Rate::parse(fps)?;
        let timeline = root.join("timeline.json");
        if timeline.exists() {
            return Err(format!(
                "{} is there already: create the project in another folder",
                timeline.display()
            ));
        }
        for dir in FOLDERS {
            fs::create_dir_all(root.join(dir)).map_err(|e| format!("{dir}: {e}"))?;
        }
        let fps = fps.parse::<f64>().map_or(json!(fps), |n| json!(n));
        let canvas =
            json!({ "canvas": { "width": width, "height": height, "fps": fps }, "tracks": {} });
        write(&timeline, &pretty(&canvas), false)?;
        let name =
            (root.file_name()).map_or("project".into(), |n| n.to_string_lossy().into_owned());
        let config = json!({ "name": name, "timeline": "timeline.json" });
        write(&root.join(FILE), &pretty(&config), false)?;
        Project::open(&root)
    }

    /// Every timeline, as a path in the project: the entry, then those in `timelines/` by name.
    pub fn timelines(&self) -> Vec<String> {
        let mut more: Vec<String> = (fs::read_dir(self.root.join("timelines")).into_iter())
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                (name.ends_with(".json") && e.path().is_file()).then(|| format!("timelines/{name}"))
            })
            .filter(|p| *p != self.timeline)
            .collect();
        more.sort();
        std::iter::once(self.timeline.clone()).chain(more).collect()
    }

    /// The timeline a command names, or the entry: a path from the current folder, or one in
    /// the project, or a name in `timelines/` with or without `.json`.
    pub fn timeline(&self, name: Option<&str>) -> Result<PathBuf, Error> {
        let Some(name) = name else {
            return Ok(self.root.join(&self.timeline));
        };
        let mut tried = vec![absolute(Path::new(name)), self.root.join(name)];
        tried.push(self.root.join("timelines").join(name));
        tried.push(self.root.join("timelines").join(format!("{name}.json")));
        (tried.into_iter().find(|p| p.is_file())).ok_or_else(|| {
            format!(
                "no timeline {name}: the project has {}",
                self.timelines().join(", ")
            )
        })
    }

    /// `path` as a path in the project, `/`-separated, if it is in it.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let rest = absolute(path).strip_prefix(&self.root).ok()?.to_path_buf();
        Some(rest.to_string_lossy().replace('\\', "/"))
    }

    /// The project held for writing, until the lock is dropped.
    pub fn lock(&self) -> Result<Lock, Error> {
        let dir = self.root.join(".valle");
        fs::create_dir_all(&dir).map_err(|e| format!(".valle: {e}"))?;
        let file = (File::options().create(true).truncate(false).write(true))
            .open(dir.join("lock"))
            .map_err(|e| format!(".valle/lock: {e}"))?;
        file.lock().map_err(|e| format!(".valle/lock: {e}"))?;
        Ok(Lock(file))
    }
}

/// A project held for writing.
pub struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// `path` absolute, its `.` and `..` resolved without reading the disk.
pub fn absolute(path: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// A `/`-separated relative path that stays in the folder it is relative to, normalized.
pub fn inside(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return None;
    }
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// JSON as a file has it: indented, with a last newline.
pub fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).expect("JSON prints");
    text.push('\n');
    text.into_bytes()
}

/// Writes `bytes` to `path` whole or not at all: into a new file beside it, then moved over it,
/// or, unless `replace`, put there only if nothing is.
pub fn write(path: &Path, bytes: &[u8], replace: bool) -> Result<(), Error> {
    let shown = || path.display().to_string();
    let dir = path
        .parent()
        .ok_or_else(|| format!("{}: no folder", shown()))?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let temp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map_or("file".into(), |n| n.to_string_lossy()),
        std::process::id()
    ));
    let written = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match replace {
            true => fs::rename(&temp, path),
            false => fs::hard_link(&temp, path),
        }
    })();
    let _ = fs::remove_file(&temp);
    written.map_err(|e| format!("{}: {e}", shown()))
}

/// The SHA-256 of a file, in hex.
pub fn digest(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0; 1 << 20];
    loop {
        let n = (file.read(&mut buffer)).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        context.update(&buffer[..n]);
    }
    Ok(hex(context.finish().as_ref()))
}

/// The SHA-256 of bytes, in hex.
pub fn digest_bytes(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&SHA256, bytes).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
