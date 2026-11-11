//! `valle timeline check` and `valle timeline render`: a document lowered to the scene it plays
//! as, which the Motion commands then check and draw; `valle timeline query`; and `valle
//! timeline edit`.

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
