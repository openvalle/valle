//! `valle media`: tools apart from timelines and projects, and the models they run.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use valle_media::download::{self, Source};
use valle_media::models::{self, ModelSpec};
use valle_media::transcribe::{self as asr, SAMPLE_RATE, Transcript};
use valle_native::media;

use crate::Failure;
use crate::cli::Model;

pub fn model(command: Model, quiet: bool) -> Result<Value, Failure> {
    let root = models::root().map_err(|e| Failure::new("media", e))?;
    match command {
        Model::List => Ok(list(&root)),
        Model::Install {
            models,
            source,
            from,
        } => {
            let source = Source::parse(&source).map_err(|e| Failure::new("usage", e))?;
            let specs = models
                .iter()
                .map(|id| models::find(id))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Failure::new("usage", e).hint("valle media model list"))?;
            match from {
                Some(from) => import(&root, &specs, &from),
                None => install(&root, &specs, source, quiet),
            }
        }
    }
}

fn list(root: &Path) -> Value {
    let specs = models::catalog();
    let mut lines = Vec::new();
    let entries: Vec<Value> = specs
        .iter()
        .map(|spec| {
            let installed = models::installed(root, spec);
            let dir = models::dir(root, spec);
            lines.push(format!(
                "{}  {}, {}, {}",
                spec.id,
                models::purpose(spec),
                gigabytes(models::size(spec)),
                match installed {
                    true => format!("installed in {}", dir.display()),
                    false => "not installed".into(),
                }
            ));
            json!({
                "id": spec.id,
                "purpose": models::purpose(spec),
                "bytes": models::size(spec),
                "repository": spec.repository,
                "revision": spec.revision,
                "license": spec.license,
                "installed": installed,
                "folder": dir.display().to_string(),
            })
        })
        .collect();
    let missing: Vec<&str> = specs
        .iter()
        .filter(|s| !models::installed(root, s))
        .map(|s| s.id.as_str())
        .collect();
    if !missing.is_empty() {
        lines.push(format!("Install: {}", install_command(&missing)));
    }
    json!({
        "status": "ok",
        "folder": root.display().to_string(),
        "models": entries,
        "summary": lines.join("\n"),
    })
}

fn install(
    root: &Path,
    specs: &[ModelSpec],
    source: Source,
    quiet: bool,
) -> Result<Value, Failure> {
    let mut installed = Vec::new();
    for spec in specs {
        if models::installed(root, spec) {
            installed.push((spec, models::dir(root, spec)));
            continue;
        }
        let mut shown = Shown::new(spec, quiet);
        let dir = download::install(root, spec, source, &mut |file, done| {
            shown.progress(file, done)
        })
        .map_err(|e| {
            let others: Vec<String> = Source::ALL
                .into_iter()
                .filter(|s| *s != source)
                .map(|s| format!("--source {}", s.name()))
                .collect();
            Failure::new("download", e).hint(format!(
                "run it again to pick up where it stopped, or try {}",
                others.join(" or ")
            ))
        })?;
        shown.finish();
        installed.push((spec, dir));
    }
    Ok(done(&installed))
}

fn import(root: &Path, specs: &[ModelSpec], from: &Path) -> Result<Value, Failure> {
    let [spec] = specs else {
        return Err(Failure::new("usage", "--from installs one model at a time"));
    };
    let dir = models::import(root, spec, from).map_err(|e| {
        Failure::new("media", e).hint(format!("or download it: {}", install_command(&[&spec.id])))
    })?;
    Ok(done(&[(spec, dir)]))
}

fn done(installed: &[(&ModelSpec, std::path::PathBuf)]) -> Value {
    let summary: Vec<String> = installed
        .iter()
        .map(|(spec, dir)| format!("Installed {} in {}", spec.id, dir.display()))
        .collect();
    json!({
        "status": "ok",
        "models": installed.iter().map(|(spec, dir)| json!({
            "id": spec.id,
            "folder": dir.display().to_string(),
        })).collect::<Vec<_>>(),
        "summary": summary.join("\n"),
    })
}

