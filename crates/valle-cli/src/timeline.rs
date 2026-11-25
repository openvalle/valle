//! `valle timeline check` and `valle timeline render`: a document lowered to the scene it plays
//! as, which the Motion commands then check and draw; `valle timeline query`; `valle timeline
//! edit`; and `valle timeline captions`, SRT and VTT in and out.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use valle_engine::audio::Mixer;
use valle_engine::mix;
use valle_engine::time::Tick;
use valle_engine::timeline::{self, Host, Lowered};
use valle_native::media;
use valle_timeline::query::Filter;
use valle_timeline::subtitles;

use crate::Failure;
use crate::args::Args;

/// The document at `args.file`: its path, text, and what it parses to.
fn document(
    args: &Args,
    allow_empty: bool,
) -> Result<(String, String, valle_timeline::Document), Failure> {
    let path = args.file.replace('\\', "/");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let root = serde_json::from_str(&text)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let doc = match allow_empty {
        true => valle_timeline::Document::read_edit(&root),
        false => valle_timeline::Document::read(&root),
    }
    .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    Ok((path, text, doc))
}

/// The document edited by `op`, JSON or `@file`, as Studio edits it, and written back whole or
/// not at all.
pub fn edit(file: &str, op: &str) -> Result<Value, Failure> {
    let path = file.replace('\\', "/");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let op = crate::args::json(op, "--op")?;
    let edited = valle_timeline::edit::report(&text, &op)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let root =
        serde_json::from_str(&edited.text).map_err(|e| Failure::new("timeline", e.to_string()))?;
    let doc =
        valle_timeline::Document::read_edit(&root).map_err(|e| Failure::new("timeline", e))?;
    if doc.duration > Tick::ZERO {
        lower_document(&path, &doc)?;
    }
    valle_project::write(Path::new(&path), edited.text.as_bytes(), true)
        .map_err(|e| Failure::new("timeline", e))?;
    let summary = format!("edited {path}\n{}", edited.summary());
    Ok(json!({ "status": "ok", "summary": summary, "file": path, "changes": edited.changes }))
}

/// The cues of `subtitles` as caption clips in the document, written back only if it is valid.
pub fn import_captions(
    file: &str,
    subtitles: &Path,
    track: Option<&str>,
    clip: Option<&str>,
) -> Result<Value, Failure> {
    let path = file.replace('\\', "/");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let source = subtitles.display().to_string();
    let bytes =
        std::fs::read(subtitles).map_err(|e| Failure::new("usage", format!("{source}: {e}")))?;
    let cues = String::from_utf8(bytes)
        .map_err(|_| "not UTF-8 text".to_string())
        .and_then(|t| subtitles::read(&t))
        .map_err(|e| {
            Failure::new("usage", format!("{source}: {e}")).hint("an .srt or .vtt file, in UTF-8")
        })?;
    let (edited, done) = subtitles::import(&text, &cues, track, clip)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let root =
        serde_json::from_str(&edited).map_err(|e| Failure::new("timeline", e.to_string()))?;
    let doc =
        valle_timeline::Document::read_edit(&root).map_err(|e| Failure::new("timeline", e))?;
    if doc.duration > Tick::ZERO {
        lower_document(&path, &doc)?;
    }
    valle_project::write(Path::new(&path), edited.as_bytes(), true)
        .map_err(|e| Failure::new("timeline", e))?;
    let mut summary = format!(
        "{} captions from {source} on {} of {path}",
        done.captions, done.track
    );
    if done.shortened > 0 {
        summary += &format!("\n{} shortened to meet the next", done.shortened);
    }
    if done.left_out > 0 {
        summary += &format!(
            "\n{} left out: under a frame, or outside the clip",
            done.left_out
        );
    }
    Ok(json!({
        "status": "ok",
        "summary": summary,
        "file": path,
        "track": done.track,
        "captions": done.captions,
        "shortened": done.shortened,
        "leftOut": done.left_out,
    }))
}

/// A caption track of the document written out as an .srt or .vtt file.
pub fn export_captions(file: &str, output: &Path, track: Option<&str>) -> Result<Value, Failure> {
    let out = output.display().to_string();
    let format = subtitles::Format::of(&out).ok_or_else(|| {
        Failure::new("usage", format!("{out}: not an .srt or .vtt file"))
            .hint("name the file .srt or .vtt")
    })?;
    let args = Args {
        file: file.to_string(),
        ..Args::default()
    };
    let (path, _, doc) = document(&args, true)?;
    let (at, cues) = subtitles::export(&doc, track)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    valle_project::write(output, subtitles::write(&cues, format).as_bytes(), true)
        .map_err(|e| Failure::new("usage", format!("{out}: {e}")))?;
    Ok(json!({
        "status": "ok",
        "summary": format!("{} captions from {at} of {path} in {out}", cues.len()),
        "file": out,
        "track": at,
        "captions": cues.len(),
    }))
}

