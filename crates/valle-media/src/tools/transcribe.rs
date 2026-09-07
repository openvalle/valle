//! Complete-file transcription workflow.

use std::{collections::BTreeMap, fs::File, io::Write, path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    analysis::{Sentence, Transcript, sentences},
    models::{
        ModelErrorCode, ModelManager, ModelSelection, ResolveRequest, RunBackendPreference,
        adapters::qwen_asr::{
            ALIGNER_MODEL_ID, ALIGNER_RELEASE_VERSION, ASR_MODEL_ID, ASR_RELEASE_VERSION,
            QwenAlignerSession, QwenAsrSession, resolve_alignment_language,
        },
    },
};

use super::{
    ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport, RunContext, StageTiming,
    ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun,
    audio_workspace::AudioWorkspace,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    probe_audio_input,
};

const ASR_SAMPLE_RATE: u32 = 16_000;
const ASR_CHANNELS: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptPresentation {
    None,
    WordJson,
    SentenceJson { max_len_seconds: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscribeRequest {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub lang: Option<String>,
    pub align: bool,
    pub presentation: TranscriptPresentation,
    pub model: ModelSelection,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeResult {
    pub transcript: Transcript,
    pub text: String,
    pub audio_seconds: f64,
    pub sentence_count: Option<usize>,
    pub outputs: Vec<OutputArtifact>,
}

#[derive(Debug, Serialize)]
struct SentenceDocument<'a> {
    lang: Option<&'a str>,
    sentences: &'a [Sentence],
}

pub fn run(
    request: TranscribeRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<TranscribeResult>, ToolError> {
    context.validate()?;
    validate_request(&request)?;
    if !matches!(&request.presentation, TranscriptPresentation::None) {
        let output = request.output.clone().unwrap_or_else(|| {
            request.input.with_extension(match &request.presentation {
                TranscriptPresentation::None => unreachable!(),
                TranscriptPresentation::WordJson => "words.json",
                TranscriptPresentation::SentenceJson { .. } => "sentences.json",
            })
        });
        FileOutputTransaction::validate_target(&output, request.overwrite)?;
    }
    let started = Instant::now();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;

    let asr_candidates =
        resolve_qwen_candidates(context.models, request.model.clone(), ASR_RELEASE_VERSION)?;
    let mut aligner_candidates = request
        .align
        .then(|| {
            resolve_qwen_candidates(
                context.models,
                default_aligner_selection(request.model.backend),
                ALIGNER_RELEASE_VERSION,
            )
        })
        .transpose()?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decode_started = Instant::now();
    let input_report = probe_audio_input(&request.input)?;
    let mut workspace = AudioWorkspace::decode(
        &request.input,
        ASR_SAMPLE_RATE,
        ASR_CHANNELS,
        &context.resources,
        &context.cancellation,
    )?;
    let decode_seconds = decode_started.elapsed().as_secs_f64();
    let audio_seconds = workspace.total_frames() as f64 / f64::from(workspace.sample_rate());
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let mut load_seconds = 0.0;
    let load_started = Instant::now();
    let opened = asr_candidates.open(
        &format!("load {ASR_MODEL_ID}@{ASR_RELEASE_VERSION}"),
        |model| {
            QwenAsrSession::open(
                model,
                request.lang.as_deref(),
                context.resources.cpu_threads,
            )
        },
    )?;
    let mut asr = opened.session;
    let asr_provenance = ModelProvenance::from(&opened.model.resolved);
    let mut warnings = opened.warnings;
    // Resolving a candidate is not the same as using its model. Empty ASR output skips alignment,
    // so record aligner provenance only after a concrete aligner session opens successfully.
    let mut aligner_provenance = None;
    load_seconds += load_started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Inferencing));
    let mut inference_seconds = 0.0;
    let asr_window_frames = u64::try_from(asr.max_chunk_samples()).map_err(|_| {
        ToolError::new(
            ToolErrorCode::ModelLoadFailed,
            "Qwen ASR route window does not fit the host frame counter",
        )
    })?;
    let total_windows = workspace.total_frames().div_ceil(asr_window_frames);
    let mut text = String::new();
    let mut segments = Vec::new();
    for window_index in 0..total_windows {
        check_cancelled(context)?;
        let start_frame = window_index * asr_window_frames;
        let end_frame = (start_frame + asr_window_frames).min(workspace.total_frames());
        let chunk = workspace.read_frames(start_frame, end_frame)?;
        let inference_started = Instant::now();
        let output = asr
            .transcribe(&chunk.samples, start_frame)
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::InferenceFailed,
                    format!(
                        "transcribe {} window {}: {error:#}",
                        request.input.display(),
                        window_index + 1
                    ),
                )
            })?;
        inference_seconds += inference_started.elapsed().as_secs_f64();
        append_transcript_text(&mut text, &output.text);
        let window_start_ms = frames_to_millis(start_frame);
        let window_end_ms = frames_to_millis(end_frame);
        for mut segment in output.segments {
            segment.start_ms = segment.start_ms.clamp(window_start_ms, window_end_ms);
            segment.end_ms = segment
                .end_ms
                .clamp(window_start_ms, window_end_ms)
                .max(segment.start_ms);
            segments.push(segment);
        }
        context.progress.event(ToolEvent::count(
            ToolPhase::Inferencing,
            window_index + 1,
            total_windows,
        ));
    }
    drop(asr);

    let (alignment_language, lang_code) =
        resolve_alignment_language(request.lang.as_deref(), &text)
            .map_err(|error| ToolError::new(ToolErrorCode::InvalidInput, error.to_string()))?;
    let mut words = Vec::new();
    if request.align && !text.is_empty() {
        context.progress.event(ToolEvent::phase(ToolPhase::Loading));
        let load_started = Instant::now();
        let candidates = aligner_candidates
            .take()
            .expect("alignment model was resolved when alignment was requested");
        let opened = candidates.open(
            &format!("load {ALIGNER_MODEL_ID}@{ALIGNER_RELEASE_VERSION}"),
            |model| QwenAlignerSession::open(model, context.resources.cpu_threads),
        )?;
        let mut aligner = opened.session;
        aligner_provenance = Some(ModelProvenance::from(&opened.model.resolved));
        warnings.extend(opened.warnings);
        load_seconds += load_started.elapsed().as_secs_f64();
        let segment_count = segments
            .iter()
            .filter(|segment| !segment.text.trim().is_empty() && segment.end_ms > segment.start_ms)
            .count() as u64;
        let mut completed = 0_u64;
        let mut last_start = 0.0_f64;
        for segment in &segments {
            if segment.text.trim().is_empty() || segment.end_ms <= segment.start_ms {
                continue;
            }
            check_cancelled(context)?;
            let start_frame = millis_to_frames(segment.start_ms).min(workspace.total_frames());
            let end_frame = millis_to_frames(segment.end_ms).min(workspace.total_frames());
            if end_frame <= start_frame {
                continue;
            }
            let chunk = workspace.read_frames(start_frame, end_frame)?;
            let inference_started = Instant::now();
            let aligned = aligner
                .align(&chunk.samples, &segment.text, alignment_language)
                .map_err(|error| {
                    ToolError::new(
                        ToolErrorCode::InferenceFailed,
                        format!(
                            "align {} segment at {}ms: {error:#}",
                            request.input.display(),
                            segment.start_ms
                        ),
                    )
                })?;
            inference_seconds += inference_started.elapsed().as_secs_f64();
            let offset = segment.start_ms as f64 / 1_000.0;
            for unit in aligned {
                let start = (offset + unit.start_ms as f64 / 1_000.0).max(last_start);
                let end = (offset + unit.end_ms as f64 / 1_000.0).max(start);
                last_start = start;
                words.push(crate::analysis::Word {
                    id: format!("w{}", words.len()),
                    text: unit.text,
                    start,
                    end,
                });
            }
            completed += 1;
            context.progress.event(ToolEvent::count(
                ToolPhase::Inferencing,
                completed,
                segment_count,
            ));
        }
    }
    check_cancelled(context)?;

    let mut transcript = Transcript {
        audio: request
            .input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        lang: Some(lang_code.to_owned()),
        words,
    };
    quantize_word_times(&mut transcript)?;
    transcript.validate().map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("invalid ASR transcript: {error}"),
        )
    })?;

    let mut outputs = Vec::new();
    let mut sentence_count = None;
    let encode_started = Instant::now();
    match &request.presentation {
        TranscriptPresentation::None => {}
        TranscriptPresentation::WordJson => {
            context
                .progress
                .event(ToolEvent::phase(ToolPhase::Encoding));
            let target = request
                .output
                .clone()
                .unwrap_or_else(|| request.input.with_extension("words.json"));
            write_json(&target, request.overwrite, &transcript)?;
            outputs.push(OutputArtifact {
                role: "transcript_words".to_owned(),
                path: target,
                media_type: "application/json".to_owned(),
                summary: None,
            });
        }
        TranscriptPresentation::SentenceJson { max_len_seconds } => {
            context
                .progress
                .event(ToolEvent::phase(ToolPhase::Encoding));
            let projected = sentences(&transcript.words, *max_len_seconds);
            sentence_count = Some(projected.len());
            let target = request
                .output
                .clone()
                .unwrap_or_else(|| request.input.with_extension("sentences.json"));
            write_json(
                &target,
                request.overwrite,
                &SentenceDocument {
                    lang: transcript.lang.as_deref(),
                    sentences: &projected,
                },
            )?;
            outputs.push(OutputArtifact {
                role: "transcript_sentences".to_owned(),
                path: target,
                media_type: "application/json".to_owned(),
                summary: None,
            });
        }
    }
    let encode_seconds = encode_started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;

    let mut models = vec![asr_provenance];
    if let Some(aligner) = aligner_provenance {
        models.push(aligner);
    }
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));
    Ok(ToolRun {
        result: TranscribeResult {
            transcript,
            text,
            audio_seconds,
            sentence_count,
            outputs: outputs.clone(),
        },
        report: ProcessingReport {
            operation: "transcribe".to_owned(),
            parameters: normalized_parameters(&request),
            input: request.input,
            input_media_type: input_report.media_type,
            input_summary: input_report.summary,
            outputs,
            models,
            processed: ProcessedMedia {
                frames: 0,
                audio_seconds,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                inference_seconds,
                encode_seconds,
                total_seconds,
                ..StageTiming::default()
            },
        },
        warnings,
    })
}

