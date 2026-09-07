//! Thin CLI adapter for `valle media separate`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output_dir: PathBuf,
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
            separate::{SeparateRequest, run as run_separate},
        },
    };

    if let Err(error) =
        super::media::validate_distinct_paths(input, Some(&output_dir), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    let prepared_report = match report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, overwrite, &[input, output_dir.as_path()]))
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
    let request = SeparateRequest {
        input: input.to_owned(),
        output_dir,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
    };
    let mut run = match run_separate(request, &mut context) {
        Ok(run) => run,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Some(prepared_report) = prepared_report {
        prepared_report.write(&mut run);
    }

    if json {
        super::media::print_json_success(&run)?;
    } else {
        let directory = run
            .result
            .outputs
            .first()
            .and_then(|artifact| artifact.path.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(no output)".to_owned());
        eprintln!(
            "separated {:.1}s audio in {:.1}s (RTF {:.2}) ({directory})",
            run.result.audio_seconds, run.report.timing.total_seconds, run.result.real_time_factor,
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}
