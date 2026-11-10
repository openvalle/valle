//! `valle project add`: files and folders copied into the project, or linked where they are,
//! each bound in the entry timeline's `resources`. A component goes to `components/`, anything
//! else to `media/`. Importing resources does not add clips.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{Error, Project, absolute, digest, pretty, write};

/// What `add` did with one file or folder.
struct Added {
    path: String,
    reused: bool,
}

impl Project {
    /// Adds each of `from`, copying it in unless `link`: `status` is `partial` when some of them
    /// could not be added, the others are then kept, and adding them again reuses them.
    pub fn add(&self, from: &[PathBuf], link: bool) -> Result<Value, Error> {
        let _lock = self.lock()?;
        let entry = self.root.join(&self.timeline);
        let bytes = fs::read(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
        let mut doc: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        valle_timeline::Document::read_edit(&doc)?;
        let before = doc.clone();
        let resources = doc
            .as_object_mut()
            .ok_or("a timeline object")?
            .entry("resources")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("resources is an object")?;
        let folder = entry.parent().unwrap_or(&self.root);
        let (mut added, mut failed) = (Vec::new(), Vec::new());
        for source in from {
            let shown = source.display().to_string();
            match self.add_one(source, link) {
                Ok(a) => {
                    let mut files = Vec::new();
                    let component = self.component(&a.path);
                    if let Some(entry) = &component {
                        files.push(entry.clone());
                    } else if self.root.join(&a.path).is_dir() {
                        crate::history::walk(&self.root, &a.path, &mut files);
                        files.sort();
                    } else {
                        files.push(a.path.clone());
                    }
                    if files.is_empty() {
                        failed.push(json!({ "from": shown, "message": format!("{shown}: no files to add") }));
                    }
                    for file in files {
                        let resource = if Path::new(&file).is_absolute() {
                            file.clone()
                        } else {
                            self.for_entry(&file)
                        };
                        let full = absolute(&folder.join(&resource));
                        let existing = resources.iter().find_map(|(name, v)| {
                            let file = v.as_str().or_else(|| {
                                v.as_array().filter(|a| a.len() == 1)?.first()?.as_str()
                            })?;
                            (absolute(&folder.join(file)) == full).then(|| name.clone())
                        });
                        let reused = a.reused || existing.is_some();
                        let alias = existing.unwrap_or_else(|| {
                            alias(&file, &resources.keys().cloned().collect::<Vec<_>>())
                        });
                        resources
                            .entry(alias.clone())
                            .or_insert_with(|| json!(resource));
                        added.push(json!({
                            "from": shown,
                            "path": if component.is_some() { &a.path } else { &file },
                            "reused": reused,
                            "alias": alias,
                            "resource": Value::Object([(alias.clone(), resources[&alias].clone())].into_iter().collect()),
                        }));
                    }
                }
                Err(message) => failed.push(json!({ "from": shown, "message": message })),
            }
        }
        if !added.is_empty() && before != doc {
            write(&entry, &pretty(&doc), true)?;
        }
        if added.is_empty() && !failed.is_empty() {
            let first = failed[0]["message"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            return Err(first);
        }
        let status = if failed.is_empty() { "ok" } else { "partial" };
        Ok(json!({ "status": status, "added": added, "failed": failed }))
    }

    fn add_one(&self, source: &Path, link: bool) -> Result<Added, Error> {
        let meta = fs::metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
        let name = (source.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no file name", source.display()))?;
        let component = match meta.is_dir() {
            true => top_level_component(source).is_some(),
            false => name.ends_with(".motion.tsx"),
        };
        if component && !meta.is_dir() {
            lone(source)?;
        }
        if link {
            return Ok(Added {
                path: absolute(source).to_string_lossy().replace('\\', "/"),
                reused: false,
            });
        }
        if let Some(path) = self.relative(source) {
            return Ok(Added { path, reused: true });
        }
        let folder = if component { "components" } else { "media" };
        let dir = self.root.join(folder);
        fs::create_dir_all(&dir).map_err(|e| format!("{folder}: {e}"))?;
        // The file of the same content there, else the name, with -2, -3… if it is taken.
        let (stem, ext) = split(&name, meta.is_dir());
        for k in 1.. {
            let candidate = match k {
                1 => name.clone(),
                k => format!("{stem}-{k}{ext}"),
            };
            let taken = (fs::read_dir(&dir).into_iter().flatten().flatten())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case(&candidate));
            let path = format!("{folder}/{}", taken.as_deref().unwrap_or(&candidate));
            let target = self.root.join(&path);
            match taken {
                Some(_) if same(source, &target)? => return Ok(Added { path, reused: true }),
                Some(_) => continue,
                None => {
                    copy(source, &target)?;
                    return Ok(Added {
                        path,
                        reused: false,
                    });
                }
            }
        }
        unreachable!("a free name")
    }

    /// The component a path added to `components/` is: the file, or the `.motion.tsx` at the top
    /// of the folder.
    fn component(&self, path: &str) -> Option<String> {
        let full = self.root.join(path);
        match full.is_dir() {
            true => top_level_component(&full).map(|f| format!("{path}/{f}")),
            false => path.ends_with(".motion.tsx").then(|| path.to_string()),
        }
    }

    /// A path in the project as the entry timeline's resources write it: from its folder.
    fn for_entry(&self, path: &str) -> String {
        let depth = self.timeline.matches('/').count();
        format!("{}{path}", "../".repeat(depth))
    }
}

/// The `.motion.tsx` at the top of a folder, the first by name.
fn top_level_component(dir: &Path) -> Option<String> {
    let mut found: Vec<String> = (fs::read_dir(dir).ok()?.flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".motion.tsx"))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// A lone component file may not import its own modules or carry its own files: those make it
/// a folder.
fn lone(file: &Path) -> Result<(), Error> {
    let entry = file.to_string_lossy().replace('\\', "/");
    let read = |p: &str| fs::read_to_string(p).ok();
    let more = match valle_engine::front::files(&entry, &read) {
        Ok(files) => files.len() > 1,
        Err(e) => e.contains("cannot import"),
    };
    match more {
        true => Err(format!(
            "{entry} imports modules or files of its own: add its folder, which is the component"
        )),
        false => Ok(()),
    }
}

/// A name's stem and extension, each entry's compound suffix as one; a folder's name is all stem.
fn split(name: &str, folder: bool) -> (String, String) {
    if folder {
        return (name.to_string(), String::new());
    }
    if let Ok(kind) = valle_engine::entry::Kind::read(name) {
        let stem = name.strip_suffix(kind.suffix()).unwrap();
        return (stem.to_string(), kind.suffix().into());
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    }
}

/// An alias for a file: its stem in the characters aliases have, not one of `taken`.
fn alias(path: &str, taken: &[String]) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, _) = split(name, false);
    let mut base: String = (stem.chars())
        .map(|c| match c.is_ascii_alphanumeric() || "._-".contains(c) {
            true => c,
            false => '-',
        })
        .take(56)
        .collect();
    if base.trim_matches('-').is_empty() {
        base = "resource".into();
    }
    (1..)
        .map(|k| match k {
            1 => base.clone(),
            k => format!("{base}-{k}"),
        })
        .find(|a| !taken.contains(a))
        .expect("a free alias")
}

