//! Thin CLI adapter for `valle media matte`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output: Option<PathBuf>,
    range: Option<String>,
    fps: f64,
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
            PreparedRunReport, RunContext, TimeRange,
            matte::{MatteRequest, run as run_matte},
        },
    };

    let parsed_range = match parse_range(range.as_deref()) {
        Ok(range) => range,
        Err(error) => return super::media::render_error(&error, json),
    };
    let range = match parsed_range
        .map(|(start, end)| TimeRange::new(start, end))
        .transpose()
    {
        Ok(range) => range,
        Err(error) => return super::media::render_error(&error, json),
    };
    let output = output.unwrap_or_else(|| {
        if input
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("png"))
        {
            input.with_extension("foreground.png")
        } else {
            input.with_extension("foreground.mov")
        }
    });
    if let Err(error) =
        super::media::validate_distinct_paths(input, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    let prepared_report = match report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, overwrite, &[input, output.as_path()]))
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
    let request = MatteRequest {
        input: input.to_owned(),
        output,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        range,
        sample_fps: Some(fps),
        overwrite,
    };
    let mut run = match run_matte(request, &mut context) {
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
        if let Some(fps) = run.result.fps {
            eprintln!(
                "created {} frames at {} fps ({:.0} ms/frame) ({output})",
                run.result.frames, fps, run.result.milliseconds_per_frame,
            );
        } else {
            eprintln!(
                "created transparent PNG ({:.0} ms) ({output})",
                run.result.milliseconds_per_frame,
            );
        }
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// Parse the comma-separated `start,end` spelling into a typed half-open range.
fn parse_range(
    value: Option<&str>,
) -> std::result::Result<Option<(f64, Option<f64>)>, valle_media::tools::ToolError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let (start, end) = value.split_once(',').ok_or_else(|| {
        valle_media::tools::ToolError::invalid_input(format!(
            "--range wants 'start,end' seconds (got {value:?})"
        ))
    })?;
    let start = start.trim().parse::<f64>().map_err(|error| {
        valle_media::tools::ToolError::invalid_input(format!(
            "invalid --range start {start:?}: {error}"
        ))
    })?;
    let end = if end.trim().is_empty() {
        None
    } else {
        Some(end.trim().parse::<f64>().map_err(|error| {
            valle_media::tools::ToolError::invalid_input(format!(
                "invalid --range end {end:?}: {error}"
            ))
        })?)
    };
    Ok(Some((start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parser_preserves_open_end() {
        assert_eq!(parse_range(None).unwrap(), None);
        assert_eq!(parse_range(Some("1.5,4")).unwrap(), Some((1.5, Some(4.0))));
        assert_eq!(parse_range(Some("2,")).unwrap(), Some((2.0, None)));
        assert!(parse_range(Some("2")).is_err());
    }
}
