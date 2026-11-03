//! `valle project …`: a project folder's commands. The project is the nearest folder up from the
//! current one with a `project.json`, or the one `-C` names; a command that takes a timeline
//! takes the entry one if none is named.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use valle_project::{Project, digest, history::now};

use crate::args::Args;
use crate::motion::counted;
use crate::{Failure, motion, timeline};

/// The words of a project command: its own options taken out, the rest as they were.
struct Words {
    folder: Option<String>,
    flags: Vec<String>,
    values: Vec<(String, String)>,
    rest: Vec<String>,
}

/// Options of the project commands that take a value, and those that do not.
const VALUED: [&str; 5] = ["-C", "--width", "--height", "--fps", "-m"];
const FLAGS: [&str; 2] = ["--link", "--cost"];

impl Words {
    fn read(words: &[String]) -> Result<Words, Failure> {
        let mut out = Words {
            folder: None,
            flags: Vec::new(),
            values: Vec::new(),
            rest: Vec::new(),
        };
        let mut words = words.iter();
        while let Some(w) = words.next() {
            if VALUED.contains(&w.as_str()) {
                let v = (words.next().cloned())
                    .ok_or_else(|| Failure::new("usage", format!("{w} needs a value")))?;
                match w.as_str() {
                    "-C" => out.folder = Some(v),
                    _ => out.values.push((w.clone(), v)),
                }
            } else if FLAGS.contains(&w.as_str()) {
                out.flags.push(w.clone());
            } else if w != "--json" && w != "--events" {
                out.rest.push(w.clone());
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&str> {
        (self.values.iter()).find_map(|(k, v)| (k == name).then_some(v.as_str()))
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    /// The words left that are not options: at most `n` of them.
    fn plain(&self, n: usize, command: &str) -> Result<Vec<&str>, Failure> {
        let plain: Vec<&str> = self.rest.iter().map(String::as_str).collect();
        if let Some(o) = plain.iter().find(|w| w.starts_with('-')) {
            return Err(
                Failure::new("usage", format!("project {command} takes no {o}"))
                    .hint("valle --help lists the options"),
            );
        }
        if plain.len() > n {
            return Err(
                Failure::new("usage", format!("project {command}: too many words"))
                    .hint("valle --help lists the commands"),
            );
        }
        Ok(plain)
    }

    fn project(&self) -> Result<Project, Failure> {
        let start = PathBuf::from(self.folder.as_deref().unwrap_or("."));
        Project::discover(&start).map_err(|e| Failure::new("project", e))
    }
}

fn failed(e: String) -> Failure {
    Failure::new("project", e)
}

pub fn run(command: &str, words: &[String]) -> Result<Value, Failure> {
    let w = Words::read(words)?;
    match command {
        "create" => create(&w),
        "show" => show(&w),
        "add" => add(&w),
        "check" => check(&w),
        "save" => save(&w),
        "history" => history(&w),
        "restore" => restore(&w),
        "render" => render(&w),
        _ => Err(
            Failure::new("usage", format!("no command project {command}"))
                .hint("valle --help lists the commands"),
        ),
    }
}

fn create(w: &Words) -> Result<Value, Failure> {
    let [dir] = w.plain(1, "create")?[..] else {
        return Err(Failure::new("usage", "project create <folder>"));
    };
    let size = |name: &str, default: u32| -> Result<u32, Failure> {
        match w.value(name) {
            None => Ok(default),
            Some(v) => (v.parse().ok().filter(|n| *n > 0))
                .ok_or_else(|| Failure::new("usage", format!("{name} is pixels above 0, not {v}"))),
        }
    };
    let (width, height) = (size("--width", 1920)?, size("--height", 1080)?);
    let fps = w.value("--fps").unwrap_or("30");
    let p = Project::create(Path::new(dir), width, height, fps).map_err(failed)?;
    Ok(json!({
        "status": "ok",
        "summary": format!("created {} in {}: {width}×{height} at {fps} fps", p.name, p.root.display()),
        "root": p.root.display().to_string(),
        "timeline": p.timeline,
    }))
}

fn show(w: &Words) -> Result<Value, Failure> {
    w.plain(0, "show")?;
    let p = w.project()?;
    let mut out = p.show().map_err(failed)?;
    let mut lines = vec![format!("{} ({})", p.name, p.root.display())];
    for t in out["timelines"].as_array().into_iter().flatten() {
        let path = t["path"].as_str().unwrap_or_default();
        lines.push(match t["error"].as_str() {
            Some(e) => format!("  {path}: {e}"),
            None => format!(
                "  {path}: {}×{} at {} fps, {}, {}, {} s",
                t["canvas"]["width"],
                t["canvas"]["height"],
                t["canvas"]["fps"].as_str().unwrap_or_default(),
                counted(t["tracks"].as_u64().unwrap_or(0) as usize, "track"),
                counted(t["clips"].as_u64().unwrap_or(0) as usize, "clip"),
                t["duration"]
            ),
        });
    }
    for (key, what) in [
        ("resources", "resource"),
        ("external", "external file"),
        ("missing", "missing file"),
    ] {
        let n = out[key].as_array().map_or(0, Vec::len);
        if n > 0 || key == "resources" {
            lines.push(format!("  {}", counted(n, what)));
        }
    }
    for m in out["missing"].as_array().into_iter().flatten() {
        lines.push(format!(
            "    {} → {} ({})",
            m["alias"].as_str().unwrap_or_default(),
            m["file"].as_str().unwrap_or_default(),
            m["timeline"].as_str().unwrap_or_default()
        ));
    }
    let snapshots = out["snapshots"].as_u64().unwrap_or(0) as usize;
    lines.push(format!("  {}", counted(snapshots, "snapshot")));
    if let Some(r) = out["lastRender"]["output"].as_str() {
        lines.push(format!("  last render: {r}"));
    }
    out["status"] = "ok".into();
    out["summary"] = lines.join("\n").into();
    Ok(out)
}

fn add(w: &Words) -> Result<Value, Failure> {
    let files = w.plain(usize::MAX, "add")?;
    if files.is_empty() {
        return Err(Failure::new("usage", "project add <file or folder>…"));
    }
    let p = w.project()?;
    let from: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
    let mut out = p.add(&from, w.flag("--link")).map_err(failed)?;
    let mut lines: Vec<String> = (out["added"].as_array().into_iter().flatten())
        .map(|a| {
            let again = if a["reused"] == true {
                " (already there)"
            } else {
                ""
            };
            format!(
                "{}{again}: \"{}\": {}",
                a["path"].as_str().unwrap_or_default(),
                a["alias"].as_str().unwrap_or_default(),
                a["resource"][a["alias"].as_str().unwrap_or_default()]
            )
        })
        .collect();
    for f in out["failed"].as_array().into_iter().flatten() {
        lines.push(format!(
            "not added: {}",
            f["message"].as_str().unwrap_or_default()
        ));
    }
    out["summary"] = lines.join("\n").into();
    Ok(out)
}

fn check(w: &Words) -> Result<Value, Failure> {
    let named = w.plain(1, "check")?;
    let p = w.project()?;
    let path = p.timeline(named.first().copied()).map_err(failed)?;
    let rel = p
        .relative(&path)
        .ok_or_else(|| failed(format!("{} is not in the project", path.display())))?;
    let (mut problems, external) = p.check(&rel).map_err(failed)?;
    let args = timeline_args(&path);
    if problems.is_empty() {
        let lowered = match w.flag("--cost") {
            true => motion::check(&args).map(|_| ()),
            false => timeline::lower(&args).map(|_| ()),
        };
        if let Err(f) = lowered {
            if f.problems.is_empty() {
                problems.push(f.message);
            } else {
                return Err(f);
            }
        }
    }
    if !problems.is_empty() {
        let mut f = Failure::new(
            "check",
            format!("{rel}: {}", counted(problems.len(), "problem")),
        );
        f.problems = problems
            .into_iter()
            .map(|m| json!({ "message": m }))
            .collect();
        return Err(f.hint("fix each, then check again"));
    }
    let frames = if w.flag("--cost") {
        ", every frame planned"
    } else {
        ""
    };
    let mut summary = format!("{rel}: no problems{frames}");
    if !external.is_empty() {
        summary += &format!("; {} out of the project", counted(external.len(), "file"));
    }
    Ok(json!({ "status": "ok", "summary": summary, "timeline": rel, "external": external }))
}

fn save(w: &Words) -> Result<Value, Failure> {
    w.plain(0, "save")?;
    let p = w.project()?;
    let (n, new) = p.save(w.value("-m"), "save").map_err(failed)?;
    let summary = match new {
        true => format!("saved snapshot {n}"),
        false => format!("nothing changed since snapshot {n}"),
    };
    Ok(json!({ "status": "ok", "summary": summary, "snapshot": n, "new": new }))
}

fn history(w: &Words) -> Result<Value, Failure> {
    w.plain(0, "history")?;
    let p = w.project()?;
    let history = p.history().map_err(failed)?;
    let lines: Vec<String> = (history.iter())
        .map(|s| {
            let message = s
                .message
                .as_deref()
                .map(|m| format!(": {m}"))
                .unwrap_or_default();
            format!("{} ({}, {} files){message}", s.number, s.by, s.files.len())
        })
        .collect();
    let summary = match lines.is_empty() {
        true => "no snapshots yet: valle project save makes one".to_string(),
        false => lines.join("\n"),
    };
    let snapshots: Vec<Value> = history.iter().map(|s| s.summary()).collect();
    Ok(json!({ "status": "ok", "summary": summary, "snapshots": snapshots }))
}

fn restore(w: &Words) -> Result<Value, Failure> {
    let [n] = w.plain(1, "restore")?[..] else {
        return Err(Failure::new("usage", "project restore <snapshot>"));
    };
    let n: u64 = (n.parse().ok().filter(|n| *n > 0))
        .ok_or_else(|| Failure::new("usage", format!("a snapshot is a number from 1, not {n}")))?;
    let p = w.project()?;
    let mut out = p.restore(n).map_err(failed)?;
    let mut summary = format!(
        "restored snapshot {n}: {} put back, {} unchanged; what was there is snapshot {}",
        counted(out["restored"].as_array().map_or(0, Vec::len), "file"),
        out["unchanged"],
        out["backup"]
    );
    for s in out["skipped"].as_array().into_iter().flatten() {
        summary += &format!(
            "\nleft: {} ({})",
            s["path"].as_str().unwrap_or_default(),
            s["reason"].as_str().unwrap_or_default()
        );
    }
    for a in out["added"].as_array().into_iter().flatten() {
        summary += &format!("\nkept, added since: {}", a.as_str().unwrap_or_default());
    }
    out["status"] = "ok".into();
    out["summary"] = summary.into();
    Ok(out)
}

/// A timeline's arguments as `valle timeline check` reads them.
fn timeline_args(path: &Path) -> Args {
    Args {
        file: path.to_string_lossy().into_owned(),
        timeline: true,
        ..Args::default()
    }
}

/// Renders a timeline, by default into `renders/`. A full render, all frames into a video,
/// first saves a snapshot, and leaves a record of what it read and made beside it.
fn render(w: &Words) -> Result<Value, Failure> {
    let mut rest = w.rest.clone();
    // The timeline is the one word not an option or an option's value.
    let mut named = None;
    let mut i = 0;
    while i < rest.len() {
        let word = &rest[i];
        if word.starts_with('-') {
            i += if ["--storyboard"].contains(&word.as_str()) {
                1
            } else {
                2
            };
        } else if named.is_none() {
            named = Some(rest.remove(i));
        } else {
            return Err(Failure::new(
                "usage",
                format!("project render: one timeline, not also {word}"),
            ));
        }
    }
    let p = w.project()?;
    let path = p.timeline(named.as_deref()).map_err(failed)?;
    let rel = p
        .relative(&path)
        .unwrap_or_else(|| path.display().to_string());
    let mut words = vec![path.to_string_lossy().into_owned()];
    words.extend(rest);
    let mut args = Args::parse(&words)?;
    args.timeline = true;
    let video = (args.out.as_deref()).is_none_or(|o| {
        let o = o.to_ascii_lowercase();
        o.ends_with(".mp4") || o.ends_with(".mov")
    });
    let full = video && args.frames.is_none() && !args.storyboard;
    let doc = p.document(&rel).map_err(failed)?;
    let (snapshot, _) = match full {
        true => p
            .save(None, "render")
            .map(|(n, new)| (Some(n), new))
            .map_err(failed)?,
        false => (None, false),
    };
    if args.out.is_none() {
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap_or("timeline")
            .trim_end_matches(".json")
            .trim_end_matches(".timeline");
        let transparent = !doc.canvas.background.to_ascii_lowercase().ends_with("ff");
        let mov = transparent || args.option("codec") == Some("prores");
        let name = match (snapshot, args.storyboard) {
            (Some(n), _) => format!("{stem}-{n}.{}", if mov { "mov" } else { "mp4" }),
            (None, true) => format!("{stem}-storyboard.png"),
            (None, false) => format!("{stem}-frames"),
        };
        args.out = Some(
            free(&p.root.join("renders").join(name))
                .display()
                .to_string(),
        );
    }
    let out = args.out.clone().expect("an output");
    let inputs = media(&p, &rel, &doc);
    let mut result = motion::render(&args)?;
    let Some(n) = snapshot.filter(|_| Path::new(&out).is_file()) else {
        return Ok(result);
    };
    // Media changed while it was read: the render is kept, and says so.
    let warnings: Vec<String> = (inputs.iter())
        .filter(|(file, sha)| digest(Path::new(file)).ok().as_ref() != Some(sha))
        .map(|(file, _)| {
            format!("{file} changed during the render: it shows the file as it was when read")
        })
        .collect();
    let output = p.relative(Path::new(&out)).unwrap_or_else(|| out.clone());
    let record = json!({
        "time": now(),
        "timeline": rel,
        "output": output,
        "sha256": digest(Path::new(&out)).ok(),
        "inputs": inputs.iter().map(|(f, sha)| json!({ "file": f, "sha256": sha })).collect::<Vec<_>>(),
        "options": args.options.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "warnings": warnings,
        "valle": env!("CARGO_PKG_VERSION"),
    });
    p.record(n, record).map_err(failed)?;
    result["snapshot"] = n.into();
    result["warnings"] = json!(warnings);
    if let Some(s) = result["summary"].as_str() {
        let mut s = format!("{s} (snapshot {n})");
        for w in &warnings {
            s += &format!("\nwarning: {w}");
        }
        result["summary"] = s.into();
    }
    Ok(result)
}

/// The media files on disk a timeline names, with their SHA-256 before it renders.
fn media(p: &Project, rel: &str, doc: &valle_timeline::Document) -> Vec<(String, String)> {
    let folder = p.root.join(rel);
    let folder = folder.parent().unwrap_or(&p.root).to_path_buf();
    let mut files: Vec<(String, String)> = (p.named(rel, doc).into_iter())
        .filter(|n| !n.missing && !n.written.starts_with("http"))
        .filter_map(|n| {
            let full = valle_project::absolute(&folder.join(&n.written));
            let sha = digest(&full).ok()?;
            Some((full.display().to_string(), sha))
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// `path`, or with `-2`, `-3`… before its extension if something is there.
fn free(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    (2..)
        .map(|k| path.with_file_name(format!("{stem}-{k}{ext}")))
        .find(|p| !p.exists())
        .expect("a free name")
}
