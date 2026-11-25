//! `valle media`: tools apart from timelines and projects, and the models they run.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use valle_media::download::{self, Source};
use valle_media::inspect;
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

/// What counts as silence: every 10 ms under `below` dBFS, for at least `min` seconds.
pub struct Thresholds {
    pub below: f64,
    pub min: f64,
}

pub fn inspect(
    file: &Path,
    thresholds: &Thresholds,
    sheet: Option<(&Path, u32)>,
    quiet: bool,
) -> Result<Value, Failure> {
    if !thresholds.min.is_finite() || thresholds.min < 0.01 {
        return Err(Failure::new("usage", "--silence-min is 0.01 s or more"));
    }
    if !thresholds.below.is_finite() || thresholds.below >= 0.0 {
        return Err(Failure::new(
            "usage",
            "--silence-below is a level under 0 dBFS, such as -40",
        ));
    }
    if sheet.is_some_and(|(o, _)| o.extension().is_none_or(|e| !e.eq_ignore_ascii_case("png"))) {
        return Err(Failure::new("usage", "--sheet is a .png file"));
    }
    let info = media::probe(file).map_err(|e| Failure::new("video", e))?;
    if sheet.is_some() && info.size.is_none() {
        return Err(Failure::new(
            "usage",
            format!("{} has no video to draw a sheet of", file.display()),
        ));
    }
    let (measured, cached) = measured(file, quiet)?;
    let duration = info
        .duration
        .map(|t| t as f64 / media::TICKS_PER_SECOND as f64);
    let mut value = json!({
        "status": "ok",
        "file": file.display().to_string(),
        "duration": duration.map(|d| round(d, 3)),
        "width": info.size.map(|s| s[0]),
        "height": info.size.map(|s| s[1]),
        "fps": info.fps.map(|[n, d]| if d == 1 { n.to_string() } else { format!("{n}/{d}") }),
        "audio": info.audio,
        "cached": cached,
    });
    let mut lines = vec![shape(file, &info, duration)];
    if let Some(differences) = &measured.differences {
        let cuts = inspect::cuts(differences);
        lines.push(match cuts.is_empty() {
            true => "No cuts".into(),
            false => format!(
                "{}: {}",
                crate::motion::counted(cuts.len(), "cut"),
                listed(cuts.iter().map(|c| format!("{:.2}", c.time)))
            ),
        });
        value["cuts"] = cuts
            .iter()
            .map(|c| json!({ "time": round(c.time, 3), "difference": round(c.difference, 1) }))
            .collect();
    }
    if let Some(sound) = &measured.sound {
        let silences = inspect::silences(sound, thresholds.below, thresholds.min);
        let under = format!(
            "under {} dBFS for {} s or more",
            thresholds.below, thresholds.min
        );
        lines.push(match silences.is_empty() {
            true => format!("No silence {under}"),
            false => format!(
                "{} {under}: {}",
                crate::motion::counted(silences.len(), "silence"),
                listed(
                    silences
                        .iter()
                        .map(|s| format!("{:.2}–{:.2}", s.start, s.end))
                )
            ),
        });
        value["silences"] = silences
            .iter()
            .map(|s| {
                json!({ "start": round(s.start, 3), "end": round(s.end, 3), "level": round(s.level.max(-120.0), 1) })
            })
            .collect();
        let integrated = inspect::integrated(sound);
        let peak = inspect::true_peak(sound);
        lines.push(match (integrated, peak) {
            (Some(l), Some(p)) => format!("Loudness {l:.1} LUFS, true peak {p:.1} dBTP"),
            (None, Some(p)) => format!("Too quiet to measure loudness; true peak {p:.1} dBTP"),
            _ => "Silent throughout".into(),
        });
        value["loudness"] = json!({
            "integrated": integrated.map(|l| round(l, 1)),
            "truePeak": peak.map(|p| round(p, 1)),
            "seconds": inspect::each_second(sound).iter().map(|s| s.map(|l| round(l, 1))).collect::<Vec<_>>(),
        });
    }
    if let Some((out, samples)) = sheet {
        let layout = contact_sheet(file, &info, out, samples)?;
        lines.push(format!(
            "Sheet of {} frames in {}",
            layout["cells"].as_array().map_or(0, Vec::len),
            out.display()
        ));
        value["sheet"] = layout;
    }
    value["summary"] = lines.join("\n").into();
    Ok(value)
}

/// What inspecting a file measures, apart from any threshold.
struct Measured {
    /// Each frame after the first, by when it starts in seconds, with how much it differs from
    /// the one before; none without video.
    differences: Option<Vec<(f64, f64)>>,
    sound: Option<inspect::Sound>,
}

/// The file's measures, from the cache while the file is unchanged, and whether they were.
fn measured(file: &Path, quiet: bool) -> Result<(Measured, bool), Failure> {
    let cache = cache_path(file);
    if let Some(found) = cache.as_ref().and_then(|p| read_cache(p)) {
        return Ok((found, true));
    }
    if !quiet {
        eprintln!("Inspecting {}", file.display());
    }
    let failed = |e| Failure::new("video", e);
    let differences = media::differences(file).map_err(failed)?.map(|d| {
        d.into_iter()
            .map(|(t, d)| (t as f64 / media::TICKS_PER_SECOND as f64, d))
            .collect()
    });
    // Measured as stereo, a file of more channels mixed down to it.
    let mut meter: Option<inspect::Meter> = None;
    let found = media::sound_blocks(file, inspect::RATE, &|n| n.min(2), &mut |block, n| {
        meter
            .get_or_insert_with(|| inspect::Meter::new(n))
            .feed(block);
        Ok(())
    })
    .map_err(failed)?;
    let sound = found.map(|n| meter.unwrap_or_else(|| inspect::Meter::new(n)).finish());
    let measured = Measured { differences, sound };
    if let Some(path) = cache {
        // A cache that cannot be written is measured again next time.
        let _ = write_cache(&path, &measured);
    }
    Ok((measured, false))
}

