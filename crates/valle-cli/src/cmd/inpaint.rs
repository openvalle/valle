//! Thin CLI adapter for `valle media inpaint`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    mask: &Path,
    output: Option<PathBuf>,
    range: Option<String>,
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
            inpaint::{InpaintRequest, run as run_inpaint},
        },
    };

    let range = match range.as_deref().map(parse_range).transpose() {
        Ok(range) => range,
        Err(error) => return super::media::render_error(&error, json),
    };
    let output = output.unwrap_or_else(|| default_output(input));
    if let Err(error) =
        super::media::validate_distinct_paths(input, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    if let Err(error) =
        super::media::validate_distinct_paths(mask, Some(&output), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    let prepared_report = match report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, overwrite, &[input, mask, output.as_path()]))
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
    let request = InpaintRequest {
        input: input.to_owned(),
        mask: mask.to_owned(),
        output,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        range,
        overwrite,
    };
    let mut run = match run_inpaint(request, &mut context) {
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
            "inpainted {} frame(s) with {} task(s) ({:.0} ms/frame) ({output})",
            run.result.frames, run.result.tasks, run.result.milliseconds_per_frame,
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

fn default_output(input: &Path) -> PathBuf {
    let extension = if input
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("png"))
    {
        "png"
    } else {
        "mp4"
    };
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    input.with_file_name(format!("{stem}.inpainted.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_preserves_image_or_video_kind() {
        assert_eq!(
            default_output(Path::new("/tmp/photo.PNG")),
            PathBuf::from("/tmp/photo.inpainted.png")
        );
        assert_eq!(
            default_output(Path::new("/tmp/clip.mov")),
            PathBuf::from("/tmp/clip.inpainted.mp4")
        );
    }

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
