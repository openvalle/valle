//! `valle project …`: a project folder's commands. The project is the nearest folder up from the
//! current one with a `project.json`, or the one `-C` names; a command that takes a timeline
//! takes the entry one if none is named.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use valle_project::{Project, digest, history::now};

use valle_native::media::Codec;

use crate::args::Args;
use crate::cli::{self, ProjectCommand, RenderOptions};
use crate::motion::counted;
use crate::{Failure, motion, timeline};

/// The project in `folder`, or the nearest one up from the current folder.
fn project(folder: Option<&Path>) -> Result<Project, Failure> {
    Project::discover(folder.unwrap_or(Path::new("."))).map_err(|e| Failure::new("project", e))
}

fn failed(e: String) -> Failure {
    Failure::new("project", e)
}

pub fn run(folder: Option<&Path>, command: ProjectCommand) -> Result<Value, Failure> {
    match command {
        ProjectCommand::Create {
            folder,
            width,
            height,
            fps,
        } => create(&folder, width, height, &fps),
        ProjectCommand::Show => show(&project(folder)?),
        ProjectCommand::Add { paths, link } => add(&project(folder)?, &paths, link),
        ProjectCommand::Check { timeline, cost } => check(&project(folder)?, timeline, cost),
        ProjectCommand::Save { message } => save(&project(folder)?, message.as_deref()),
        ProjectCommand::History => history(&project(folder)?),
        ProjectCommand::Restore { snapshot } => restore(&project(folder)?, snapshot),
        ProjectCommand::Render {
            timeline,
            out,
            options,
        } => render(&project(folder)?, timeline, out, options),
        ProjectCommand::Studio { .. } => unreachable!("main runs Studio"),
    }
}

fn create(dir: &Path, width: u32, height: u32, fps: &str) -> Result<Value, Failure> {
    let p = Project::create(dir, width, height, fps).map_err(failed)?;
    Ok(json!({
        "status": "ok",
        "summary": format!(
            "created {} in {}: {width}×{height} at {fps} fps\nnext: valle project add <files> \
             (run in {}), then put clips in {}; valle docs project",
            p.name,
            p.root.display(),
            p.root.display(),
            p.timeline
        ),
        "root": p.root.display().to_string(),
        "timeline": p.timeline,
    }))
}

fn show(p: &Project) -> Result<Value, Failure> {
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

/// How a timeline clip shows the file at `path`, by its alias.
fn clip_of(path: &str, alias: &str) -> String {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        _ if path.starts_with("components/") || path.ends_with(".motion.tsx") => {
            format!("{{ \"kind\": \"motion\", \"component\": \"{alias}\" }}")
        }
        "png" | "jpg" | "jpeg" | "webp" | "svg" => {
            format!("{{ \"kind\": \"image\", \"src\": \"{alias}\" }}")
        }
        "wav" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" => {
            format!("{{ \"src\": \"{alias}\" }} on an audio track")
        }
        "ttf" | "otf" | "woff" | "woff2" => {
            format!("\"fontFamily\": \"{alias}\" in a caption style")
        }
        _ if path.ends_with(".timeline.json") => {
            format!("{{ \"kind\": \"timeline\", \"src\": \"{alias}\" }}")
        }
        _ => format!("{{ \"kind\": \"video\", \"src\": \"{alias}\" }}"),
    }
}