fn append_transcript_text(transcript: &mut String, chunk: &str) {
    let chunk = chunk.trim();
    if chunk.is_empty() {
        return;
    }
    if transcript
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && chunk
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        transcript.push(' ');
    }
    transcript.push_str(chunk);
}

fn frames_to_millis(frames: u64) -> u64 {
    frames.saturating_mul(1_000) / u64::from(ASR_SAMPLE_RATE)
}

fn millis_to_frames(milliseconds: u64) -> u64 {
    milliseconds
        .saturating_mul(u64::from(ASR_SAMPLE_RATE))
        .div_ceil(1_000)
}

fn validate_request(request: &TranscribeRequest) -> Result<(), ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if request.model.id != ASR_MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "transcribe supports model {ASR_MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    if matches!(request.presentation, TranscriptPresentation::None) && request.output.is_some() {
        return Err(ToolError::invalid_input(
            "an output path requires word_json or sentence_json presentation",
        ));
    }
    if let TranscriptPresentation::SentenceJson { max_len_seconds } = request.presentation
        && (!max_len_seconds.is_finite() || max_len_seconds <= 0.0)
    {
        return Err(ToolError::invalid_input(
            "sentence max_len_seconds must be finite and greater than zero",
        ));
    }
    if !request.align && !matches!(request.presentation, TranscriptPresentation::None) {
        return Err(ToolError::invalid_input(
            "word and sentence JSON presentations require alignment",
        ));
    }
    if request.model.backend != RunBackendPreference::Auto {
        return Err(ToolError::new(
            ToolErrorCode::NoCompatibleRoute,
            "Qwen transcription supports --backend auto only; its native-cpu route is selected internally",
        )
        .with_hint("use --backend auto"));
    }
    if let Some(output) = &request.output
        && paths_refer_to_same_file(&request.input, output)
    {
        return Err(ToolError::invalid_input(
            "transcribe input and output must be different files",
        ));
    }
    Ok(())
}

