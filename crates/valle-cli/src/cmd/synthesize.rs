//! Thin CLI adapter for `valle media synthesize`.

use std::{path::Path, process::ExitCode};

use anyhow::Result;
use valle_media::{
    models::{ModelManager, ModelSelection},
    tools::{
        PreparedRunReport, RunContext, ToolError,
        synthesize::{SynthesizeRequest, run as run_synthesize},
    },
};

pub(crate) fn run(args: crate::SynthesizeArgs, json: bool) -> Result<ExitCode> {
    let mut protected_paths = vec![args.reference.as_path()];
    protected_paths.extend(args.text_file.as_deref());
    protected_paths.extend(args.ref_text_file.as_deref());
    for input in &protected_paths {
        if let Err(error) =
            super::media::validate_distinct_paths(input, Some(&args.output), args.report.as_deref())
        {
            return super::media::render_error(&error, json);
        }
    }
    protected_paths.push(&args.output);
    let prepared_report = match args
        .report
        .as_deref()
        .map(|path| PreparedRunReport::new(path, args.overwrite, &protected_paths))
        .transpose()
    {
        Ok(prepared) => prepared,
        Err(error) => return super::media::render_error(&error, json),
    };
    let text = match read_text(args.text, args.text_file.as_deref(), "synthesis") {
        Ok(Some(text)) => text,
        Ok(None) => {
            return super::media::render_error(
                &ToolError::invalid_input("provide --text or --text-file"),
                json,
            );
        }
        Err(error) => return super::media::render_error(&error, json),
    };
    let reference_text = match read_text(args.ref_text, args.ref_text_file.as_deref(), "reference")
    {
        Ok(text) => text,
        Err(error) => return super::media::render_error(&error, json),
    };
    let request = SynthesizeRequest {
        text,
        reference: args.reference,
        reference_text,
        output: args.output,
        lang: args.lang,
        seed: args.seed,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        max_chunk_chars: args.max_chunk_chars,
        model: ModelSelection {
            id: args.model,
            version: args.model_version,
            backend: args.backend.into(),
        },
        overwrite: args.overwrite,
    };
    let manager = ModelManager::new();
    let mut progress = super::media::TerminalProgress::new(!json);
    let mut context = RunContext::new(&manager, &mut progress);
    if let Err(error) = super::media::enable_cancellation(&mut context) {
        return super::media::render_error(&error, json);
    }
    let mut run = match run_synthesize(request, &mut context) {
        Ok(run) => run,
        Err(error) => return super::media::render_error(&error, json),
    };
    if let Some(prepared_report) = prepared_report {
        prepared_report.write(&mut run);
    }
    if json {
        super::media::print_json_success(&run)?;
    } else {
        eprintln!(
            "synthesized {:.1}s speech in {:.1}s ({})",
            run.result.audio_seconds,
            run.report.timing.total_seconds,
            run.result.outputs[0].path.display(),
        );
        super::media::print_warnings(&run.warnings);
    }
    Ok(ExitCode::SUCCESS)
}

fn read_text(
    text: Option<String>,
    file: Option<&Path>,
    role: &str,
) -> Result<Option<String>, ToolError> {
    match (text, file) {
        (Some(_), Some(_)) => Err(ToolError::invalid_input(format!(
            "provide either {role} text or a text file"
        ))),
        (text, None) => Ok(text),
        (None, Some(path)) => std::fs::read_to_string(path).map(Some).map_err(|error| {
            ToolError::invalid_input(format!("read {role} text {}: {error}", path.display()))
        }),
    }
}