pub fn transcribe(
    file: &Path,
    output: Option<&Path>,
    language: Option<String>,
    words: bool,
    quiet: bool,
) -> Result<Value, Failure> {
    let root = models::root().map_err(|e| Failure::new("media", e))?;
    let missing = asr::missing(&root, words);
    if !missing.is_empty() {
        let names: Vec<String> = missing
            .iter()
            .map(|s| format!("{} ({})", s.id, gigabytes(models::size(s))))
            .collect();
        let ids: Vec<&str> = missing.iter().map(|s| s.id.as_str()).collect();
        let failure = Failure::new("model", format!("needs {} installed", names.join(" and ")));
        return Err(failure.hint(format!(
            "{}; or --from a folder that holds one. valle media model list says what is installed",
            install_command(&ids)
        )));
    }
    if output.is_some_and(|o| o.extension().is_none_or(|e| e != "json")) {
        return Err(Failure::new("usage", "-o is a .json file"));
    }
    let samples = media::sound_at(file, SAMPLE_RATE, 1)
        .map_err(|e| Failure::new("video", e))?
        .ok_or_else(|| Failure::new("media", format!("{} has no sound", file.display())))?;
    if !quiet {
        let seconds = samples.len() as f64 / f64::from(SAMPLE_RATE);
        eprintln!("Transcribing {:.1} s of {}", seconds, file.display());
    }
    let transcript =
        asr::transcribe(&root, samples, language, words).map_err(|e| Failure::new("media", e))?;
    let document = document(&transcript, words);
    let segments = transcript.segments.len();
    let text = transcript.text.trim();
    let Some(output) = output else {
        let mut value = document;
        value["status"] = "ok".into();
        value["summary"] = text.into();
        return Ok(value);
    };
    let json = serde_json::to_string_pretty(&document).expect("a transcript is JSON");
    std::fs::write(output, json + "\n")
        .map_err(|e| Failure::new("media", format!("{}: {e}", output.display())))?;
    Ok(json!({
        "status": "ok",
        "file": output.display().to_string(),
        "language": transcript.language,
        "duration": seconds(transcript.duration_ms),
        "segments": segments,
        "summary": format!("Wrote {} segments to {}\n{text}", segments, output.display()),
    }))
}

/// A transcript as written: times in seconds into the file, as a timeline gives them.
fn document(transcript: &Transcript, words: bool) -> Value {
    let segments: Vec<Value> = transcript
        .segments
        .iter()
        .map(|s| {
            let mut segment = json!({
                "start": seconds(s.start_ms),
                "end": seconds(s.end_ms),
                "language": s.language,
                "text": s.text,
            });
            if words {
                segment["words"] = s
                    .words
                    .iter()
                    .map(|w| {
                        json!({ "text": w.text, "start": seconds(w.start_ms), "end": seconds(w.end_ms) })
                    })
                    .collect();
            }
            segment
        })
        .collect();
    json!({
        "model": transcript.model,
        "language": transcript.language,
        "duration": seconds(transcript.duration_ms),
        "text": transcript.text,
        "segments": segments,
    })
}

fn seconds(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

/// The command that installs `ids`.
pub fn install_command(ids: &[&str]) -> String {
    format!("valle media model install {}", ids.join(" "))
}

/// A size in gigabytes, as `1.88 GB`.
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.2} GB", bytes as f64 / 1e9)
}

/// What a download has come to, on stderr: at most once a second, and as each file is done.
struct Shown {
    quiet: bool,
    id: String,
    total: u64,
    /// The bytes of the files before this one.
    before: u64,
    file: String,
    done: u64,
    said: Option<Instant>,
}

impl Shown {
    fn new(spec: &ModelSpec, quiet: bool) -> Shown {
        if !quiet {
            eprintln!("Installing {} ({})", spec.id, gigabytes(models::size(spec)));
        }
        Shown {
            quiet,
            id: spec.id.clone(),
            total: models::size(spec),
            before: 0,
            file: String::new(),
            done: 0,
            said: None,
        }
    }

    fn progress(&mut self, file: &models::ModelFile, done: u64) {
        if file.path != self.file {
            self.before += self.done;
            self.file = file.path.clone();
        }
        self.done = done;
        let due = self
            .said
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if self.quiet || !due {
            return;
        }
        self.said = Some(Instant::now());
        let all = self.before + done;
        let percent = all * 100 / self.total.max(1);
        let mut err = std::io::stderr().lock();
        let _ = write!(
            err,
            "\r{}: {percent}% of {}",
            self.id,
            gigabytes(self.total)
        );
        let _ = err.flush();
    }

    fn finish(&self) {
        if !self.quiet && self.said.is_some() {
            eprintln!("\r{}: 100% of {}", self.id, gigabytes(self.total));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_what_to_install() {
        let root = std::env::temp_dir().join(format!("valle-cli-models-{}", std::process::id()));
        let value = list(&root);
        let summary = value["summary"].as_str().unwrap();
        assert!(summary.contains("qwen3-asr-0.6b  speech to text, 1.88 GB, not installed"));
        assert!(summary.ends_with(&install_command(&[models::ASR, models::ALIGNER])));
        assert_eq!(value["models"][0]["installed"], false);
    }
}
