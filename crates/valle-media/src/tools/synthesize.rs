//! Complete-file reference voice cloning with bounded decode and streamed WAV output.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use valle_tts::{AudioChunk, Language, ReferenceVoice, SynthesisOptions, SynthesisSummary};

use crate::{
    codec::{FloatWavWriter, LibavAudioStream, validate_float_wav},
    frame::AudioBuffer,
    models::{
        ModelSelection, RunBackendPreference,
        adapters::qwen_tts,
        inference::qwen_tts::{MAX_REFERENCE_SECONDS, SAMPLE_RATE_HZ},
    },
};

use super::{
    CancellationToken, MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia,
    ProcessingReport, ResourcePolicy, RunContext, StageTiming, ToolError, ToolErrorCode, ToolEvent,
    ToolPhase, ToolRun,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    probe_audio_input,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesizeRequest {
    pub text: String,
    pub reference: PathBuf,
    pub reference_text: Option<String>,
    pub output: PathBuf,
    pub lang: String,
    pub seed: i64,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_chunk_chars: usize,
    pub model: ModelSelection,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesizeResult {
    pub audio_seconds: f64,
    pub samples: u64,
    pub text_chunks: u64,
    pub outputs: Vec<OutputArtifact>,
}

pub fn run(
    request: SynthesizeRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<SynthesizeResult>, ToolError> {
    context.validate()?;
    let language = validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    check_cancelled(&context.cancellation)?;
    let started = Instant::now();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decode_started = Instant::now();
    let input_report = probe_audio_input(&request.reference)?;
    let reference = decode_reference(&request, &context.resources, &context.cancellation)?;
    let decode_seconds = decode_started.elapsed().as_secs_f64();
    check_cancelled(&context.cancellation)?;
    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open("load voice cloning model", qwen_tts::open)?;
    let mut session = opened.session;
    let load_seconds = load_started.elapsed().as_secs_f64();
    let options = SynthesisOptions {
        language,
        seed: request.seed,
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        max_chunk_chars: request.max_chunk_chars,
        cancellation: valle_tts::CancellationToken::from_shared_flag(
            context.cancellation.shared_flag(),
        ),
    };
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Inferencing));
    let inference_started = Instant::now();
    let (output, summary, encode_seconds) = synthesize_output(
        &request.output,
        request.overwrite,
        &context.resources,
        &context.cancellation,
        |sink| session.synthesize(&request.text, &reference, &options, sink),
    )?;
    let inference_seconds = (inference_started.elapsed().as_secs_f64() - encode_seconds).max(0.0);
    let audio_seconds = summary.samples as f64 / f64::from(summary.sample_rate);
    let outputs = vec![OutputArtifact {
        role: "synthesized_speech".to_owned(),
        path: output,
        media_type: "audio/wav".to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(audio_seconds),
                sample_rate_hz: Some(SAMPLE_RATE_HZ),
                channels: Some(1),
                sample_format: Some("f32".to_owned()),
                ..Default::default()
            }
            .with_audio_encoding("wav", "pcm_f32le"),
        ),
    }];
    let mut parameters = BTreeMap::new();
    parameters.insert("model".into(), request.model.id.into());
    parameters.insert("modelVersion".into(), request.model.version.into());
    parameters.insert("backend".into(), "auto".into());
    parameters.insert("text".into(), request.text.into());
    parameters.insert("referenceText".into(), request.reference_text.into());
    parameters.insert("language".into(), request.lang.into());
    parameters.insert("seed".into(), request.seed.into());
    parameters.insert("maxTokens".into(), request.max_tokens.into());
    parameters.insert("temperature".into(), request.temperature.into());
    parameters.insert("maxChunkChars".into(), request.max_chunk_chars.into());
    context.progress.event(ToolEvent::count(
        ToolPhase::Completed,
        summary.text_chunks,
        summary.text_chunks,
    ));
    Ok(ToolRun {
        result: SynthesizeResult {
            audio_seconds,
            samples: summary.samples,
            text_chunks: summary.text_chunks,
            outputs: outputs.clone(),
        },
        report: ProcessingReport {
            operation: "synthesize".into(),
            parameters,
            input: request.reference,
            input_media_type: input_report.media_type,
            input_summary: input_report.summary,
            outputs,
            models: vec![ModelProvenance::from(&opened.model.resolved)],
            processed: ProcessedMedia {
                frames: 0,
                audio_seconds,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds: 0.0,
                inference_seconds,
                postprocess_seconds: 0.0,
                encode_seconds,
                total_seconds: started.elapsed().as_secs_f64(),
            },
        },
        warnings: opened.warnings,
    })
}

