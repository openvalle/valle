//! `valle project …`: a project folder's commands. The project is the nearest folder up from the
//! current one with a `project.json`, or the one `-C` names; a command that takes a timeline
//! takes the entry one if none is named.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use valle_project::notes::{NewNote, Note, Region, Target};
use valle_project::{Project, digest, history::now};

use valle_native::media::{self, Codec};

use crate::args::Args;
use crate::cli::{self, NotesCommand, ProjectCommand, RenderOptions};
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
        ProjectCommand::Notes { command } => notes(&project(folder)?, command),
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
    let mut probed = std::collections::BTreeMap::new();
    for resource in out["resources"].as_array_mut().into_iter().flatten() {
        let file = resource["file"].as_str().unwrap_or_default();
        let timeline = p
            .root
            .join(resource["timeline"].as_str().unwrap_or_default());
        let path = timeline.parent().unwrap_or(&p.root).join(file);
        let info = probed
            .entry(path.clone())
            .or_insert_with(|| source_info(&path, file));
        if let Some(info) = info {
            resource["media"] = info.clone();
            lines.push(format!(
                "  {} ({}): {}",
                resource["alias"].as_str().unwrap_or_default(),
                resource["timeline"].as_str().unwrap_or_default(),
                media_summary(info)
            ));
        }
    }
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
        _ if path.ends_with(".motion.tsx") => {
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

/// Probe playable media at the native CLI boundary; project manifests stay portable.
/// An unavailable probe does not undo an import or prevent inspecting the rest of a project.
fn source_info(path: &Path, written: &str) -> Option<Value> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "svg") {
        if written.starts_with("https://") || written.starts_with("http://") {
            return Some(json!({"error": "remote media has not been probed"}));
        }
        let picture = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| valle_engine::image::Picture::load(&bytes, &mut Vec::new()));
        return Some(match picture {
            Ok(picture) => {
                let size = match picture.natural() {
                    valle_engine::layout::Natural::Size(size) => Some(size),
                    _ => None,
                };
                json!({"duration":null,"width":size.map(|s| s[0]),"height":size.map(|s| s[1]),"fps":null,"audio":false,"hdr":null})
            }
            Err(error) => json!({"error": error}),
        });
    }
    if !matches!(
        ext.as_str(),
        "mp4"
            | "mov"
            | "mkv"
            | "webm"
            | "avi"
            | "m4v"
            | "ts"
            | "mts"
            | "m2ts"
            | "wav"
            | "mp3"
            | "m4a"
            | "aac"
            | "flac"
            | "ogg"
            | "opus"
            | "aiff"
            | "aif"
    ) {
        return None;
    }
    if written.starts_with("https://") || written.starts_with("http://") {
        return Some(json!({"error": "remote media has not been probed"}));
    }
    Some(match media::probe(path) {
        Ok(info) => json!({
            "duration": info.duration.map(|t| t as f64 / media::TICKS_PER_SECOND as f64),
            "width": info.size.map(|s| s[0]),
            "height": info.size.map(|s| s[1]),
            "fps": info.fps.map(|[n, d]| if d == 1 { n.to_string() } else { format!("{n}/{d}") }),
            "audio": info.audio,
            "hdr": info.hdr,
        }),
        Err(error) => json!({"error": error}),
    })
}

