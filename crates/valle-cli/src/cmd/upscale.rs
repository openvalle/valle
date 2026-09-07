//! Thin CLI adapter for `valle media upscale`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output: Option<PathBuf>,
    range: Option<String>,
    scale: u32,
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
            upscale::{UpscaleRequest, run as run_upscale},
        },
    };

    let range = match parse_range(range.as_deref()).and_then(|range| {
        range
            .map(|(start, end)| TimeRange::new(start, end))
            .transpose()
    }) {
        Ok(range) => range,
        Err(error) => return super::media::render_error(&error, json),
    };
    let output = output.unwrap_or_else(|| default_output(input, scale));
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
    let request = UpscaleRequest {
        input: input.to_owned(),
        output,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        scale,
        range,
        overwrite,
    };
    let mut run = match run_upscale(request, &mut context) {
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
            "upscaled {} frame(s) {}x{} -> {}x{} at x{} ({:.0} ms/frame) ({output})",
            run.result.frames,
            run.result.source_width,
            run.result.source_height,
            run.result.output_width,
            run.result.output_height,
            run.result.scale,
            run.result.milliseconds_per_frame,
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}

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

fn default_output(input: &Path, scale: u32) -> PathBuf {
    let parent = input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("output");
    let extension = if input
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        "png"
    } else {
        "mp4"
    };
    parent.join(format!("{stem}-x{scale}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_preserves_png_and_normalizes_video_to_mp4() {
        assert_eq!(
            default_output(Path::new("frames/input.png"), 4),
            PathBuf::from("frames/input-x4.png")
        );
        assert_eq!(
            default_output(Path::new("clips/input.mov"), 4),
            PathBuf::from("clips/input-x4.mp4")
        );
    }

    #[test]

    fn range_parser_preserves_open_end() {
        assert_eq!(parse_range(None).unwrap(), None);
        assert_eq!(parse_range(Some("1.5,4")).unwrap(), Some((1.5, Some(4.0))));
        assert_eq!(parse_range(Some("2,")).unwrap(), Some((2.0, None)));
        assert!(parse_range(Some("2")).is_err());
    }
}
