//! `valle timeline check` and `valle timeline render`: a document lowered to the scene it plays
//! as, which the Motion commands then check and draw; `valle timeline query`; and `valle
//! timeline edit`.

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
fn document(args: &Args) -> Result<(String, String, valle_timeline::Document), Failure> {
    let path = args.file.replace('\\', "/");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let doc = valle_timeline::parse(&text)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    Ok((path, text, doc))
}

/// The document edited by `--op`, as Studio edits it, and written back whole or not at all.
pub fn edit(args: &Args) -> Result<Value, Failure> {
    let (path, text, _) = document(args)?;
    let op = args
        .option("op")
        .ok_or_else(|| Failure::new("usage", "edit needs --op <json|@file>"))?;
    let op = crate::args::json(op, "--op")?;
    let edited = valle_timeline::edit::apply(&text, &op)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    valle_project::write(Path::new(&path), edited.as_bytes(), true)
        .map_err(|e| Failure::new("timeline", e))?;
    Ok(json!({ "status": "ok", "summary": format!("edited {path}"), "file": path }))
}

/// The clips the options pick: `--clip`, `--track`, `--index`, `--at`, `--marker`, `--meta`.
pub fn query(args: &Args) -> Result<Value, Failure> {
    let (path, text, doc) = document(args)?;
    let mut filter = Filter::default();
    for (k, v) in &args.options {
        let v = v.clone();
        match k.as_str() {
            "clip" => filter.clip = Some(v),
            "track" => filter.track = Some(v),
            "index" => {
                filter.index = Some(v.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    Failure::new("usage", format!("--index is a place from 1, not {v}"))
                })?)
            }
            "at" => filter.at = Some(v),
            "marker" => filter.marker = Some(v),
            "meta" => match v.split_once('=') {
                Some((key, value)) if !key.is_empty() => {
                    filter.meta.push((key.to_string(), value.to_string()))
                }
                _ => {
                    return Err(Failure::new(
                        "usage",
                        format!("--meta is key=value, not {v}"),
                    ));
                }
            },
            o => return Err(Failure::new("usage", format!("query takes no --{o}"))),
        }
    }
    let found = valle_timeline::query::query(&text, &doc, &filter)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let lines: Vec<String> = (found["clips"].as_array().into_iter().flatten())
        .map(|c| {
            let line = c["line"]
                .as_u64()
                .map(|l| format!(", line {l}"))
                .unwrap_or_default();
            format!(
                "{} ({} #{}, {}-{}{line})",
                c["id"].as_str().unwrap_or_default(),
                c["track"].as_str().unwrap_or_default(),
                c["index"],
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
    let (path, _, doc) = document(args)?;
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let sound = |p: &str| media::sound(Path::new(p));
    let host = mix::Host {
        read: &read,
        sound: &sound,
    };
    mix::mixer(&path, &doc, &host).map_err(|e| Failure::new("sound", format!("{path}: {e}")))
}

pub fn lower(args: &Args) -> Result<Lowered, Failure> {
    let i = &args.inputs;
    if !(i.props.is_empty() && i.data.is_empty() && i.resources.is_empty())
        || !i.pictures.is_empty()
        || i.caption.is_some()
    {
        return Err(Failure::new(
            "usage",
            "a timeline binds its components' props, data and resources itself",
        )
        .hint("--props, --data, --resource, --input and --caption are for valle motion"));
    }
    let (path, _, doc) = document(args)?;
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let length = |p: &str| media::length(Path::new(p)).ok().map(Tick);
    let host = Host {
        read: &read,
        length: &length,
    };
    timeline::lower(&path, &doc, &host)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))
}
