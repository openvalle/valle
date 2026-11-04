//! `valle`: the command line. Each command is a function that gives JSON; this file only reads the
//! arguments and prints the result, as text or, with `--json`, as the JSON itself.

mod args;
mod http;
mod motion;
mod picture;
mod project;
mod studio;
mod timeline;
mod video;

use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use args::Args;

/// Scene code runs on the thread that loads it, which needs more than the 12 MiB stack QuickJS
/// is given.
const STACK: usize = 64 << 20;

/// Whether `--events` was given: what the command does is printed as it goes, a line of JSON
/// each, before its result.
static EVENTS: AtomicBool = AtomicBool::new(false);

/// Says, with `--events`, that `done` of `total` frames are drawn.
pub fn progress(done: usize, total: usize) {
    if EVENTS.load(Ordering::Relaxed) {
        let line = json!({ "event": "progress", "done": done, "frames": total });
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

const USAGE: &str = "\
usage: valle <command> [options]

  motion check <scene.tsx>             compile the scene and check every frame without drawing
  motion render <scene.tsx> -o <out>   draw frames: <out> is a .png (one frame), a folder (a
                                       sequence), or a .mp4 or .mov video
  timeline check <edit.timeline.json>  check the document, its clips' sources and every frame
  timeline render <edit.timeline.json> -o <out>
                                       draw the timeline's frames, as motion render does, its
                                       sound mixed into a video; a .wav <out> is its sound alone
  timeline query <edit.timeline.json> [filters]
                                       the clips the filters pick, with their program and source
                                       timecodes, lines, anchors and the clips anchored to them
  timeline edit <edit.timeline.json> --op <json|@file>
                                       edit it as Studio does: set, move, trim, delete or add a
                                       clip ({ \"op\": \"move\", \"clip\": \"intro\", \"start\": 2 })

  project create <folder> [--width W --height H --fps F]
                                       a new project: project.json, timeline.json and its
                                       folders (1920×1080 at 30 fps unless given)
  project show                         its timelines, resources, files out of the project or
                                       missing, snapshots and last render
  project add <file or folder>… [--link]
                                       copy files in (components/, else media/), or link them
                                       where they are; gives each an alias and resource entry
  project check [timeline] [--cost]    check a timeline, its files and components' folders;
                                       --cost also plans every frame
  project save [-m <message>]          save a snapshot of the sources
  project history                      the snapshots
  project restore <n>                  put snapshot n's sources back, saving what they replace
  project render [timeline] [-o <out>] render, by default a video into renders/; a full render
                                       saves a snapshot and records what it read and made

  motion studio <scene.tsx>            open Studio in the browser on the scene, the timeline or
  timeline studio <edit.timeline.json> the project, served from 127.0.0.1 until stopped
  project studio [folder]

options of studio:
  --port <n>                  the port to listen on (0, the default: one the system picks)
  --web-assets-dir <folder>   where Studio and the SDK are built (web/ of the source)

options of every command:
  --json                      print the result as JSON
  --events                    print what it does as it goes, a line of JSON each, then the
                              result as JSON; studio says `ready` with its address and stays

options of project:
  -C <folder>                 the project there, not the one the current folder is in

options of query (every one given holds):
  --clip <id>                 the clip of that id
  --track <id>                the clips of that track
  --index <n>                 the nth clip of its track by start, from 1
  --at <seconds|timecode>     the clips on at that time, with the source timecode there
  --marker <name>             the clips that carry that marker, or that the timeline's is on
  --meta <key=value>          metadata: a dotted path from the top, or a key at any depth

options of motion:
  --props <json|@file>        values of the props controls.props declares
  --data <json|@file>         values of the data controls.data declares
  --resource <name>=<path>    a file bound to a resource; a font's faces, once each
  --input <name>=<path>       a picture a role reads as input://<name>
  --caption <json|@file>      the caption a captionPresenter() shows

options of render (motion and timeline):
  --frame <n>                 the frame a .png shows (0)
  --frames <a,b,…|a..b>       the frames to draw (all)
  --storyboard                draw the frames side by side into the one .png
  --codec <h264|hevc|prores>  a video's codec: h264 for a .mp4, prores (4444, with alpha) for
                              a .mov
  --output-size <WxH>        output pixels, without relayout (up to 7680x4320 / 8K)
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
    // Some of it done and the rest not: the command failed.
    let partial = matches!(&result, Ok(v) if v["status"] == "partial");
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
                let message = p["message"].as_str().unwrap_or("");
                let (first, count) = (&p["first"], p["count"].as_u64().unwrap_or(1));
                let more = match count {
                    1 => String::new(),
                    n => format!(" and {} more", n - 1),
                };
                match first.is_null() {
                    true => eprintln!("{message}"),
                    false => eprintln!("frame {first}{more}: {message}"),
                }
            }
            eprintln!("error: {}", f.message);
            if let Some(hint) = f.hint {
                eprintln!("hint: {hint}");
            }
            return ExitCode::FAILURE;
        }
    }
    match partial {
        true => ExitCode::FAILURE,
        false => ExitCode::SUCCESS,
    }
}

fn run(words: &[String]) -> (Result<Value, Failure>, bool) {
    let events = words.iter().any(|w| w == "--events");
    EVENTS.store(events, Ordering::Relaxed);
    let json = events || words.iter().any(|w| w == "--json");
    let result = (|| {
        let usage = || Failure::new("usage", "no such command").hint(USAGE.trim_end());
        if let [group, command, rest @ ..] = words
            && command == "studio"
            && ["motion", "timeline", "project"].contains(&group.as_str())
        {
            return studio::run(group, rest, events);
        }
        if let [group, command, rest @ ..] = words
            && group == "project"
        {
            return project::run(command, rest);
        }
        let (group, command, rest) = match words {
            [group, command, rest @ ..] if group == "motion" || group == "timeline" => {
                (group.as_str(), command.as_str(), rest)
            }
            [help] if help == "--help" || help == "help" => {
                return Ok(json!({ "status": "ok", "usage": USAGE }));
            }
            _ => return Err(usage()),
        };
        let mut args = Args::parse(rest)?;
        let document = args.file.ends_with(".timeline.json");
        args.timeline = group == "timeline";
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
            "query" if group == "timeline" => timeline::query(&args),
            "edit" if group == "timeline" => timeline::edit(&args),
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