fn validate_request(request: &SynthesizeRequest) -> Result<Language, ToolError> {
    if request.text.trim().is_empty() || request.text.contains('\0') {
        return Err(ToolError::invalid_input(
            "synthesis text must be non-empty and contain no NUL characters",
        ));
    }
    if let Some(text) = &request.reference_text
        && (text.trim().is_empty() || text.contains('\0'))
    {
        return Err(ToolError::invalid_input(
            "reference transcript must be non-empty and contain no NUL characters",
        ));
    }
    if !request.reference.is_file() {
        return Err(ToolError::invalid_input(format!(
            "reference audio is missing: {}",
            request.reference.display()
        )));
    }
    if paths_refer_to_same_file(&request.reference, &request.output) {
        return Err(ToolError::invalid_input(
            "reference audio and synthesis output must be different files",
        ));
    }
    if !request
        .output
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
    {
        return Err(ToolError::invalid_input(
            "synthesis output must use the .wav extension",
        ));
    }
    if request.model.backend != RunBackendPreference::Auto {
        return Err(ToolError::new(
            ToolErrorCode::NoCompatibleRoute,
            "voice cloning uses its native CPU route; select backend auto",
        )
        .with_hint("use --backend auto"));
    }
    if !(1..=4096).contains(&request.max_tokens)
        || !(1..=500).contains(&request.max_chunk_chars)
        || !request.temperature.is_finite()
        || !(0.0..=2.0).contains(&request.temperature)
    {
        return Err(ToolError::invalid_input(
            "invalid synthesis token budget, chunk size or temperature",
        ));
    }
    Language::parse(&request.lang).map_err(|error| ToolError::invalid_input(error.to_string()))
}

fn decode_reference(
    request: &SynthesizeRequest,
    resources: &ResourcePolicy,
    cancellation: &CancellationToken,
) -> Result<ReferenceVoice, ToolError> {
    let mut decoder = LibavAudioStream::open(&request.reference, SAMPLE_RATE_HZ, 1)
        .map_err(|error| ToolError::invalid_input(format!("decode reference audio: {error:#}")))?;
    let max_samples = SAMPLE_RATE_HZ as usize * MAX_REFERENCE_SECONDS as usize;
    let mut samples = Vec::new();
    loop {
        check_cancelled(cancellation)?;
        let remaining = max_samples + 1 - samples.len();
        let chunk = decoder.read(remaining.min(16_384)).map_err(|error| {
            ToolError::invalid_input(format!("decode reference audio: {error:#}"))
        })?;
        if chunk.samples.is_empty() {
            break;
        }
        if (samples.len() + chunk.samples.len()) as u64 * 4 > resources.audio_memory_budget_bytes {
            return Err(ToolError::invalid_input(
                "reference audio exceeds the job audio memory budget",
            ));
        }
        samples.extend(chunk.samples);
        if samples.len() > max_samples {
            return Err(ToolError::invalid_input(
                "reference audio must be at most 15 seconds",
            ));
        }
    }
    ReferenceVoice::new(samples, SAMPLE_RATE_HZ, request.reference_text.clone())
        .map_err(|error| ToolError::invalid_input(error.to_string()))
}