/// Where a file's measures are kept: by its full path, size and time of change, and this Valle.
fn cache_path(file: &Path) -> Option<std::path::PathBuf> {
    let full = std::fs::canonicalize(file).ok()?;
    let meta = std::fs::metadata(&full).ok()?;
    let changed = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let key = format!(
        "{}\n{}\n{}\n{}",
        full.display(),
        meta.len(),
        changed.as_nanos(),
        env!("CARGO_PKG_VERSION")
    );
    let digest = ring::digest::digest(&ring::digest::SHA256, key.as_bytes());
    let name: String = digest.as_ref()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = valle_native::urls::global_dir().ok()?.join("inspect");
    Some(dir.join(format!("{name}.json")))
}

fn read_cache(path: &Path) -> Option<Measured> {
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let numbers =
        |v: &Value| -> Option<Vec<f64>> { v.as_array()?.iter().map(Value::as_f64).collect() };
    let differences = match &value["differences"] {
        Value::Null => None,
        v => Some(
            v.as_array()?
                .iter()
                .map(|p| Some((p[0].as_f64()?, p[1].as_f64()?)))
                .collect::<Option<Vec<_>>>()?,
        ),
    };
    let sound = match &value["sound"] {
        Value::Null => None,
        v => Some(inspect::Sound {
            levels: numbers(&v["levels"])?
                .into_iter()
                .map(|l| l as f32)
                .collect(),
            steps: numbers(&v["steps"])?,
            peak: v["peak"].as_f64()?,
        }),
    };
    Some(Measured { differences, sound })
}

fn write_cache(path: &Path, measured: &Measured) -> std::io::Result<()> {
    let value = json!({
        "differences": measured.differences.as_ref().map(|d| d.iter().map(|(t, d)| json!([t, d])).collect::<Vec<_>>()),
        "sound": measured.sound.as_ref().map(|s| json!({ "levels": s.levels, "steps": s.steps, "peak": s.peak })),
    });
    let dir = path.parent().expect("in the cache's folder");
    std::fs::create_dir_all(dir)?;
    // Written whole, then moved in, so a reader never sees half of it.
    let part = path.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&part, value.to_string())?;
    std::fs::rename(&part, path)
}

/// `samples` frames from the first to the last of the file's video, drawn into a storyboard at
/// `out`; its layout.
fn contact_sheet(
    file: &Path,
    info: &media::Info,
    out: &Path,
    samples: u32,
) -> Result<Value, Failure> {
    let (Some(size), Some(duration)) = (info.size, info.duration) else {
        return Err(Failure::new(
            "usage",
            format!("{} has no video to draw a sheet of", file.display()),
        ));
    };
    let [n, d] = info.fps.unwrap_or([30, 1]);
    let fps = valle_timeline::time::Rate::new(i64::from(n), i64::from(d))
        .map_err(|e| Failure::new("video", e))?;
    let count = fps.frames_in(valle_timeline::time::Tick(duration)).max(1);
    let n = i64::from(samples).min(count);
    let indices: Vec<i64> = match n {
        1 => vec![0],
        _ => (0..n)
            .map(|i| {
                ((i128::from(i) * i128::from(count - 1) + i128::from((n - 1) / 2))
                    / i128::from(n - 1)) as i64
            })
            .collect(),
    };
    let mut reader = media::reader(file, None).map_err(|e| Failure::new("video", e))?;
    let mut board = crate::picture::Storyboard::new(size, &indices, fps)?;
    for (k, &i) in indices.iter().enumerate() {
        let at = fps.frame_start(i).map_err(|e| Failure::new("video", e))?;
        let pixels = reader.frame(at.0).map_err(|e| Failure::new("video", e))?;
        board.paint(k, &pixels);
        crate::progress(k + 1, indices.len());
    }
    let (gpu, _) = crate::gpu()?;
    board.label(&crate::picture::Labels::new(&gpu, crate::motion::fonts())?);
    crate::picture::write_png(out, board.size, &board.pixels)?;
    let mut layout = board.layout;
    layout["file"] = out.display().to_string().into();
    Ok(layout)
}

/// The file's length, size, rate and whether it has sound, as one line.
fn shape(file: &Path, info: &media::Info, duration: Option<f64>) -> String {
    let mut parts = vec![file.display().to_string()];
    if let Some(d) = duration {
        parts.push(format!("{d:.2} s"));
    }
    if let Some([w, h]) = info.size {
        parts.push(format!("{w}×{h}"));
    }
    if let Some([n, d]) = info.fps {
        parts.push(format!("{} fps", round(f64::from(n) / f64::from(d), 3)));
    }
    parts.push(if info.audio { "sound" } else { "no sound" }.into());
    parts.join(", ")
}

/// Items in a line: the first 10, and how many more.
fn listed(items: impl ExactSizeIterator<Item = String>) -> String {
    let more = items.len().saturating_sub(10);
    let mut text: Vec<String> = items.take(10).collect();
    if more > 0 {
        text.push(format!("and {more} more"));
    }
    text.join(", ")
}

fn round(v: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (v * scale).round() / scale
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
