//! `valle project show` and the project's part of `valle project check`: the timelines, the
//! files their resources name, which of those are out of the project or missing, and whether
//! components read local files from the project.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use valle_timeline::Document;

use crate::{Error, Project, absolute};

/// A file a timeline's resource names.
pub struct Named {
    pub timeline: String,
    pub alias: String,
    /// As the timeline writes it.
    pub written: String,
    /// Its path in the project; none for one out of it or on the web.
    pub path: Option<String>,
    /// Out of the project, or on the web.
    pub external: bool,
    pub missing: bool,
}

impl Named {
    fn json(&self) -> Value {
        json!({
            "timeline": self.timeline,
            "alias": self.alias,
            "file": self.written,
            "external": self.external,
            "missing": self.missing,
        })
    }
}

impl Project {
    /// The timeline at `rel` in the project, including an empty edit with imported resources.
    pub fn document(&self, rel: &str) -> Result<Document, Error> {
        valle_engine::entry::Kind::Timeline.check(rel)?;
        let text = (fs::read_to_string(self.root.join(rel))).map_err(|e| format!("{rel}: {e}"))?;
        let value = serde_json::from_str(&text).map_err(|e| format!("{rel}: {e}"))?;
        Document::read_edit(&value).map_err(|e| format!("{rel}: {e}"))
    }

    /// The files the resources of the timeline at `rel` name.
    pub fn named(&self, rel: &str, doc: &Document) -> Vec<Named> {
        let folder = self.root.join(rel);
        let folder = folder.parent().unwrap_or(&self.root);
        let mut out = Vec::new();
        for (alias, resource) in &doc.resources {
            for written in &resource.files {
                let web = written.starts_with("http://") || written.starts_with("https://");
                let full = absolute(&folder.join(written));
                let path = (!web).then(|| self.relative(&full)).flatten();
                out.push(Named {
                    timeline: rel.to_string(),
                    alias: alias.clone(),
                    written: written.clone(),
                    external: web || path.is_none(),
                    missing: !web && !full.exists(),
                    path,
                });
            }
        }
        out
    }

    /// Resources of this timeline and those it nests, with paths relative to their own file.
    pub fn dependencies(&self, rel: &str, doc: &Document) -> Vec<Named> {
        let mut seen = BTreeSet::from([absolute(&self.root.join(rel))]);
        let mut out = self.named(rel, doc);
        let mut i = 0;
        while i < out.len() {
            let n = &out[i];
            let parent = self.root.join(&n.timeline);
            let full = absolute(&parent.parent().unwrap_or(&self.root).join(&n.written));
            if !n.missing
                && !n.written.starts_with("http://")
                && !n.written.starts_with("https://")
                && n.written.ends_with(".timeline.json")
                && seen.insert(full.clone())
            {
                let path = self
                    .relative(&full)
                    .unwrap_or_else(|| full.display().to_string());
                if let Ok(doc) = self.document(&path) {
                    out.extend(self.named(&path, &doc));
                }
            }
            i += 1;
        }
        out
    }

    /// The project at a glance: its canvas, timelines with their tracks and clips, resources,
    /// external and missing files, snapshots and the last render.
    pub fn show(&self) -> Result<Value, Error> {
        let mut timelines = Vec::new();
        let mut named = Vec::new();
        for rel in self.timelines() {
            match self.document(&rel) {
                Ok(doc) => {
                    let clips: usize = doc.tracks.iter().map(|t| t.clips.len()).sum();
                    timelines.push(json!({
                        "path": rel,
                        "name": doc.name,
                        "canvas": {
                            "width": doc.canvas.width,
                            "height": doc.canvas.height,
                            "fps": doc.canvas.fps.to_string(),
                        },
                        "tracks": doc.tracks.len(),
                        "clips": clips,
                        "duration": doc.duration.seconds(),
                    }));
                    named.extend(self.dependencies(&rel, &doc));
                }
                Err(e) => timelines.push(json!({ "path": rel, "error": e })),
            }
        }
        let history = self.history()?;
        let mut seen = BTreeSet::new();
        named.retain(|n| seen.insert((n.timeline.clone(), n.alias.clone(), n.written.clone())));
        let listed = |pick: fn(&Named) -> bool| -> Vec<Value> {
            named.iter().filter(|n| pick(n)).map(Named::json).collect()
        };
        Ok(json!({
            "name": self.name,
            "root": self.root.display().to_string(),
            "timeline": self.timeline,
            "timelines": timelines,
            "resources": listed(|_| true),
            "external": listed(|n| n.external),
            "missing": listed(|n| n.missing),
            "snapshots": history.len(),
            "lastSnapshot": history.last().map(|s| s.summary()),
            "lastRender": self.last_render(),
        }))
    }

    /// What is wrong with the timeline at `rel` and the timelines it nests, that the project
    /// sees: missing files and component dependencies outside the project; and its external files.
    pub fn check(&self, rel: &str) -> Result<(Vec<String>, Vec<Value>), Error> {
        let (mut problems, mut external) = (Vec::new(), Vec::new());
        let doc = self.document(rel)?;
        for n in self.dependencies(rel, &doc) {
            if n.external {
                external.push(n.json());
            }
            if n.missing {
                problems.push(format!(
                    "{}: resources.{}: {}: not found",
                    n.timeline, n.alias, n.written
                ));
                continue;
            }
            let Some(path) = &n.path else {
                continue;
            };
            if path.ends_with(".motion.tsx") {
                problems.extend(self.boundary(path));
            }
        }
        Ok((problems, external))
    }

    /// Components can share project files. An implicit local dependency outside the project
    /// would be lost when the project is moved.
    fn boundary(&self, component: &str) -> Vec<String> {
        let entry = self
            .root
            .join(component)
            .to_string_lossy()
            .replace('\\', "/");
        let read = |p: &str| fs::read_to_string(p).ok();
        let files = match valle_engine::front::files(&entry, &read) {
            Ok(files) => files,
            Err(e) => return vec![e],
        };
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        files
            .into_iter()
            .filter_map(|file| {
                let path = Path::new(&file);
                let inside = path.canonicalize().map_or_else(
                    |_| self.relative(path).is_some(),
                    |target| target.starts_with(&root),
                );
                (!inside).then(|| format!("{component} reads {file}, outside the project"))
            })
            .collect()
    }
}