fn media_summary(info: &Value) -> String {
    if let Some(error) = info["error"].as_str() {
        return format!("media info unavailable: {error}");
    }
    let mut parts = Vec::new();
    if let Some(duration) = info["duration"].as_f64() {
        parts.push(format!("{duration} s"));
    }
    if let (Some(w), Some(h)) = (info["width"].as_f64(), info["height"].as_f64()) {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(fps) = info["fps"].as_str() {
        parts.push(format!("{fps} fps"));
    }
    parts.push(
        if info["audio"] == true {
            "audio"
        } else {
            "no audio"
        }
        .into(),
    );
    parts.join(", ")
}

fn add(p: &Project, from: &[PathBuf], link: bool) -> Result<Value, Failure> {
    let mut out = p.add(from, link).map_err(failed)?;
    // `path` can name a component's folder; the binding names the actual scene or media file.
    for a in out["added"].as_array_mut().into_iter().flatten() {
        let alias = a["alias"].as_str().expect("an added file has an alias");
        let resource = &a["resource"][alias];
        let path = resource
            .as_str()
            .or_else(|| resource[0].as_str())
            .expect("an added resource binds one file");
        let clip = clip_of(path, alias);
        let timeline = p.root.join(&p.timeline);
        let info = source_info(&timeline.parent().unwrap_or(&p.root).join(path), path);
        a["clip"] = clip.into();
        if let Some(info) = info {
            a["media"] = info;
        }
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
            let info = a
                .get("media")
                .map(|m| format!("; {}", media_summary(m)))
                .unwrap_or_default();
            format!("{alias}: {path}{again}{info}; a clip shows it as {clip}")
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
    let mut warnings = Vec::new();
    if problems.is_empty() {
        let lowered = match cost {
            true => motion::check(&args).map(|out| {
                warnings.extend(out["warnings"].as_array().into_iter().flatten().cloned());
            }),
            false => timeline::check_audio(&args).and_then(|_| {
                let doc = p.document(&rel).map_err(failed)?;
                if doc.duration > valle_engine::time::Tick::ZERO {
                    timeline::lower(&args)?;
                }
                for source in p.dependencies(&rel, &doc) {
                    let parent = p.root.join(&source.timeline);
                    let path = parent.parent().unwrap_or(&p.root).join(&source.written);
                    if let Some(info) = source_info(&path, &source.written)
                        && let Some(transfer) = info["hdr"].as_str()
                    {
                        warnings.push(motion::hdr_warning(&source.written, transfer));
                    }
                }
                Ok(())
            }),
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
    for warning in &warnings {
        summary.push_str(&format!(
            "\nwarning: {}",
            warning["message"].as_str().unwrap_or_default()
        ));
    }
    Ok(
        json!({ "status": "ok", "summary": summary, "timeline": rel, "external": external, "warnings": warnings }),
    )
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

fn notes(p: &Project, command: Option<NotesCommand>) -> Result<Value, Failure> {
    let command = command.unwrap_or(NotesCommand::List {
        open: false,
        resolved: false,
        timeline: None,
    });
    let one = |note: Note, done: &str| -> Result<Value, Failure> {
        let mut out = placed(p, &note);
        let summary = format!("{done} {}\n{}", note.id, line(&note, &out));
        out["status"] = "ok".into();
        out["summary"] = summary.into();
        Ok(out)
    };
    match command {
        NotesCommand::List {
            open,
            resolved,
            timeline,
        } => {
            let timeline = match timeline {
                Some(name) if name.ends_with(".motion.tsx") => {
                    Some(Some(p.reviewed(&name).map_err(failed)?))
                }
                Some(name) => Some(p.relative(&p.timeline(Some(&name)).map_err(failed)?)),
                None => None,
            };
            let notes: Vec<Note> = (p.notes().map_err(failed)?.into_iter())
                .filter(|n| !open || !n.resolved())
                .filter(|n| !resolved || n.resolved())
                .filter(|n| {
                    timeline
                        .as_ref()
                        .is_none_or(|t| t.as_deref() == Some(&n.file))
                })
                .collect();
            let listed: Vec<Value> = notes.iter().map(|n| placed(p, n)).collect();
            let lines: Vec<String> = (notes.iter().zip(&listed))
                .map(|(n, v)| line(n, v))
                .collect();
            let summary = match lines.is_empty() {
                true => "no notes".to_string(),
                false => lines.join("\n"),
            };
            Ok(json!({ "status": "ok", "summary": summary, "notes": listed }))
        }
        NotesCommand::Show { id } => {
            let note = p.note(&id).map_err(failed)?;
            let mut out = placed(p, &note);
            let mut summary = line(&note, &out);
            if !note.targets.is_empty() {
                let about: Vec<String> = (note.targets.iter())
                    .map(|t| {
                        let mut s = t.clip.clone().unwrap_or_default();
                        if let Some(offset) = t.offset {
                            s += &format!("@{offset}");
                        }
                        if let Some(run) = t.run {
                            s += &format!(" run {run}");
                        }
                        if let Some(path) = &t.path {
                            s = format!("{path} {s}").trim_end().to_string();
                        }
                        s
                    })
                    .collect();
                summary += &format!("\n  about {}", about.join(", "));
            }
            for m in &note.thread {
                summary += &format!("\n  {} (snapshot {}): {}", m.by, m.snapshot, m.text);
            }
            out["status"] = "ok".into();
            out["summary"] = summary.into();
            Ok(out)
        }
        NotesCommand::Add {
            text,
            at,
            end,
            timeline,
            component,
            region,
            clips,
            elements,
            image,
            by,
        } => {
            let region = region.as_deref().map(parse_region).transpose()?;
            let mut targets: Vec<Target> = clips
                .iter()
                .map(|c| parse_target(c))
                .collect::<Result<_, _>>()?;
            targets.extend(elements.into_iter().map(|path| Target {
                clip: None,
                offset: None,
                run: None,
                path: Some(path),
                site: None,
            }));
            let image = match image {
                Some(path) => Some(
                    std::fs::read(&path).map_err(|e| failed(format!("{}: {e}", path.display())))?,
                ),
                None => None,
            };
            let note = p
                .add_note(NewNote {
                    file: (component.or(timeline)).unwrap_or_else(|| p.timeline.clone()),
                    start: at,
                    end,
                    region,
                    targets,
                    image,
                    by,
                    text,
                })
                .map_err(failed)?;
            one(note, "added")
        }
        NotesCommand::Reply { id, text, by } => {
            one(p.reply(&id, &by, &text).map_err(failed)?, "replied to")
        }
        NotesCommand::Resolve { id, text, by } => one(
            p.resolve(&id, &by, text.as_deref()).map_err(failed)?,
            "resolved",
        ),
        NotesCommand::Reopen { id, text, by } => one(
            p.reopen(&id, &by, text.as_deref()).map_err(failed)?,
            "reopened",
        ),
        NotesCommand::Delete { id } => {
            p.delete_note(&id).map_err(failed)?;
            Ok(json!({ "status": "ok", "summary": format!("deleted {id}"), "id": id }))
        }
    }
}

/// A note as `reviews/notes.json` has it, with where it is now in its timeline as `now`, or
/// `gone` where the clips it is about are, and its picture as a path in the project. A note on
/// a component is where it was written, while the component is there.
pub fn placed(p: &Project, note: &Note) -> Value {
    let mut out = note.to_json();
    if note.on_component() {
        match p.root.join(&note.file).is_file() {
            true => out["now"] = out["at"].clone(),
            false => out["gone"] = true.into(),
        }
    } else {
        placed_on_timeline(p, note, &mut out);
    }
    if let Some(image) = &note.image {
        out["picture"] = format!("reviews/{image}").into();
    }
    out
}

fn placed_on_timeline(p: &Project, note: &Note, out: &mut Value) {
    match p.document(&note.file) {
        Ok(doc) => match note.now(&doc) {
            Some((start, end)) => {
                let mut now = json!({ "start": round(start) });
                if let Some(end) = end {
                    now["end"] = json!(round(end));
                }
                out["now"] = now;
            }
            None => out["gone"] = true.into(),
        },
        Err(e) => out["problem"] = e.into(),
    }
}

/// A note Studio adds: JSON of `timeline` or `component`, `at` (`start` and `end`), `region`,
/// `targets` and `text`, by `user` unless `by` says, with the picture seen as `image`, if any.
pub fn new_note(given: &Value, image: &[u8]) -> Result<NewNote, String> {
    let fraction = |k: &str| {
        given["region"][k]
            .as_f64()
            .ok_or(format!("region.{k}: a fraction"))
    };
    let region = match given.get("region").filter(|r| !r.is_null()) {
        Some(_) => Some(Region {
            x: fraction("x")?,
            y: fraction("y")?,
            w: fraction("w")?,
            h: fraction("h")?,
        }),
        None => None,
    };
    let text = |v: &Value| v.as_str().map(str::to_string);
    let targets = (given["targets"].as_array().into_iter().flatten())
        .map(|t| {
            Ok(Target {
                clip: text(&t["clip"]),
                offset: t["offset"].as_f64(),
                run: t["run"].as_u64(),
                path: text(&t["path"]),
                site: text(&t["site"]),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(NewNote {
        file: (text(&given["timeline"]).or_else(|| text(&given["component"])))
            .ok_or("timeline or component: what the note is on")?,
        start: given["at"]["start"].as_f64().ok_or("at.start: seconds")?,
        end: given["at"]["end"].as_f64(),
        region,
        targets,
        image: (!image.is_empty()).then(|| image.to_vec()),
        by: text(&given["by"]).unwrap_or_else(|| "user".into()),
        text: text(&given["text"]).ok_or("text: what to change")?,
    })
}

/// A note on a line: its id, state, where it is now, and what it says first.
fn line(note: &Note, placed: &Value) -> String {
    let at = |v: &Value| match (v["start"].as_f64(), v["end"].as_f64()) {
        (Some(start), Some(end)) => format!("{start}–{end} s"),
        (Some(start), None) => format!("{start} s"),
        _ => String::new(),
    };
    let place = match (&placed["now"], &placed["problem"]) {
        (now, _) if now.is_object() => at(now),
        (_, Value::String(e)) => e.clone(),
        _ => format!("gone, was at {}", at(&placed["at"])),
    };
    let state = match note.resolved_in {
        Some(n) => format!("resolved in snapshot {n}"),
        None => "open".to_string(),
    };
    let replies = match note.thread.len() - 1 {
        0 => String::new(),
        1 => " (1 reply)".to_string(),
        n => format!(" ({n} replies)"),
    };
    let first = note.thread.first().map_or("", |m| m.text.as_str());
    format!(
        "{} {state}, {}, {place}: {first}{replies}",
        note.id, note.file
    )
}

fn round(seconds: f64) -> f64 {
    (seconds * 1e6).round() / 1e6
}

/// `x,y,w,h`, fractions of the canvas.
fn parse_region(text: &str) -> Result<Region, Failure> {
    let bad = || {
        failed(format!(
            "--region {text}: x,y,w,h as fractions of the canvas"
        ))
    };
    let v: Vec<f64> = (text.split(','))
        .map(|n| n.trim().parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    match v[..] {
        [x, y, w, h] => Ok(Region { x, y, w, h }),
        _ => Err(bad()),
    }
}

/// `id`, or `id@seconds` into the clip.
fn parse_target(text: &str) -> Result<Target, Failure> {
    let (clip, offset) = match text.rsplit_once('@') {
        Some((clip, s)) => {
            let s = (s.parse::<f64>()).map_err(|_| failed(format!("--clip {text}: id@seconds")))?;
            (clip, Some(s))
        }
        None => (text, None),
    };
    Ok(Target {
        clip: Some(clip.to_string()),
        offset,
        run: None,
        path: None,
        site: None,
    })
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
    args.validate_output()?;
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
        args.out = Some(
            free(&directory.join(name), args.storyboard)
                .display()
                .to_string(),
        );
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
fn free(path: &Path, storyboard: bool) -> PathBuf {
    // A paginated storyboard has no file at its base name. Its first page reserves that name.
    let available =
        |p: &Path| !p.exists() && (!storyboard || !crate::picture::page_path(p, 0, 2).exists());
    if available(path) {
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
        .find(|p| available(p))
        .expect("a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[test]
    fn copied_and_linked_components_describe_the_bound_scene() {
        let dir = std::env::temp_dir().join(format!("valle-add-components-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("title");
        std::fs::create_dir_all(&folder).unwrap();
        let source = indoc! {"
            export const composition = { width: 32, height: 32, fps: 30, duration: 1 };
            export default function S() { return <View style={{ flex: 1 }} />; }
        "};
        std::fs::write(folder.join("title.motion.tsx"), source).unwrap();
        let single = dir.join("single.motion.tsx");
        std::fs::write(&single, source).unwrap();
        for link in [false, true] {
            let root = dir.join(if link { "linked" } else { "copied" });
            let p = Project::create(&root, 32, 32, "30").unwrap();
            let out = add(&p, &[folder.clone(), single.clone()], link)
                .unwrap_or_else(|e| panic!("{}", e.message));
            assert_eq!(out["added"].as_array().unwrap().len(), 2);
            for a in out["added"].as_array().unwrap() {
                let alias = a["alias"].as_str().unwrap();
                let hint = a["clip"].as_str().unwrap();
                let clip: Value = serde_json::from_str(hint).unwrap();
                assert_eq!(clip, json!({ "kind": "motion", "component": alias }), "{a}");
                assert!(
                    a["resource"][alias]
                        .as_str()
                        .unwrap()
                        .ends_with(".motion.tsx")
                );
                assert!(out["summary"].as_str().unwrap().contains(hint));
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_files_inside_components_keep_their_clip_kind() {
        let dir = std::env::temp_dir().join(format!("valle-add-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = Project::create(&dir, 32, 32, "30").unwrap();
        let timeline = p.root.join(&p.timeline);
        let mut doc: Value = serde_json::from_slice(&std::fs::read(&timeline).unwrap()).unwrap();
        // Reusing a resource also permits its existing single-file array binding.
        doc["resources"] = json!({ "art": ["components/logo.svg"] });
        std::fs::write(&timeline, doc.to_string()).unwrap();
        for (file, kind) in [
            ("logo.svg", "image"),
            ("nested.timeline.json", "timeline"),
            ("shot.mp4", "video"),
        ] {
            // Adding binds files without decoding them; this test checks the usage hint.
            let path = dir.join("components").join(file);
            std::fs::write(&path, []).unwrap();
            let out = add(&p, &[path], false).unwrap_or_else(|e| panic!("{}", e.message));
            let a = &out["added"][0];
            assert_eq!(a["reused"], true);
            let alias = a["alias"].as_str().unwrap();
            if file == "logo.svg" {
                assert_eq!(alias, "art");
                assert_eq!(a["resource"][alias], json!(["components/logo.svg"]));
            }
            let hint = a["clip"].as_str().unwrap();
            let clip: Value = serde_json::from_str(hint).unwrap();
            assert_eq!(clip, json!({ "kind": kind, "src": alias }), "{a}");
            assert!(out["summary"].as_str().unwrap().contains(hint));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
