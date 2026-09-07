//! Thin CLI adapter for `valle media segment`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output: PathBuf,
    foreground_output: Option<PathBuf>,
    prompt: PathBuf,
    range: Option<String>,
    threshold: Option<f32>,
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
            segment::{SegmentRequest, run as run_segment},
        },
    };

    let range = match range.as_deref().map(parse_range).transpose() {
        Ok(range) => range,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Err(error) =
        super::media::validate_distinct_paths(input, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    if let Err(error) =
        super::media::validate_distinct_paths(&prompt, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    if let Some(foreground) = foreground_output.as_deref() {
        for protected in [input, prompt.as_path(), output.as_path()] {
            if let Err(error) = super::media::validate_distinct_paths(
                protected,
                Some(foreground),
                report.as_deref(),
            ) {
                return super::media::render_error(&error, json);
            }
        }
    }
    let mut protected_paths = vec![input, prompt.as_path(), output.as_path()];
    if let Some(foreground) = foreground_output.as_deref() {
        protected_paths.push(foreground);
    }
    let prepared_report = match report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, overwrite, &protected_paths))
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
    let request = SegmentRequest {
        input: input.to_owned(),
        output,
        foreground_output,
        prompt,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        range,
        threshold,
        overwrite,
    };
    let mut run = match run_segment(request, &mut context) {
        Ok(run) => run,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Some(prepared_report) = prepared_report {
        prepared_report.write(&mut run);
    }

    if json {
        super::media::print_json_success(&run)?;
    } else {
        let outputs = run
            .result
            .outputs
            .iter()
            .map(|artifact| format!("{}={}", artifact.role, artifact.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "segmented {} frame(s) in {:.1}s ({:.1} ms/frame) ({outputs})",
            run.result.frames, run.report.timing.total_seconds, run.result.milliseconds_per_frame,
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}

fn parse_range(
    value: &str,
) -> Result<valle_media::tools::TimeRange, valle_media::tools::ToolError> {
    use valle_media::tools::{TimeRange, ToolError};

    let Some((start, end)) = value.split_once(',') else {
        return Err(ToolError::invalid_input(
            "range must use start,end seconds (for example 1.5,8.0)",
        ));
    };
    let start = start
        .trim()
        .parse::<f64>()
        .map_err(|_| ToolError::invalid_input("range start is not a number"))?;
    let end = if end.trim().is_empty() {
        None
    } else {
        Some(
            end.trim()
                .parse::<f64>()
                .map_err(|_| ToolError::invalid_input("range end is not a number"))?,
        )
    };
    TimeRange::new(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parser_is_half_open_and_accepts_open_end() {
        assert_eq!(
            parse_range("1.5,2.5").unwrap(),
            valle_media::tools::TimeRange::new(1.5, Some(2.5)).unwrap()
        );
        assert_eq!(
            parse_range("1.5,").unwrap(),
            valle_media::tools::TimeRange::new(1.5, None).unwrap()
        );
        assert!(parse_range("1.5").is_err());
    }
}
