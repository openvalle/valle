//! `valle`: the command line. Each command is a function that gives JSON; this file only reads the
//! arguments and prints the result, as text or, with `--json`, as the JSON itself.

mod args;
mod motion;
mod picture;
mod timeline;
mod video;

use std::process::ExitCode;

use serde_json::{Value, json};

use args::Args;

/// Scene code runs on the thread that loads it, which needs more than the 12 MiB stack QuickJS
/// is given.
const STACK: usize = 64 << 20;

const USAGE: &str = "\
usage: valle <command> [options]

  motion check <scene.tsx>             compile the scene and check every frame without drawing
  motion render <scene.tsx> -o <out>   draw frames: <out> is a .png (one frame), a folder (a
                                       sequence), or a .mp4 or .mov video
  timeline check <edit.timeline.json>  check the document, its clips' sources and every frame
  timeline render <edit.timeline.json> -o <out>
                                       draw the timeline's frames, as motion render does, its
                                       sound mixed into a video; a .wav <out> is its sound alone

options of motion:
  --props <json|@file>        values of the props controls.props declares
  --data <json|@file>         values of the data controls.data declares
  --resource <name>=<path>    a file bound to a resource; a font's faces, once each
  --input <name>=<path>       a picture a role reads as input://<name>
  --caption <json|@file>      the caption a captionPresenter() shows
  --json                      print the result as JSON

options of render (motion and timeline):
  --frame <n>                 the frame a .png shows (0)
  --frames <a,b,…|a..b>       the frames to draw (all)
  --storyboard                draw the frames side by side into the one .png
  --codec <h264|hevc|prores>  a video's codec: h264 for a .mp4, prores (4444, with alpha) for
                              a .mov
  --encoder <vt|ffmpeg>       the hardware encoder (VideoToolbox), or FFmpeg's own on the CPU
                              (x264, x265, prores_ks); the hardware's where it has the codec
  --decoder <vt|ffmpeg>       how videos the scene shows are decoded: the hardware's (where it
                              opens the file) or FFmpeg's own
  --bitrate <bits>            a video's bits a second, such as 8M, in place of its quality
";

/// A command's failure: a code tools can match, what went wrong, and what to do about it.
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
    /// Each problem a check found: its message, the first frame it is in, and in how many.
    pub problems: Vec<Value>,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
            hint: None,
            problems: Vec::new(),
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Failure {
        self.hint = Some(hint.into());
        self
    }
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(&words));
    let (result, json) = match thread.map(|t| t.join()) {
        Ok(Ok(done)) => done,
        _ => (Err(Failure::new("internal", "the command stopped")), false),
    };
    match (result, json) {
        (Ok(value), true) => println!("{value}"),
        (Ok(value), false) => print_text(&value),
        (Err(f), true) => {
            let mut out = json!({ "status": "error", "code": f.code, "message": f.message });
            if let Some(hint) = f.hint {
                out["hint"] = hint.into();
            }
            if !f.problems.is_empty() {
                out["problems"] = f.problems.into();
            }
            println!("{out}");
            return ExitCode::FAILURE;
        }
        (Err(f), false) => {
            for p in &f.problems {
                let (first, count) = (&p["first"], p["count"].as_u64().unwrap_or(1));
                let more = match count {
                    1 => String::new(),
                    n => format!(" and {} more", n - 1),
                };
                eprintln!(
                    "frame {first}{more}: {}",
                    p["message"].as_str().unwrap_or("")
                );
            }
            eprintln!("error: {}", f.message);
            if let Some(hint) = f.hint {
                eprintln!("hint: {hint}");
            }
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn run(words: &[String]) -> (Result<Value, Failure>, bool) {
    let json = words.iter().any(|w| w == "--json");
    let result = (|| {
        let usage = || Failure::new("usage", "no such command").hint(USAGE.trim_end());
        let (group, command, rest) = match words {
            [group, command, rest @ ..] if group == "motion" || group == "timeline" => {
                (group.as_str(), command.as_str(), rest)
            }
            [help] if help == "--help" || help == "help" => {
                return Ok(json!({ "status": "ok", "usage": USAGE }));
            }
            _ => return Err(usage()),
        };
        let args = Args::parse(rest)?;
        let document = args.file.ends_with(".timeline.json");
        if group == "motion" && document {
            return Err(Failure::new("usage", "a .timeline.json is a timeline")
                .hint(format!("valle timeline {command} {}", args.file)));
        }
        if group == "timeline" && !document {
            return Err(Failure::new(
                "usage",
                "valle timeline takes a .timeline.json",
            ));
        }
        match command {
            "check" => motion::check(&args),
            "render" => motion::render(&args),
            _ => Err(usage()),
        }
    })();
    (result, json)
}

/// A result as text: the usage as it is, else its summary.
fn print_text(value: &Value) {
    if let Some(usage) = value["usage"].as_str() {
        print!("{usage}");
        return;
    }
    if let Some(summary) = value["summary"].as_str() {
        println!("{summary}");
    }
}
