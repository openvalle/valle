//! `valle`: the command line. Each command is a function that gives JSON; this file only reads the
//! arguments and prints the result, as text or, with `--json`, as the JSON itself.

mod args;
mod cli;
mod docs;
mod http;
mod motion;
mod picture;
mod project;
mod studio;
mod timeline;
mod video;

use std::io::Write;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use clap::error::ErrorKind;
use serde_json::{Value, json};
use valle_engine::gpu::Gpu;

use cli::{Cli, Command, Motion, ProjectCommand, Timeline};

/// Scene code runs on the thread that loads it, which needs more than the 12 MiB stack QuickJS
/// is given.
const STACK: usize = 64 << 20;

/// Whether `--events` was given: what the command does is printed as it goes, a line of JSON
/// each, before its result.
static EVENTS: AtomicBool = AtomicBool::new(false);

/// When progress was last said, and at what whole percent.
static SAID: Mutex<Option<(Instant, usize)>> = Mutex::new(None);

/// Says, with `--events`, that `done` of `total` frames are drawn: at the first and the last
/// frame, and between them as the whole percent changes, at most four times a second, so a
/// long render says it in a hundred lines or fewer.
pub fn progress(done: usize, total: usize) {
    if !EVENTS.load(Ordering::Relaxed) {
        return;
    }
    let percent = done * 100 / total.max(1);
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    let due = match *said {
        _ if done == 1 || done == total => true,
        None => true,
        Some((at, last)) => percent > last && at.elapsed() >= Duration::from_millis(250),
    };
    if !due {
        return;
    }
    *said = Some((Instant::now(), percent));
    let line = json!({ "event": "progress", "done": done, "frames": total });
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

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

/// The device frames are drawn on, and what it is: the system's GPU, or the lavapipe the
/// executable carries where there is none, as `valle_native::gpu::adapter` picks.
pub fn gpu() -> Result<(Gpu, String), Failure> {
    #[cfg(all(
        feature = "embed-lavapipe",
        any(target_os = "linux", target_os = "windows")
    ))]
    // `cargo xtask package` names the driver valle-lavapipe compressed for this platform.
    let embedded: Option<&[u8]> = Some(include_bytes!(env!("VALLE_LAVAPIPE")));
    #[cfg(not(all(
        feature = "embed-lavapipe",
        any(target_os = "linux", target_os = "windows")
    )))]
    let embedded = None;
    let failed = |e| Failure::new("gpu", e);
    let chosen = valle_native::gpu::adapter(embedded).map_err(failed)?;
    let name = chosen.name();
    Ok((Gpu::with(chosen.adapter).map_err(failed)?, name))
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    // Cocoa's save panel needs the main thread. Studio invokes this short-lived helper so
    // choosing a file does not block its HTTP service or require a separate GUI event loop.
    if let [command, name] = words.as_slice()
        && command == "--studio-save-file"
    {
        println!("{}", studio::save_file(name));
        return ExitCode::SUCCESS;
    }
    let cli = match Cli::try_parse_from(std::iter::once("valle".to_string()).chain(words.clone())) {
        Ok(cli) => cli,
        Err(e) => return parse_failed(&e, &words),
    };
    let json = cli.json || cli.events;
    EVENTS.store(cli.events, Ordering::Relaxed);
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(cli));
    let result = match thread.map(|t| t.join()) {
        Ok(Ok(done)) => done,
        _ => Err(Failure::new("internal", "the command stopped")),
    };
    // Some of it done and the rest not: the command failed.
    let partial = matches!(&result, Ok(v) if v["status"] == "partial");
    match (result, json) {
        (Ok(value), true) => println!("{}", lean(value)),
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

/// What clap could not read, or the help or version asked for: as JSON with `--json` or
/// `--events`, else as clap writes it.
fn parse_failed(e: &clap::Error, words: &[String]) -> ExitCode {
    let json = words.iter().any(|w| w == "--json" || w == "--events");
    let shown = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
    if !json {
        let _ = e.print();
    } else if shown {
        println!("{}", json!({ "status": "ok", "usage": e.to_string() }));
    } else {
        let text = e.to_string();
        let message = (text.lines().next().unwrap_or_default())
            .trim_start_matches("error: ")
            .to_string();
        let out = json!({ "status": "error", "code": "usage", "message": message, "hint": text });
        println!("{out}");
    }
    match shown {
        true => ExitCode::SUCCESS,
        false => ExitCode::FAILURE,
    }
}

fn run(cli: Cli) -> Result<Value, Failure> {
    let events = cli.events;
    match cli.command {
        Command::Motion(Motion::Check { file, bind, frames }) => {
            scene_file(&file, false, "check")?;
            motion::check(&cli::scene_args(
                file,
                false,
                Some(bind),
                Some(frames),
                None,
            )?)
        }
        Command::Motion(Motion::Render { file, bind, render }) => {
            scene_file(&file, false, "render")?;
            motion::render(&cli::scene_args(
                file,
                false,
                Some(bind),
                None,
                Some(render),
            )?)
        }
        Command::Motion(Motion::Studio { file, serve }) => {
            studio::run("motion", Some(file), serve, events)
        }
        Command::Timeline(Timeline::Check { file, frames }) => {
            scene_file(&file, true, "check")?;
            motion::check(&cli::scene_args(file, true, None, Some(frames), None)?)
        }
        Command::Timeline(Timeline::Render { file, render }) => {
            scene_file(&file, true, "render")?;
            motion::render(&cli::scene_args(file, true, None, None, Some(render))?)
        }
        Command::Timeline(Timeline::Query { file, filter }) => {
            scene_file(&file, true, "query")?;
            timeline::query(
                &cli::scene_args(file, true, None, None, None)?,
                &filter.filter(),
            )
        }
        Command::Timeline(Timeline::Edit { file, op }) => {
            scene_file(&file, true, "edit")?;
            timeline::edit(&file, &op)
        }
        Command::Timeline(Timeline::Studio { file, serve }) => {
            studio::run("timeline", Some(file), serve, events)
        }
        Command::Docs { topic, section } => docs::run(topic, section),
        Command::Project(p) => match p.command {
            ProjectCommand::Studio { folder, serve } => {
                studio::run("project", folder.or(p.folder), serve, events)
            }
            command => project::run(p.folder.as_deref(), command),
        },
    }
}

/// A scene for `valle motion`, a JSON document for `valle timeline`.
fn scene_file(file: &str, timeline: bool, command: &str) -> Result<(), Failure> {
    match (timeline, file.ends_with(".json")) {
        (false, true) => Err(Failure::new("usage", "a JSON document is a timeline")
            .hint(format!("valle timeline {command} {file}"))),
        (true, false) => Err(
            Failure::new("usage", "valle timeline takes a JSON document")
                .hint(format!("valle motion {command} {file}")),
        ),
        _ => Ok(()),
    }
}

/// A result as JSON without its empty fields: a `null`, or an empty list, says nothing, and
/// costs a reader to skip. Only the top level: what it holds, such as a clip, stays as it is.
fn lean(mut value: Value) -> Value {
    if let Some(o) = value.as_object_mut() {
        o.retain(|_, v| !(v.is_null() || v.as_array().is_some_and(Vec::is_empty)));
    }
    value
}

/// A result as text: its summary, or a guide's content.
fn print_text(value: &Value) {
    let text = value["summary"].as_str().or(value["content"].as_str());
    if let Some(text) = text {
        // A reader such as `head` may stop before the end.
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", text.trim_end_matches('\n'));
    }
}
