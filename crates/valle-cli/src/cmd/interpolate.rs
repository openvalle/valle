//! Thin CLI adapter for `valle media interpolate`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output: PathBuf,
    fps: u32,
    shots: Option<PathBuf>,
    model: String,
    model_version: Option<String>,
    backend: crate::MediaBackendArg,
    overwrite: bool,
    report: Option<PathBuf>,
    json: bool,
) -> Result<std::process::ExitCode> {
    use valle_media::{
        models::{ModelManager, ModelSelection},
        tools::{
            PreparedRunReport, RunContext,
            interpolate::{InterpolateRequest, run as run_interpolate},
        },
    };

    if let Err(error) =
        super::media::validate_distinct_paths(input, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    if let Some(shots) = shots.as_deref()
        && let Err(error) =
            super::media::validate_distinct_paths(shots, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    let mut protected = vec![input, output.as_path()];
    if let Some(shots) = shots.as_deref() {
        protected.push(shots);
    }
    let prepared_report = match report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, overwrite, &protected))
        .transpose()
    {
        Ok(prepared) => prepared,
        Err(error) => return super::media::render_error(&error, json),
    };
    let manager = ModelManager::new();
    let mut progress = super::media::TerminalProgress::new(!json);
    let mut context = RunContext::new(&manager, &mut progress);
    if let Err(error) = super::media::enable_cancellation(&mut context) {
        return super::media::render_error(&error, json);
    }
    let request = InterpolateRequest {
        input: input.to_owned(),
        output,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        fps,
        shots,
        overwrite,
    };
    let mut run = match run_interpolate(request, &mut context) {
        Ok(run) => run,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Some(prepared_report) = prepared_report {
        prepared_report.write(&mut run);
    }

    if json {
        super::media::print_json_success(&run)?;
    } else {
        let output = run
            .result
            .outputs
            .first()
            .map(|artifact| artifact.path.display().to_string())
            .unwrap_or_else(|| "(no output)".to_owned());
        eprintln!(
            "interpolated {} -> {} frames at {} fps in {:.1}s ({output})",
            run.result.source_frames,
            run.result.output_frames,
            run.result.output_fps,
            run.report.timing.total_seconds,
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}