/// What the frame at `at` seconds draws at `point`, or in `region`, fractions of the canvas.
pub fn pick(
    args: &Args,
    at: f64,
    point: Option<&str>,
    region: Option<&str>,
) -> Result<Value, Failure> {
    let fractions = |text: &str, n: usize, what: &str| -> Result<Vec<f32>, Failure> {
        let parts: Vec<f32> = (text.split(','))
            .map(|p| p.trim().parse::<f32>())
            .collect::<Result<_, _>>()
            .map_err(|_| Failure::new("usage", format!("{what}: numbers, as {text:?} is not")))?;
        if parts.len() != n || parts.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return Err(Failure::new(
                "usage",
                format!("{what}: {n} fractions of the canvas, each from 0 to 1"),
            ));
        }
        Ok(parts)
    };
    let mut scene = crate::motion::load(args)?.scene;
    let c = scene.composition();
    let tick = Tick::from_seconds(at).map_err(|e| Failure::new("usage", format!("--at: {e}")))?;
    if tick >= c.duration {
        return Err(Failure::new(
            "usage",
            format!("--at: the timeline is {} s long", c.duration.seconds()),
        ));
    }
    let frame = c.fps.frame_at(tick);
    let shown = c
        .fps
        .frame_start(frame)
        .map_err(|e| Failure::new("scene", e))?;
    let drawn = scene.frame(shown).map_err(|e| Failure::new("scene", e))?;
    let [w, h] = [c.width as f32, c.height as f32];
    let hits = match (point, region) {
        (Some(p), _) => {
            let p = fractions(p, 2, "--point")?;
            valle_engine::pick::at(&drawn, [p[0] * w, p[1] * h])
        }
        (None, Some(r)) => {
            let r = fractions(r, 4, "--region")?;
            let area = valle_engine::layout::Rect {
                x: r[0] * w,
                y: r[1] * h,
                width: r[2] * w,
                height: r[3] * h,
            };
            valle_engine::pick::within(&drawn, area)
        }
        (None, None) => return Err(Failure::new("usage", "--point or --region")),
    };
    let mut named = valle_engine::pick::named(&hits);
    // Lines in the files beside the timeline, as its folder names them.
    let path = args.file.replace('\\', "/");
    let dir = path.rfind('/').map(|end| &path[..=end]);
    for v in &mut named {
        let (Some(dir), Some(site)) = (dir, v["site"].as_str()) else {
            continue;
        };
        if let Some(near) = site.strip_prefix(dir) {
            v["site"] = json!(near);
        }
    }
    let lines: Vec<String> = (named.iter())
        .map(|v| {
            let mut s = v["clip"].as_str().unwrap_or("(no clip)").to_string();
            if let Some(path) = v["path"].as_str() {
                s += &format!(" {path}");
            }
            if let Some(site) = v["site"].as_str() {
                s += &format!(" at {site}");
            }
            s
        })
        .collect();
    let summary = match lines.is_empty() {
        true => format!("nothing drawn there at frame {frame}"),
        false => format!("at frame {frame}, top first:\n  {}", lines.join("\n  ")),
    };
    Ok(json!({ "status": "ok", "summary": summary, "frame": frame, "hits": named }))
}

/// The clips `filter` picks.
pub fn query(args: &Args, filter: &Filter) -> Result<Value, Failure> {
    let (path, text, doc) = document(args, true)?;
    let found = valle_timeline::query::query(&text, &doc, filter)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let lines: Vec<String> = (found["clips"].as_array().into_iter().flatten())
        .map(|c| {
            let line = c["line"]
                .as_u64()
                .map(|l| format!(", line {l}"))
                .unwrap_or_default();
            // The path edits and --clip take, so a clip found can be named back.
            format!(
                "{} at {}, {}–{}{line}",
                c["id"].as_str().unwrap_or_default(),
                c["path"].as_str().unwrap_or_default(),
                c["program"][0].as_str().unwrap_or_default(),
                c["program"][1].as_str().unwrap_or_default(),
            )
        })
        .collect();
    let summary = match lines.len() {
        0 => "no clips".to_string(),
        1 => format!("1 clip: {}", lines[0]),
        n => format!("{n} clips:\n  {}", lines.join("\n  ")),
    };
    let mut out = json!({ "status": "ok", "summary": summary });
    out["count"] = found["count"].clone();
    out["clips"] = found["clips"].clone();
    Ok(out)
}

/// The mixer of the document's sound, its files decoded.
pub fn mixer(args: &Args) -> Result<Mixer, Failure> {
    let (path, _, doc) = document(args, false)?;
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let sound = |p: &str| media::sound(Path::new(p));
    let host = mix::Host {
        read: &read,
        sound: &sound,
    };
    mix::mixer(&path, &doc, &host).map_err(|e| Failure::new("sound", format!("{path}: {e}")))
}

/// Check stream presence once per file; checking a long edit does not decode its soundtrack.
pub fn check_audio(args: &Args) -> Result<(), Failure> {
    let (path, _, doc) = document(args, true)?;
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let streams = RefCell::new(BTreeMap::new());
    let audio = |p: &str| {
        if let Some(&has) = streams.borrow().get(p) {
            return Ok(has);
        }
        if !p.starts_with("https://") && !p.starts_with("http://") && !Path::new(p).is_file() {
            return Err(format!("{p}: not found"));
        }
        let has = media::probe(Path::new(p))?.audio;
        streams.borrow_mut().insert(p.to_string(), has);
        Ok(has)
    };
    mix::check(&path, &doc, &read, &audio).map_err(|e| Failure::new("sound", e))
}

pub fn lower(args: &Args) -> Result<Lowered, Failure> {
    let (path, _, doc) = document(args, false)?;
    lower_document(&path, &doc)
}

/// Probe source bounds before accepting an edit, using the same rules as check and render.
fn lower_document(path: &str, doc: &valle_timeline::Document) -> Result<Lowered, Failure> {
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let length = |p: &str| media::length(Path::new(p)).ok().map(Tick);
    let host = Host {
        read: &read,
        length: &length,
    };
    timeline::lower(path, doc, &host).map_err(|e| Failure::new("timeline", format!("{path}: {e}")))
}
