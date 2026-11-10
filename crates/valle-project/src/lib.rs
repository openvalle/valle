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
        valle_engine::entry::Kind::Timeline
            .check(&timeline)
            .map_err(|e| bad(&e))?;
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
        let timeline = root.join("main.timeline.json");
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
        let config = json!({ "name": name, "timeline": "main.timeline.json" });
        write(&root.join(FILE), &pretty(&config), false)?;
        Project::open(&root)
    }

    /// Every timeline, as a path in the project: the entry, then those in `timelines/` by name.
    pub fn timelines(&self) -> Vec<String> {
        let mut more = Vec::new();
        history::walk(&self.root, "timelines", &mut more);
        more.retain(|p| p.ends_with(".timeline.json"));
        // A referenced timeline can live outside timelines/, but still belongs to the edit.
        for rel in std::iter::once(self.timeline.clone()).chain(more.clone()) {
            if let Ok(doc) = self.document(&rel) {
                for n in self.dependencies(&rel, &doc) {
                    if let Some(path) = n.path
                        && path.ends_with(".timeline.json")
                        && self.document(&path).is_ok()
                    {
                        more.push(path);
                    }
                }
            }
        }
        more.retain(|p| *p != self.timeline);
        more.sort();
        more.dedup();
        std::iter::once(self.timeline.clone()).chain(more).collect()
    }

    /// The timeline a command names, or the entry: a path from the current folder, or one in
    /// the project, or a name in `timelines/` with or without `.timeline.json`.
    pub fn timeline(&self, name: Option<&str>) -> Result<PathBuf, Error> {
        let Some(name) = name else {
            return Ok(self.root.join(&self.timeline));
        };
        let mut tried = vec![absolute(Path::new(name)), self.root.join(name)];
        tried.push(self.root.join("timelines").join(name));
        tried.push(
            self.root
                .join("timelines")
                .join(format!("{name}.timeline.json")),
        );
        (tried
            .into_iter()
            .find(|p| p.is_file() && p.to_string_lossy().ends_with(".timeline.json")))
        .ok_or_else(|| {
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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use crate::{Project, pretty};

    /// An empty folder of its own for a test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("valle-project-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
        fs::write(path, text).expect("written");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read")
    }

    /// A project with a timeline playing one solid clip, and its resources as given.
    fn project(name: &str, resources: serde_json::Value) -> Project {
        let dir = folder(name);
        let p = Project::create(&dir.join("promo"), 640, 360, "30000/1001").expect("created");
        let timeline = json!({
            "canvas": { "width": 640, "height": 360, "fps": "30000/1001" },
            "resources": resources,
            "tracks": { "visual": [{ "clips": [{ "kind": "solid", "color": "#000000ff", "start": 0, "duration": 1 }] }] }
        });
        fs::write(p.root.join("main.timeline.json"), pretty(&timeline)).expect("timeline");
        p
    }

    #[test]
    fn a_project_is_found_from_any_folder_in_it() {
        let dir = folder("found");
        let p = Project::create(&dir.join("promo"), 1920, 1080, "30").expect("created");
        assert_eq!(
            (p.name.as_str(), p.timeline.as_str()),
            ("promo", "main.timeline.json")
        );
        for sub in [
            "timelines",
            "components",
            "media",
            "renders",
            ".valle/history",
        ] {
            assert!(p.root.join(sub).is_dir(), "{sub}");
        }
        let found = Project::discover(&p.root.join("media")).expect("found");
        assert_eq!(found.root, p.root);
        assert!(Project::create(&p.root, 1920, 1080, "30").is_err());
        assert!(
            Project::discover(&dir)
                .unwrap_err()
                .contains("project create")
        );
        assert!(Project::create(&dir.join("bad"), 1920, 1080, "fast").is_err());
        // A path out of the project is not its timeline.
        put(
            &dir.join("odd/project.json"),
            r#"{"name":"odd","timeline":"../t.timeline.json"}"#,
        );
        assert!(Project::open(&dir.join("odd")).is_err());
    }

    #[test]
    fn snapshots_and_dependencies_include_nested_timelines() {
        let p = project(
            "nested",
            json!({ "child": "timelines/parts/child.timeline.json" }),
        );
        let external = folder("nested-external").join("voice.wav");
        put(&external, "sound");
        put(&p.root.join("media/clip.mp4"), "video");
        let mut child: serde_json::Value =
            serde_json::from_str(&read(&p.root.join("main.timeline.json"))).unwrap();
        child["resources"] =
            json!({ "video": "../../media/clip.mp4", "next": "../../edits/leaf.timeline.json" });
        let path = p.root.join("timelines/parts/child.timeline.json");
        put(&path, &child.to_string());
        let mut leaf = child.clone();
        leaf["resources"] = json!({ "back": "../main.timeline.json", "voice": external });
        put(&p.root.join("edits/leaf.timeline.json"), &leaf.to_string());
        assert_eq!(
            p.timelines(),
            [
                "main.timeline.json",
                "edits/leaf.timeline.json",
                "timelines/parts/child.timeline.json"
            ]
        );
        let deps = p.dependencies(
            "main.timeline.json",
            &p.document("main.timeline.json").unwrap(),
        );
        assert_eq!(deps.len(), 5);
        assert!(deps.iter().any(|n| n.alias == "voice" && n.external));
        assert_eq!(p.show().unwrap()["resources"].as_array().unwrap().len(), 5);
        assert_eq!(p.save(None, "save").unwrap(), (1, true));
        let original = read(&path);
        child["name"] = json!("changed");
        put(&path, &child.to_string());
        assert_eq!(p.save(None, "save").unwrap(), (2, true));
        p.restore(1).unwrap();
        assert_eq!(read(&path), original);
        put(&p.root.join("timelines/short.timeline.json"), &original);
        assert_eq!(
            p.timeline(Some("short")).unwrap(),
            p.root.join("timelines/short.timeline.json")
        );
    }

    #[test]
    fn timeline_names_do_not_pick_up_ordinary_json() {
        let p = project("entry-names", json!({ "data": "timelines/data.json" }));
        let timeline = read(&p.root.join(&p.timeline));
        put(&p.root.join("timelines/data.json"), &timeline);
        put(&p.root.join("timelines/cut.timeline.json"), &timeline);
        assert_eq!(
            p.timelines(),
            ["main.timeline.json", "timelines/cut.timeline.json"]
        );
        for name in ["cut", "cut.timeline.json", "timelines/cut.timeline.json"] {
            assert_eq!(
                p.timeline(Some(name)).unwrap(),
                p.root.join("timelines/cut.timeline.json")
            );
        }
        assert!(p.timeline(Some("timelines/data.json")).is_err());
        assert!(
            p.document("timelines/data.json")
                .err()
                .unwrap()
                .contains(".timeline.json")
        );
        put(
            &p.root.join("project.json"),
            r#"{"name":"bad","timeline":"timelines/data.json"}"#,
        );
        assert!(
            Project::open(&p.root)
                .unwrap_err()
                .contains(".timeline.json")
        );
    }

    #[test]
    fn copied_timelines_keep_their_compound_suffix_and_alias_stem() {
        let p = project("timeline-copy", json!({}));
        let out = folder("timeline-copy-src");
        let text = read(&p.root.join(&p.timeline));
        put(&out.join("cut.timeline.json"), &text);
        let mut other: serde_json::Value = serde_json::from_str(&text).unwrap();
        other["name"] = json!("another cut");
        put(&out.join("other/cut.timeline.json"), &other.to_string());
        let added = p
            .add(
                &[
                    out.join("cut.timeline.json"),
                    out.join("other/cut.timeline.json"),
                ],
                false,
            )
            .unwrap();
        assert_eq!(added["added"][0]["alias"], "cut");
        assert_eq!(added["added"][1]["alias"], "cut-2");
        assert_eq!(added["added"][0]["path"], "media/cut.timeline.json");
        assert_eq!(added["added"][1]["path"], "media/cut-2.timeline.json");
        assert_eq!(
            p.timelines(),
            [
                "main.timeline.json",
                "media/cut-2.timeline.json",
                "media/cut.timeline.json"
            ]
        );
    }

    #[test]
    fn files_are_copied_in_once_and_named_apart() {
        let p = project("add", json!({}));
        let out = folder("add-src");
        put(&out.join("clip.mp4"), "one");
        put(&out.join("other/Clip.MP4"), "two");
        put(
            &out.join("title.motion.tsx"),
            "export default function T() { return null; }\n",
        );
        let added = p
            .add(&[out.join("clip.mp4"), out.join("title.motion.tsx")], false)
            .expect("added");
        assert_eq!(added["status"], "ok");
        assert_eq!(added["added"][0]["path"], "media/clip.mp4");
        assert_eq!(
            added["added"][0]["resource"],
            json!({ "clip": "media/clip.mp4" })
        );
        assert_eq!(added["added"][1]["path"], "components/title.motion.tsx");
        // The same content again is the same file; other content of a name taken, in any case, is
        // named apart.
        let again = p
            .add(&[out.join("clip.mp4"), out.join("other/Clip.MP4")], false)
            .expect("again");
        assert_eq!(again["added"][0]["reused"], true);
        assert_eq!(again["added"][0]["alias"], "clip");
        assert_eq!(again["added"][1]["path"], "media/Clip-2.MP4");
        assert_eq!(read(&p.root.join("media/clip.mp4")), "one");
        // The original can go: the project keeps its copy.
        fs::remove_file(out.join("clip.mp4")).expect("removed");
        assert_eq!(read(&p.root.join("media/clip.mp4")), "one");
        // Linked, it stays where it is.
        let linked = p.add(&[out.join("other/Clip.MP4")], true).expect("linked");
        let path = linked["added"][0]["path"].as_str().expect("a path");
        assert!(Path::new(path).is_absolute(), "{path}");
        let doc = p.document("main.timeline.json").unwrap();
        assert_eq!(doc.resources["clip"].files, ["media/clip.mp4"]);
    }

    #[test]
    fn a_media_folder_binds_each_file_in_the_entry() {
        let dir = folder("folder-import");
        let p = Project::create(&dir.join("project"), 640, 360, "30").unwrap();
        let source = dir.join("shots");
        put(&source.join("a.mp4"), "first");
        put(&source.join("more/a.mp4"), "second");
        put(&source.join(".hidden"), "ignored");
        let added = p.add(std::slice::from_ref(&source), false).unwrap();
        assert_eq!(added["added"].as_array().unwrap().len(), 2);
        let saved = read(&p.root.join("main.timeline.json"));
        let doc: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            doc["resources"],
            json!({"a":"media/shots/a.mp4", "a-2":"media/shots/more/a.mp4"})
        );
        assert_eq!(doc["tracks"], json!({}));
        assert_eq!(p.show().unwrap()["resources"].as_array().unwrap().len(), 2);
        let again = p.add(&[source], false).unwrap();
        assert_eq!(again["added"][0]["alias"], "a");
        assert_eq!(again["added"][1]["alias"], "a-2");
        assert_eq!(read(&p.root.join("main.timeline.json")), saved);
        let existing = p.add(&[p.root.join("media/shots/a.mp4")], false).unwrap();
        assert_eq!(existing["added"][0]["alias"], "a");
    }

    #[test]
    fn some_files_failing_keeps_the_others() {
        let p = project("partial", json!({}));
        let out = folder("partial-src");
        put(&out.join("a.png"), "png");
        let added = p
            .add(&[out.join("a.png"), out.join("gone.png")], false)
            .expect("partial");
        assert_eq!(added["status"], "partial");
        assert_eq!(
            added["failed"][0]["from"],
            out.join("gone.png").display().to_string()
        );
        assert!(p.root.join("media/a.png").is_file());
        assert!(p.add(&[out.join("gone.png")], false).is_err());
    }

    #[test]
    fn a_component_with_its_own_files_is_added_as_its_folder() {
        let p = project("component", json!({}));
        let out = folder("component-src");
        let scene = "import { K } from \"./k\";\nexport default function C() { return null; }\n";
        put(&out.join("chart/chart.motion.tsx"), scene);
        put(&out.join("chart/k.ts"), "export const K = 1;\n");
        let lone = p
            .add(&[out.join("chart/chart.motion.tsx")], false)
            .unwrap_err();
        assert!(lone.contains("add its folder"), "{lone}");
        let added = p.add(&[out.join("chart")], false).expect("added");
        assert_eq!(added["added"][0]["path"], "components/chart");
        assert_eq!(
            added["added"][0]["resource"],
            json!({ "chart": "components/chart/chart.motion.tsx" })
        );
        assert_eq!(
            read(&p.root.join("components/chart/k.ts")),
            "export const K = 1;\n"
        );
    }

    #[test]
    fn components_keep_to_their_folders() {
        let scene =
            |import: &str| format!("{import}\nexport default function C() {{ return null; }}\n");
        let p = project(
            "boundary",
            json!({
                "lone": "components/lone.motion.tsx",
                "inside": "components/ok/ok.motion.tsx",
                "outside": "components/bad/bad.motion.tsx",
                "gone": "media/gone.mp4",
                "web": "https://example.com/a.png",
            }),
        );
        put(
            &p.root.join("components/lone.motion.tsx"),
            &scene("import { K } from \"./k\";"),
        );
        put(&p.root.join("components/k.ts"), "export const K = 1;\n");
        put(
            &p.root.join("components/ok/ok.motion.tsx"),
            &scene("import { K } from \"./k\";"),
        );
        put(&p.root.join("components/ok/k.ts"), "export const K = 1;\n");
        put(
            &p.root.join("components/bad/bad.motion.tsx"),
            &scene("import { K } from \"../k\";"),
        );
        let (problems, external) = p.check("main.timeline.json").expect("checked");
        let has = |what: &str| problems.iter().any(|m| m.contains(what));
        assert!(
            has("components/lone.motion.tsx reads components/k.ts"),
            "{problems:?}"
        );
        assert!(
            has("components/bad/bad.motion.tsx reads components/k.ts"),
            "{problems:?}"
        );
        assert!(has("resource gone names media/gone.mp4"), "{problems:?}");
        assert!(!has("ok.motion.tsx"), "{problems:?}");
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(external.len(), 1);
        assert_eq!(external[0]["file"], "https://example.com/a.png");
        let shown = p.show().expect("shown");
        assert_eq!(shown["timelines"][0]["clips"], 1);
        assert_eq!(shown["timelines"][0]["canvas"]["fps"], "30000/1001");
        assert_eq!(shown["missing"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn a_restore_puts_the_sources_back_and_can_be_undone() {
        let p = project("restore", json!({}));
        put(&p.root.join("media/data.json"), "[1]");
        put(&p.root.join("media/clip.mp4"), "video");
        put(&p.root.join("timelines/cut.timeline.json"), "{}");
        let (one, new) = p.save(Some("first"), "save").expect("saved");
        assert_eq!((one, new), (1, true));
        // Nothing changed: the same snapshot.
        assert_eq!(p.save(None, "save").expect("saved"), (1, false));
        let first = read(&p.root.join("main.timeline.json"));
        put(&p.root.join("main.timeline.json"), "broken");
        put(&p.root.join("media/data.json"), "[2]");
        put(&p.root.join("media/clip.mp4"), "edited video");
        put(&p.root.join("components/new.motion.tsx"), "added");
        let restored = p.restore(1).expect("restored");
        assert_eq!(restored["backup"], 2);
        assert_eq!(read(&p.root.join("main.timeline.json")), first);
        assert_eq!(read(&p.root.join("media/data.json")), "[1]");
        // Media is never written, and a file added since stays.
        assert_eq!(read(&p.root.join("media/clip.mp4")), "edited video");
        assert_eq!(restored["skipped"][0]["path"], "media/clip.mp4");
        assert_eq!(restored["added"], json!(["components/new.motion.tsx"]));
        // The backup has what the restore replaced.
        p.restore(2).expect("undone");
        assert_eq!(read(&p.root.join("main.timeline.json")), "broken");
        assert_eq!(read(&p.root.join("media/data.json")), "[2]");
        let history = p.history().expect("history");
        assert_eq!(
            history.iter().map(|s| s.number).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(history[1].by, "restore");
        assert_eq!(history[0].message.as_deref(), Some("first"));
        let kept = |n: usize, path: &str| {
            history[n]
                .files
                .iter()
                .find(|f| f.path == path)
                .map(|f| f.kept)
        };
        assert_eq!(kept(0, "media/clip.mp4"), Some(false));
        assert_eq!(kept(0, "timelines/cut.timeline.json"), Some(true));
    }

    #[test]
    fn a_render_is_recorded_with_its_snapshot() {
        let p = project("record", json!({}));
        let (n, _) = p.save(None, "render").expect("saved");
        assert!(p.last_render().is_none());
        p.record(n, json!({ "output": "renders/a.mp4" }))
            .expect("recorded");
        p.record(n, json!({ "output": "renders/b.mp4" }))
            .expect("recorded");
        assert_eq!(
            p.last_render().expect("a render")["output"],
            "renders/b.mp4"
        );
        assert_eq!(p.records(n).len(), 2);
    }
}
