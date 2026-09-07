//! Thin CLI adapter for `valle media transcribe`.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    input: &Path,
    output: Option<PathBuf>,
    lang: Option<&str>,
    text_only: bool,
    level: &str,
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
            PreparedRunReport, RunContext, ToolError,
            transcribe::{TranscribeRequest, TranscriptPresentation, run as run_transcribe},
        },
    };

    let presentation = if text_only {
        if level != "word" {
            let error = ToolError::invalid_input(format!(
                "--text-only cannot be combined with --level {level:?}"
            ));
            return super::media::render_error(&error, json);
        }
        TranscriptPresentation::None
    } else {
        match level {
            "word" => TranscriptPresentation::WordJson,
            "sentence" => TranscriptPresentation::SentenceJson {
                max_len_seconds: 15.0,
            },
            _ => {
                let error = ToolError::invalid_input(format!(
                    "unknown --level {level:?} (word / sentence)"
                ));
                return super::media::render_error(&error, json);
            }
        }
    };

    let planned_output = match &presentation {
        TranscriptPresentation::None => None,
        TranscriptPresentation::WordJson => Some(
            output
                .clone()
                .unwrap_or_else(|| input.with_extension("words.json")),
        ),
        TranscriptPresentation::SentenceJson { .. } => Some(
            output
                .clone()
                .unwrap_or_else(|| input.with_extension("sentences.json")),
        ),
    };
    if let Err(error) =
        super::media::validate_distinct_paths(input, planned_output.as_deref(), report.as_deref())
    {
        return super::media::render_error(&error, json);
    }
    let mut protected_paths = vec![input];
    if let Some(output) = planned_output.as_deref() {
        protected_paths.push(output);
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
    let request = TranscribeRequest {
        input: input.to_owned(),
        output,
        lang: lang.map(ToOwned::to_owned),
        align: !text_only,
        presentation,
        model: ModelSelection {
            id: model,
            version: model_version,
            backend: backend.into(),
        },
        overwrite,
    };
    let mut run = match run_transcribe(request, &mut context) {
        Ok(run) => run,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Some(prepared_report) = prepared_report {
        prepared_report.write(&mut run);
    }

    if json {
        super::media::print_json_success(&run)?;
    } else if text_only {
        println!("{}", run.result.text);
        super::media::print_warnings(&run.warnings);
    } else {
        let output = run
            .result
            .outputs
            .first()
            .map(|artifact| artifact.path.display().to_string())
            .unwrap_or_else(|| "(no output)".to_owned());
        let count = if level == "sentence" {
            format!("{} sentences", run.result.sentence_count.unwrap_or(0))
        } else {
            format!("{} words", run.result.transcript.words.len())
        };
        eprintln!(
            "transcribed {:.1}s audio to {count} in {:.1}s ({output})",
            run.result.audio_seconds, run.report.timing.total_seconds,
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(std::process::ExitCode::SUCCESS)
}