fn normalized_parameters(request: &TranscribeRequest) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = BTreeMap::new();
    parameters.insert("decodeSampleRateHz".to_owned(), ASR_SAMPLE_RATE.into());
    parameters.insert("decodeChannels".to_owned(), ASR_CHANNELS.into());
    parameters.insert("decodeSampleFormat".to_owned(), "f32".into());
    parameters.insert("align".to_owned(), request.align.into());
    parameters.insert("model".to_owned(), request.model.id.clone().into());
    parameters.insert(
        "modelVersion".to_owned(),
        request
            .model
            .version
            .as_ref()
            .map_or(serde_json::Value::Null, |version| version.clone().into()),
    );
    parameters.insert(
        "backend".to_owned(),
        serde_json::to_value(request.model.backend).expect("backend is serializable"),
    );
    if request.align {
        parameters.insert("alignerModel".to_owned(), ALIGNER_MODEL_ID.into());
        parameters.insert(
            "alignerModelVersion".to_owned(),
            ALIGNER_RELEASE_VERSION.into(),
        );
    }
    parameters.insert(
        "lang".to_owned(),
        request
            .lang
            .as_ref()
            .map_or(serde_json::Value::Null, |lang| lang.clone().into()),
    );
    parameters.insert(
        "presentation".to_owned(),
        serde_json::to_value(&request.presentation).expect("presentation is serializable"),
    );
    parameters
}

