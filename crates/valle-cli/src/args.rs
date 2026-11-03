//! A command's arguments: its file, its output, and the values bound to the scene.

use serde_json::{Map, Value};
use valle_engine::front::Inputs;

use crate::Failure;

#[derive(Default)]
pub struct Args {
    /// The scene, or the document, the command works on.
    pub file: String,
    /// Whether `file` is a timeline, not a scene.
    pub timeline: bool,
    pub out: Option<String>,
    pub inputs: Inputs,
    /// The frames to draw, if not all.
    pub frames: Option<Frames>,
    pub storyboard: bool,
    /// Options a command reads itself, by name: `--encoder vt`.
    pub options: Vec<(String, String)>,
}

pub enum Frames {
    List(Vec<i64>),
    /// From the first to the last, both drawn.
    Range(i64, i64),
}

/// Options that take a value, beyond those read here.
const VALUED: [&str; 11] = [
    "--encoder",
    "--decoder",
    "--codec",
    "--bitrate",
    "--clip",
    "--track",
    "--index",
    "--at",
    "--marker",
    "--meta",
    "--op",
];

impl Args {
    pub fn parse(words: &[String]) -> Result<Args, Failure> {
        let mut args = Args::default();
        let mut words = words.iter();
        let bad = |m: String| Failure::new("usage", m).hint("valle --help lists the options");
        while let Some(w) = words.next() {
            let mut value =
                || (words.next().cloned()).ok_or_else(|| bad(format!("{w} needs a value")));
            match w.as_str() {
                "--json" | "--events" => {}
                "--storyboard" => args.storyboard = true,
                "-o" | "--out" => args.out = Some(value()?),
                "--props" => args.inputs.props = object(&value()?, "--props")?,
                "--data" => args.inputs.data = object(&value()?, "--data")?,
                "--caption" => args.inputs.caption = Some(json(&value()?, "--caption")?),
                "--resource" => {
                    let (name, path) = pair(&value()?, "--resource")?;
                    args.inputs.resources.entry(name).or_default().push(path);
                }
                "--input" => {
                    let (name, path) = pair(&value()?, "--input")?;
                    args.inputs.pictures.insert(name, path);
                }
                "--frame" => args.frames = Some(Frames::List(vec![number(&value()?)?])),
                "--frames" => args.frames = Some(frames(&value()?)?),
                o if VALUED.contains(&o) => args.options.push((o[2..].to_string(), value()?)),
                o if o.starts_with('-') => return Err(bad(format!("no option {o}"))),
                file if args.file.is_empty() => args.file = file.to_string(),
                extra => return Err(bad(format!("one file, not also {extra}"))),
            }
        }
        if args.file.is_empty() {
            return Err(bad("which file?".into()));
        }
        Ok(args)
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        (self.options.iter()).find_map(|(k, v)| (k == name).then_some(v.as_str()))
    }
}

/// JSON written out, or `@path` for a file of it.
pub fn json(text: &str, what: &str) -> Result<Value, Failure> {
    let text = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| Failure::new("not_found", format!("{what} {path}: {e}")))?,
        None => text.to_string(),
    };
    serde_json::from_str(&text).map_err(|e| Failure::new("usage", format!("{what}: {e}")))
}

fn object(text: &str, what: &str) -> Result<Map<String, Value>, Failure> {
    match json(text, what)? {
        Value::Object(o) => Ok(o),
        _ => Err(Failure::new(
            "usage",
            format!("{what} is a JSON object of values by name"),
        )),
    }
}

fn pair(text: &str, what: &str) -> Result<(String, String), Failure> {
    match text.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => {
            Ok((name.to_string(), path.to_string()))
        }
        _ => Err(Failure::new(
            "usage",
            format!("{what} is <name>=<path>, not {text}"),
        )),
    }
}

fn number(text: &str) -> Result<i64, Failure> {
    (text.parse().ok().filter(|n| *n >= 0))
        .ok_or_else(|| Failure::new("usage", format!("a frame is a number from 0, not {text}")))
}

fn frames(text: &str) -> Result<Frames, Failure> {
    if let Some((a, b)) = text.split_once("..") {
        let (a, b) = (number(a)?, number(b)?);
        if b < a {
            return Err(Failure::new(
                "usage",
                format!("frames {a}..{b} end before they start"),
            ));
        }
        return Ok(Frames::Range(a, b));
    }
    let list = text.split(',').map(number).collect::<Result<Vec<_>, _>>()?;
    Ok(Frames::List(list))
}
