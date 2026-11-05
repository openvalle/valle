//! Module loading, import resolution and scene declarations.
use std::collections::HashMap;
use std::rc::Rc;

use super::{Composition, Prop, Role, Slot, rewrite};

use crate::template::Error;

/// Extensions an import resolves through, in order.
const EXTENSIONS: [&str; 4] = [".tsx", ".ts", ".jsx", ".js"];

/// What a scene declares, read without running it: its composition, controls and role, its
/// resources' defaults joined to its folder.
pub struct Declared {
    pub composition: Composition,
    pub props: Vec<Prop>,
    pub data: Vec<Prop>,
    pub resources: Vec<Slot>,
    pub role: Role,
}

/// What the scene at `entry` declares; `read` gives its text.
pub fn declared(entry: &str, read: &dyn Fn(&str) -> Option<String>) -> Result<Declared, Error> {
    let text = read(entry).ok_or_else(|| format!("{entry}: not found"))?;
    let code = rewrite::rewrite(entry, &text, 0).map_err(|e| format!("{entry}: {e}"))?;
    let folder = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    let composition = code.composition.ok_or_else(|| {
        format!("{entry}: a scene exports its composition, which a Timeline clip plays")
    })?;
    Ok(Declared {
        composition,
        props: code.props,
        data: code.data,
        resources: (code.resources.into_iter())
            .map(|mut slot| {
                for d in &mut slot.default {
                    if !(d.starts_with('/') || rewrite::web(d)) {
                        *d = rewrite::join(folder, d);
                    }
                }
                slot
            })
            .collect(),
        role: code.role,
    })
}

/// The files the scene at `entry` reads, without running it: the modules it imports, itself
/// first, then its resources' defaults joined to its folder, addresses on the web left out.
pub fn files(entry: &str, read: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>, Error> {
    let text = read(entry).ok_or_else(|| format!("{entry}: not found"))?;
    let mut loader = Loader {
        read,
        loaded: Vec::new(),
        by_path: HashMap::new(),
        visiting: Vec::new(),
        sites: Vec::new(),
    };
    loader.visit(entry.to_string(), text)?;
    let mut files = vec![entry.to_string()];
    let modules = loader.loaded.iter().rev().filter(|l| l.path != entry);
    files.extend(modules.map(|l| l.path.clone()));
    let folder = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    let scene = &loader.loaded[loader.by_path[entry]].code;
    for d in scene.resources.iter().flat_map(|slot| &slot.default) {
        if !rewrite::web(d) {
            files.push(match d.starts_with('/') {
                true => d.clone(),
                false => rewrite::join(folder, d),
            });
        }
    }
    Ok(files)
}

pub(super) struct Loaded {
    pub(super) path: String,
    /// The import keys that resolve to the module, its path first.
    pub(super) keys: Vec<String>,
    pub(super) code: rewrite::Rewritten,
}

pub(super) struct Loader<'r> {
    pub(super) read: &'r dyn Fn(&str) -> Option<String>,
    pub(super) loaded: Vec<Loaded>,
    pub(super) by_path: HashMap<String, usize>,
    pub(super) visiting: Vec<String>,
    pub(super) sites: Vec<Rc<str>>,
}

impl Loader<'_> {
    /// Rewrites the module at `path` and, depth first, the modules it imports.
    pub(super) fn visit(&mut self, path: String, text: String) -> Result<usize, Error> {
        self.visiting.push(path.clone());
        let code = rewrite::rewrite(&path, &text, self.sites.len()).map_err(|e| {
            let sep = if e.starts_with(|c: char| c.is_ascii_digit()) {
                ":"
            } else {
                ": "
            };
            format!("{path}{sep}{e}")
        })?;
        for s in &code.sites {
            self.sites.push(match s.line {
                0 => path.as_str().into(),
                line => format!("{path}:{line}:{}", s.col).into(),
            });
        }
        let mut imports = Vec::new();
        for key in &code.imports {
            let (target, text) = self.resolve(key).ok_or_else(|| {
                format!(
                    "{path}: cannot import {key:?}: no such file (tried {key}.tsx, .ts, .jsx, .js)"
                )
            })?;
            if let Some(i) = self.visiting.iter().position(|p| *p == target) {
                let mut cycle = self.visiting[i..].to_vec();
                cycle.push(target);
                return Err(format!(
                    "import cycle: {}; modules cannot import each other in a circle",
                    cycle.join(" → ")
                ));
            }
            let index = match self.by_path.get(&target) {
                Some(&i) => i,
                None => self.visit(target, text)?,
            };
            imports.push((key.clone(), index));
        }
        self.visiting.pop();
        self.loaded.push(Loaded {
            path: path.clone(),
            keys: vec![path.clone()],
            code,
        });
        let index = self.loaded.len() - 1;
        self.by_path.insert(path, index);
        for (key, i) in imports {
            let keys = &mut self.loaded[i].keys;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(index)
    }

    /// Rewrites a built-in module, which the runtime makes globals of.
    pub(super) fn builtin(&mut self, path: &str, text: &str) -> Result<rewrite::Rewritten, Error> {
        let code = rewrite::rewrite(path, text, self.sites.len())
            .map_err(|e| format!("engine bug: {path}: {e}"))?;
        for s in &code.sites {
            self.sites
                .push(format!("{path}:{}:{}", s.line, s.col).into());
        }
        Ok(code)
    }

    /// The file an import key names: the key itself when it has an extension, else the first of
    /// the key with each extension.
    pub(super) fn resolve(&self, key: &str) -> Option<(String, String)> {
        let exact = EXTENSIONS
            .iter()
            .any(|e| key.ends_with(e))
            .then(|| key.to_string());
        exact
            .into_iter()
            .chain(EXTENSIONS.iter().map(|e| format!("{key}{e}")))
            .find_map(|path| (self.read)(&path).map(|text| (path, text)))
    }
}