fn add(p: &Project, from: &[PathBuf], link: bool) -> Result<Value, Failure> {
    let mut out = p.add(from, link).map_err(failed)?;
    // Each file by the name its clips use, and how a clip shows it.
    for a in out["added"].as_array_mut().into_iter().flatten() {
        let clip = clip_of(
            a["path"].as_str().unwrap_or_default(),
            a["alias"].as_str().unwrap_or_default(),
        );
        a["clip"] = clip.into();
    }
    let mut lines: Vec<String> = (out["added"].as_array().into_iter().flatten())
        .map(|a| {
            let alias = a["alias"].as_str().unwrap_or_default();
            let path = a["path"].as_str().unwrap_or_default();
            let again = if a["reused"] == true {
                ", already there"
            } else {
                ""
            };
            let clip = a["clip"].as_str().unwrap_or_default();
            format!("{alias}: {path}{again}; a clip shows it as {clip}")
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

fn check(p: &Project, timeline: Option<String>, cost: bool) -> Result<Value, Failure> {
    let path = p.timeline(timeline.as_deref()).map_err(failed)?;
    let rel = p
        .relative(&path)
        .ok_or_else(|| failed(format!("{} is not in the project", path.display())))?;
    let (mut problems, external) = p.check(&rel).map_err(failed)?;
    let args = timeline_args(&path);
    if problems.is_empty() {
        let lowered = match cost {
            true => motion::check(&args).map(|_| ()),
            false => timeline::lower(&args).map(|_| ()),
        };
        // The timeline by its path in the project, as the summary names it.
        let full = path.to_string_lossy().replace('\\', "/");
        let near = |m: &str| m.replace(&full, &rel);
        if let Err(mut f) = lowered {
            f.message = near(&f.message);
            if f.problems.is_empty() {
                problems.push(f.message);
            } else {
                for p in &mut f.problems {
                    if let Some(m) = p["message"].as_str() {
                        p["message"] = near(m).into();
                    }
                }
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
    let frames = if cost { ", every frame planned" } else { "" };
    let mut summary = format!("{rel}: no problems{frames}");
    if !external.is_empty() {
        summary += &format!("; {} out of the project", counted(external.len(), "file"));
    }
    Ok(json!({ "status": "ok", "summary": summary, "timeline": rel, "external": external }))
}

fn save(p: &Project, message: Option<&str>) -> Result<Value, Failure> {
    let (n, new) = p.save(message, "save").map_err(failed)?;
    let summary = match new {
        true => format!("saved snapshot {n}"),
        false => format!("nothing changed since snapshot {n}"),
    };
    Ok(json!({ "status": "ok", "summary": summary, "snapshot": n, "new": new }))
}

fn history(p: &Project) -> Result<Value, Failure> {
    let history = p.history().map_err(failed)?;
    let lines: Vec<String> = (history.iter())
        .map(|s| {
            let message = s
                .message
                .as_deref()
                .map(|m| format!(": {m}"))
                .unwrap_or_default();
            format!(
                "snapshot {} ({}, {}){message}",
                s.number,
                s.by,
                counted(s.files.len(), "file")
            )
        })
        .collect();
    let summary = match lines.is_empty() {
        true => "no snapshots yet: valle project save makes one".to_string(),
        false => lines.join("\n"),
    };
    let snapshots: Vec<Value> = history.iter().map(|s| s.summary()).collect();
    Ok(json!({ "status": "ok", "summary": summary, "snapshots": snapshots }))
}

fn restore(p: &Project, n: u64) -> Result<Value, Failure> {
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
fn render(
    p: &Project,
    timeline: Option<String>,
    out: Option<String>,
    options: RenderOptions,
) -> Result<Value, Failure> {
    let path = p.timeline(timeline.as_deref()).map_err(failed)?;
    let rel = p
        .relative(&path)
        .unwrap_or_else(|| path.display().to_string());
    let file = path.to_string_lossy().into_owned();
    let mut args = cli::project_render_args(file, out, options);
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
        let directory = p.root.join("renders");
        std::fs::create_dir_all(&directory)
            .map_err(|e| failed(format!("{}: {e}", directory.display())))?;
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap_or("timeline")
            .trim_end_matches(".json")
            .trim_end_matches(".timeline");
        let transparent =
            valle_engine::style::color::parse(&doc.canvas.background.trim().to_ascii_lowercase())
                .map_err(failed)?[3]
                < 1.0;
        let mov = transparent || args.codec == Some(Codec::ProRes4444);
        let name = match (snapshot, args.storyboard) {
            (Some(n), _) => format!("{stem}-{n}.{}", if mov { "mov" } else { "mp4" }),
            (None, true) => format!("{stem}-storyboard.png"),
            (None, false) => format!("{stem}-frames"),
        };
        args.out = Some(free(&directory.join(name)).display().to_string());
    }
    let out = args.out.clone().expect("an output");
    let inputs = media(p, &rel, &doc);
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
        "options": args.options().iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
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
    let mut files: Vec<(String, String)> = (p.dependencies(rel, doc).into_iter())
        .filter(|n| !n.missing && !n.written.starts_with("http"))
        .filter_map(|n| {
            let timeline = p.root.join(&n.timeline);
            let folder = timeline.parent().unwrap_or(&p.root);
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