fn synthesize_output(
    output: &Path,
    overwrite: bool,
    resources: &ResourcePolicy,
    cancellation: &CancellationToken,
    synthesize: impl FnOnce(
        &mut (dyn FnMut(AudioChunk) -> Result<()> + Send),
    ) -> Result<SynthesisSummary>,
) -> Result<(PathBuf, SynthesisSummary, f64), ToolError> {
    check_cancelled(cancellation)?;
    let transaction = FileOutputTransaction::new(output, overwrite)?;
    let mut writer = FloatWavWriter::create(transaction.staging_path(), SAMPLE_RATE_HZ, 1)
        .map_err(|error| {
            ToolError::new(ToolErrorCode::OutputValidationFailed, error.to_string())
        })?;
    let mut write_error = None;
    let mut encode_seconds = 0.0;
    let summary = synthesize(&mut |chunk| {
        let started = Instant::now();
        if cancellation.is_cancelled() {
            return Err(valle_tts::Cancelled.into());
        }
        if chunk.samples.len() as u64 * 4 > resources.audio_memory_budget_bytes {
            let error =
                ToolError::invalid_input("synthesis chunk exceeds the job audio memory budget");
            write_error = Some(error.clone());
            return Err(anyhow::anyhow!(error.message));
        }
        let result = writer.write(&AudioBuffer {
            samples: chunk.samples,
            sample_rate: chunk.sample_rate,
            channels: 1,
        });
        encode_seconds += started.elapsed().as_secs_f64();
        if let Err(error) = result {
            write_error = Some(ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                error.to_string(),
            ));
            return Err(error);
        }
        Ok(())
    })
    .map_err(|error| {
        if cancellation.is_cancelled() || error.is::<valle_tts::Cancelled>() {
            ToolError::new(ToolErrorCode::Cancelled, "speech synthesis cancelled")
        } else if let Some(error) = write_error {
            error
        } else {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!("synthesize speech: {error:#}"),
            )
        }
    })?;
    check_cancelled(cancellation)?;
    if summary.sample_rate != SAMPLE_RATE_HZ
        || summary.samples == 0
        || summary.samples != writer.frames_written()
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            "synthesis summary does not match the streamed audio",
        ));
    }
    let final_started = Instant::now();
    writer.finish().map_err(|error| {
        ToolError::new(ToolErrorCode::OutputValidationFailed, error.to_string())
    })?;
    validate_float_wav(
        transaction.staging_path(),
        SAMPLE_RATE_HZ,
        1,
        summary.samples,
    )
    .map_err(|error| ToolError::new(ToolErrorCode::OutputValidationFailed, error.to_string()))?;
    check_cancelled(cancellation)?;
    let output = transaction.commit()?;
    encode_seconds += final_started.elapsed().as_secs_f64();
    Ok((output, summary, encode_seconds))
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorCode::Cancelled,
            "speech synthesis cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(samples: u64) -> SynthesisSummary {
        SynthesisSummary {
            model: "qwen3-tts-0.6b-base-q8".into(),
            sample_rate: SAMPLE_RATE_HZ,
            samples,
            text_chunks: 2,
        }
    }

    fn chunk(samples: Vec<f32>) -> AudioChunk {
        AudioChunk {
            samples,
            sample_rate: SAMPLE_RATE_HZ,
        }
    }

    #[test]
    fn publishes_streamed_audio_only_after_the_summary_and_wav_validate() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("speech.wav");
        let cancellation = CancellationToken::new();
        let (_, result, _) = synthesize_output(
            &output,
            false,
            &ResourcePolicy::default(),
            &cancellation,
            |sink| {
                sink(chunk(vec![0.1; 120]))?;
                assert!(!output.exists(), "partial synthesis must remain private");
                sink(chunk(vec![-0.2; 240]))?;
                Ok(summary(360))
            },
        )
        .unwrap();
        assert_eq!(result.samples, 360);
        validate_float_wav(&output, SAMPLE_RATE_HZ, 1, 360).unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancellation_after_a_chunk_preserves_the_old_output_and_cleans_staging() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("speech.wav");
        std::fs::write(&output, b"old output").unwrap();
        let cancellation = CancellationToken::new();
        let error = synthesize_output(
            &output,
            true,
            &ResourcePolicy::default(),
            &cancellation,
            |sink| {
                sink(chunk(vec![0.1; 120]))?;
                cancellation.cancel();
                sink(chunk(vec![0.1; 120]))?;
                Ok(summary(240))
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::Cancelled);
        assert_eq!(std::fs::read(&output).unwrap(), b"old output");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_inference_invalid_samples_and_wrong_summaries_never_publish() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("speech.wav");
        for mode in 0..3 {
            let error = synthesize_output(
                &output,
                false,
                &ResourcePolicy::default(),
                &CancellationToken::new(),
                |sink| {
                    sink(chunk(vec![0.1; 120]))?;
                    match mode {
                        0 => anyhow::bail!("model failed after its first chunk"),
                        1 => {
                            sink(chunk(vec![f32::NAN]))?;
                            Ok(summary(121))
                        }
                        _ => Ok(summary(240)),
                    }
                },
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                if mode == 0 {
                    ToolErrorCode::InferenceFailed
                } else {
                    ToolErrorCode::OutputValidationFailed
                }
            );
            assert!(!output.exists());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn streamed_chunk_memory_is_bounded_by_the_job_policy() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("speech.wav");
        let resources = ResourcePolicy {
            audio_memory_budget_bytes: 16,
            ..Default::default()
        };
        let error = synthesize_output(
            &output,
            false,
            &resources,
            &CancellationToken::new(),
            |sink| {
                sink(chunk(vec![0.1; 5]))?;
                Ok(summary(5))
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    #[ignore = "requires configured FFmpeg shared libraries"]
    fn reference_decode_accepts_short_audio_and_rejects_overlong_or_over_budget_audio() {
        let root = tempfile::tempdir().unwrap();
        let reference = root.path().join("reference.wav");
        let request = SynthesizeRequest {
            text: "Hello".into(),
            reference: reference.clone(),
            reference_text: None,
            output: root.path().join("speech.wav"),
            lang: "en".into(),
            seed: 42,
            max_tokens: 512,
            temperature: 0.9,
            max_chunk_chars: 160,
            model: ModelSelection {
                id: "qwen3-tts-0.6b-base-q8".into(),
                version: None,
                backend: RunBackendPreference::Auto,
            },
            overwrite: false,
        };
        for seconds in [1, 16] {
            let mut writer = FloatWavWriter::create(&reference, SAMPLE_RATE_HZ, 1).unwrap();
            writer
                .write(&AudioBuffer {
                    samples: vec![0.1; SAMPLE_RATE_HZ as usize * seconds],
                    sample_rate: SAMPLE_RATE_HZ,
                    channels: 1,
                })
                .unwrap();
            writer.finish().unwrap();
            let result = decode_reference(
                &request,
                &ResourcePolicy::default(),
                &CancellationToken::new(),
            );
            if seconds == 1 {
                assert!(result.is_ok());
                let policy = ResourcePolicy {
                    audio_memory_budget_bytes: 1024,
                    ..Default::default()
                };
                assert_eq!(
                    decode_reference(&request, &policy, &CancellationToken::new())
                        .unwrap_err()
                        .code,
                    ToolErrorCode::InvalidInput
                );
            } else {
                assert!(result.unwrap_err().message.contains("at most 15 seconds"));
            }
        }
    }
}