/// Whether two files, or two folders, hold the same.
fn same(a: &Path, b: &Path) -> Result<bool, Error> {
    let (ma, mb) = match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => (ma, mb),
        _ => return Ok(false),
    };
    if ma.is_dir() != mb.is_dir() {
        return Ok(false);
    }
    if !ma.is_dir() {
        return Ok(ma.len() == mb.len() && digest(a)? == digest(b)?);
    }
    let names = |d: &Path| -> Vec<String> {
        let mut n: Vec<String> = (fs::read_dir(d).into_iter().flatten().flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        n.sort();
        n
    };
    let (na, nb) = (names(a), names(b));
    if na != nb {
        return Ok(false);
    }
    for n in na {
        if !same(&a.join(&n), &b.join(&n))? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Copies a file, or a folder but its hidden files and symbolic links, to `to`, which nothing
/// is at: a file is cloned where the disk can (APFS, Btrfs), and appears whole or not at all.
fn copy(from: &Path, to: &Path) -> Result<(), Error> {
    let meta = fs::metadata(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if meta.is_dir() {
        fs::create_dir(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if name.to_string_lossy().starts_with('.') || kind.is_symlink() {
                continue;
            }
            copy(&entry.path(), &to.join(&name))?;
        }
        return Ok(());
    }
    let temp = to.with_file_name(format!(
        ".{}.{}.tmp",
        to.file_name()
            .map_or("file".into(), |n| n.to_string_lossy()),
        std::process::id()
    ));
    let copied = fs::copy(from, &temp).and_then(|_| fs::hard_link(&temp, to));
    let _ = fs::remove_file(&temp);
    copied
        .map(|_| ())
        .map_err(|e| format!("{} → {}: {e}", from.display(), to.display()))
}