fn write_json<T: Serialize>(
    target: &std::path::Path,
    overwrite: bool,
    value: &T,
) -> Result<(), ToolError> {
    let transaction = FileOutputTransaction::new(target, overwrite)?;
    let mut file = File::create(transaction.staging_path()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("create {}: {error}", transaction.staging_path().display()),
        )
    })?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("serialize {}: {error}", target.display()),
        )
    })?;
    file.write_all(b"\n").map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("write {}: {error}", target.display()),
        )
    })?;
    file.sync_all().map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("flush {}: {error}", target.display()),
        )
    })?;
    transaction.commit()?;
    Ok(())
}

fn quantize_word_times(transcript: &mut Transcript) -> Result<(), ToolError> {
    for word in &mut transcript.words {
        for time in [&mut word.start, &mut word.end] {
            if !time.is_finite() || *time < 0.0 {
                return Err(ToolError::new(
                    ToolErrorCode::InferenceFailed,
                    format!("ASR produced an invalid word time {time}"),
                ));
            }
            *time = (*time * 1_000.0).round() / 1_000.0;
        }
    }
    Ok(())
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

fn resolve_qwen_candidates(
    manager: &ModelManager,
    selection: ModelSelection,
    pinned_default_version: &str,
) -> Result<ModelSessionCandidates, ToolError> {
    let model_id = selection.id.clone();
    let preference = selection.backend;
    let version = selection
        .version
        .clone()
        .unwrap_or_else(|| pinned_default_version.to_owned());
    let candidates = manager
        .resolve_model_candidates(ResolveRequest {
            selection,
            allow_unverified: false,
        })
        .map_err(|error| {
            if matches!(
                error.code,
                ModelErrorCode::UnknownModel | ModelErrorCode::InvalidSelection
            ) {
                return ToolError::new(
                    ToolErrorCode::CatalogUnavailable,
                    format!(
                        "formal release {model_id}@{version} is not in the embedded or cached model catalog"
                    ),
                )
                .with_hint(format!(
                    "run: valle models list; install a supported version with valle models install {model_id} --backend auto"
                ));
            }
            ToolError::from(error)
        })?;
    Ok(ModelSessionCandidates::from_resolved(
        preference, candidates,
    ))
}

fn default_aligner_selection(backend: RunBackendPreference) -> ModelSelection {
    ModelSelection {
        id: ALIGNER_MODEL_ID.to_owned(),
        // `None` delegates to the Valle-owned catalog's pinned official upstream revision.
        version: None,
        backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_aligner_uses_the_embedded_catalog_version() {
        let selection = default_aligner_selection(RunBackendPreference::Auto);
        assert_eq!(selection.id, ALIGNER_MODEL_ID);
        assert_eq!(selection.version, None);
        assert_eq!(selection.backend, RunBackendPreference::Auto);
    }

    #[test]
    fn word_times_are_quantized_once_at_the_tool_boundary() {
        let mut transcript = Transcript {
            audio: None,
            lang: Some("en".to_owned()),
            words: vec![crate::analysis::Word {
                id: "w0".to_owned(),
                text: "hello".to_owned(),
                start: 0.123_51,
                end: 0.456_49,
            }],
        };
        quantize_word_times(&mut transcript).unwrap();
        assert_eq!(transcript.words[0].start, 0.124);
        assert_eq!(transcript.words[0].end, 0.456);
    }

    #[test]
    fn request_rejects_output_that_aliases_input() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"not decoded in validation").unwrap();
        let error = validate_request(&TranscribeRequest {
            input: input.clone(),
            output: Some(input),
            lang: None,
            align: true,
            presentation: TranscriptPresentation::WordJson,
            model: ModelSelection::pinned_default(ASR_MODEL_ID),
            overwrite: true,
        })
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidInput);
    }

    #[test]
    fn bounded_window_text_only_inserts_needed_ascii_spacing() {
        let mut text = String::new();
        append_transcript_text(&mut text, " hello ");
        append_transcript_text(&mut text, "world");
        append_transcript_text(&mut text, "。");
        append_transcript_text(&mut text, "你好");
        assert_eq!(text, "hello world。你好");
    }

    #[test]
    fn millisecond_ranges_round_outward_to_pcm_frames() {
        assert_eq!(frames_to_millis(16_000), 1_000);
        assert_eq!(millis_to_frames(1_000), 16_000);
        assert_eq!(millis_to_frames(1), 16);
    }

    #[test]
    fn missing_installed_qwen_artifact_never_falls_back_to_the_legacy_cache() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(ASR_MODEL_ID)).unwrap();
        let manager = ModelManager::from_models_root(root.path());
        let error = match resolve_qwen_candidates(
            &manager,
            ModelSelection::pinned_default(ASR_MODEL_ID),
            ASR_RELEASE_VERSION,
        ) {
            Ok(_) => panic!("an empty formal model store must not resolve"),
            Err(error) => error,
        };
        assert_eq!(error.code, ToolErrorCode::ModelNotInstalled);
        assert!(
            error
                .hint
                .as_deref()
                .is_some_and(|hint| { hint.contains("valle models install qwen3-asr-0.6b") })
        );
    }

    #[test]
    fn request_rejects_explicit_non_native_backend() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"validation fixture").unwrap();
        let error = validate_request(&TranscribeRequest {
            input,
            output: None,
            lang: None,
            align: false,
            presentation: TranscriptPresentation::None,
            model: ModelSelection {
                id: ASR_MODEL_ID.to_owned(),
                version: None,
                backend: RunBackendPreference::Onnx,
            },
            overwrite: false,
        })
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::NoCompatibleRoute);
        assert_eq!(error.hint.as_deref(), Some("use --backend auto"));
    }

    #[test]
    fn request_rejects_a_catalog_model_without_the_qwen_asr_contract() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"validation fixture").unwrap();
        let error = validate_request(&TranscribeRequest {
            input,
            output: None,
            lang: None,
            align: false,
            presentation: TranscriptPresentation::None,
            model: ModelSelection::pinned_default("birefnet"),
            overwrite: false,
        })
        .unwrap_err();

        assert_eq!(error.code, ToolErrorCode::UnsupportedAdapter);
    }

    #[test]
    fn normalized_parameters_include_the_requested_model_selection() {
        let request = TranscribeRequest {
            input: PathBuf::from("input.wav"),
            output: None,
            lang: Some("en".to_owned()),
            align: true,
            presentation: TranscriptPresentation::WordJson,
            model: ModelSelection {
                id: ASR_MODEL_ID.to_owned(),
                version: Some("1.2.3".to_owned()),
                backend: RunBackendPreference::Auto,
            },
            overwrite: false,
        };
        let parameters = normalized_parameters(&request);
        assert_eq!(parameters["decodeSampleRateHz"], ASR_SAMPLE_RATE);
        assert_eq!(parameters["decodeChannels"], ASR_CHANNELS);
        assert_eq!(parameters["decodeSampleFormat"], "f32");
        assert_eq!(parameters["model"], ASR_MODEL_ID);
        assert_eq!(parameters["modelVersion"], "1.2.3");
        assert_eq!(parameters["backend"], "auto");
        assert_eq!(parameters["alignerModel"], ALIGNER_MODEL_ID);
        assert_eq!(parameters["alignerModelVersion"], ALIGNER_RELEASE_VERSION);
    }
}
