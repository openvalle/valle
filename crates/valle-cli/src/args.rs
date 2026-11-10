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
const VALUED: [&str; 12] = [
    "--output-size",
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

    /// Output pixels, leaving the scene's logical canvas unchanged.
    pub fn output_size(&self, canvas: [u32; 2]) -> Result<[u32; 2], Failure> {
        let size = match self.option("output-size") {
            None => canvas,
            Some(text) => {
                let parsed = text
                    .split_once('x')
                    .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]));
                parsed.ok_or_else(|| {
                    Failure::new("usage", "--output-size is WIDTHxHEIGHT, such as 3840x2160")
                })?
            }
        };
        valle_engine::gpu::output_size(size).map_err(|e| Failure::new("usage", e))?;
        Ok(size)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Args, Failure> {
        Args::parse(&words.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn bad(words: &[&str], code: &str, message: &str) {
        let Err(e) = parse(words) else {
            panic!("accepted {words:?}")
        };
        assert_eq!(e.code, code);
        assert!(e.message.contains(message), "{words:?}: {}", e.message);
    }

    #[test]
    fn missing_values_unknown_options_and_extra_files_are_usage_errors() {
        for flag in VALUED.into_iter().chain([
            "-o",
            "--out",
            "--props",
            "--data",
            "--caption",
            "--resource",
            "--input",
            "--frame",
            "--frames",
        ]) {
            bad(&["scene.tsx", flag], "usage", "needs a value");
        }
        bad(&[], "usage", "which file");
        bad(&["scene.tsx", "--unknown"], "usage", "no option");
        bad(&["scene.tsx", "another.tsx"], "usage", "one file");
    }

    #[test]
    fn frame_lists_keep_order_and_ranges_include_both_ends() {
        let a = parse(&["scene.tsx", "--json", "--frames", "3,0,3", "--events"])
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(matches!(a.frames, Some(Frames::List(v)) if v == [3, 0, 3]));
        let a =
            parse(&["scene.tsx", "--frames", "0..0"]).unwrap_or_else(|e| panic!("{}", e.message));
        assert!(matches!(a.frames, Some(Frames::Range(0, 0))));
        for s in [
            "",
            "-1",
            "1.5",
            "1,",
            "0..",
            "1..2..3",
            "9223372036854775808",
        ] {
            bad(
                &["scene.tsx", "--frames", s],
                "usage",
                "a frame is a number",
            );
        }
        bad(&["scene.tsx", "--frames", "5..2"], "usage", "end before");
    }

    #[test]
    fn bindings_keep_resource_lists_and_separate_props_from_data() {
        let a = parse(&[
            "scene.tsx",
            "--props",
            r#"{"title":"Hi"}"#,
            "--data",
            r#"{"values":[1,2]}"#,
            "--resource",
            "photo=a.png",
            "--resource",
            "photo=b=x.png",
            "--input",
            "plate=c.png",
            "--caption",
            r#"{"runs":[{"text":"Hello"}]}"#,
        ])
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(a.inputs.resources["photo"], ["a.png", "b=x.png"]);
        assert_eq!(a.inputs.pictures["plate"], "c.png");
        assert_eq!(a.inputs.props["title"], "Hi");
        assert_eq!(a.inputs.data["values"], serde_json::json!([1, 2]));
        assert_eq!(a.inputs.caption.unwrap()["runs"][0]["text"], "Hello");
        for flag in ["--props", "--data"] {
            bad(&["scene.tsx", flag, "[]"], "usage", "JSON object");
            bad(&["scene.tsx", flag, "{"], "usage", flag);
        }
        for flag in ["--resource", "--input"] {
            for s in ["photo", "=a.png", "photo="] {
                bad(&["scene.tsx", flag, s], "usage", "<name>=<path>");
            }
        }
    }

    #[test]
    fn json_files_and_output_size_errors_keep_their_context() {
        let path = std::env::temp_dir().join(format!("valle-args-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"name":"Ada"}"#).unwrap();
        let file = format!("@{}", path.display());
        assert_eq!(
            json(&file, "--props").unwrap_or_else(|e| panic!("{}", e.message))["name"],
            "Ada"
        );
        std::fs::remove_file(path).unwrap();
        let Err(e) = json(&file, "--props") else {
            panic!("missing file accepted")
        };
        assert_eq!(e.code, "not_found");
        assert!(e.message.contains("--props"));
        for value in ["0x100", "-1x100", "3840", "3840x2160x2", "9000x9000"] {
            let a = parse(&["scene.tsx", "--output-size", value])
                .unwrap_or_else(|e| panic!("{}", e.message));
            let Err(e) = a.output_size([1920, 1080]) else {
                panic!("accepted {value}")
            };
            assert_eq!(e.code, "usage");
        }
        let a = parse(&["scene.tsx", "--output-size", "3840x2160"])
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            a.output_size([1920, 1080])
                .unwrap_or_else(|e| panic!("{}", e.message)),
            [3840, 2160]
        );
    }
}
