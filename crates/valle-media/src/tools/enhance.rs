//! Streaming file workflow for DPDFNet speech enhancement.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(test)]
use anyhow::Result as AnyResult;
use serde::{Deserialize, Serialize};

use crate::{
    codec::{
        FLAC_BITS_PER_SAMPLE, FLAC_QUANTIZATION_POLICY, FlacPcm24Writer, FloatWavWriter,
        LibavAudioStream, validate_flac_pcm24, validate_float_wav,
    },
    models::{
        ModelSelection,
        adapters::dpdfnet::{
            CHANNELS, EnhancementSession, EnhancementStats, FinishedEnhancement, MODEL_ID,
            PcmEnhancementStream, SAMPLE_RATE,
        },
    },
};

use super::{
    MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport, RunContext,
    StageTiming, ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun, ToolWarning,
    model_session::ModelSessionCandidates,
    output::{FileOutputTransaction, paths_refer_to_same_file},
    probe_audio_input,
};

const STREAM_CHUNK_FRAMES: usize = SAMPLE_RATE as usize;
const MIN_AUDIO_MEMORY_BUDGET_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnhanceRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub model: ModelSelection,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceResult {
    pub outputs: Vec<OutputArtifact>,
    pub input_samples: u64,
    pub output_samples: u64,
    pub audio_seconds: f64,
    pub processing_seconds: f64,
    pub real_time_factor: f64,
    pub real_time_multiple: f64,
    /// Maximum adapter-owned per-job heap capacity observed at chunk boundaries.
    ///
    /// This excludes the loaded model/session, decoder, encoder and returned PCM chunks.
    pub stream_working_set_high_watermark_bytes: usize,
}

#[derive(Debug)]
struct PipelineResult {
    stats: EnhancementStats,
    decode_seconds: f64,
    encode_seconds: f64,
    stream_working_set_high_watermark_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnhanceOutputFormat {
    FloatWav,
    FlacPcm24,
}

impl EnhanceOutputFormat {
    fn from_path(path: &Path) -> Result<Self, ToolError> {
        match path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("wav") => Ok(Self::FloatWav),
            Some("flac") => Ok(Self::FlacPcm24),
            _ => Err(ToolError::invalid_input(
                "enhance output must end in .wav (float32 PCM) or .flac (signed 24-bit PCM)",
            )),
        }
    }

    const fn media_type(self) -> &'static str {
        match self {
            Self::FloatWav => "audio/wav",
            Self::FlacPcm24 => "audio/flac",
        }
    }

    const fn format_name(self) -> &'static str {
        match self {
            Self::FloatWav => "wav",
            Self::FlacPcm24 => "flac",
        }
    }

    const fn sample_format(self) -> &'static str {
        match self {
            Self::FloatWav => "f32",
            Self::FlacPcm24 => "s24",
        }
    }

    const fn codec_name(self) -> &'static str {
        match self {
            Self::FloatWav => "pcm_f32le",
            Self::FlacPcm24 => "flac",
        }
    }

    fn warnings(self) -> Vec<ToolWarning> {
        match self {
            Self::FloatWav => Vec::new(),
            Self::FlacPcm24 => vec![ToolWarning {
                code: "audio_quantized_pcm24".to_owned(),
                message: format!(
                    "FLAC cannot preserve model f32 samples bit-for-bit; output uses {FLAC_QUANTIZATION_POLICY}, then FLAC losslessly preserves those PCM24 values"
                ),
            }],
        }
    }
}

enum EnhanceAudioWriter {
    FloatWav(FloatWavWriter),
    FlacPcm24(FlacPcm24Writer),
}

impl EnhanceAudioWriter {
    fn create(path: &Path, output_format: EnhanceOutputFormat) -> Result<Self, ToolError> {
        match output_format {
            EnhanceOutputFormat::FloatWav => {
                FloatWavWriter::create(path, SAMPLE_RATE, CHANNELS)
                    .map(Self::FloatWav)
                    .map_err(|error| {
                        ToolError::new(
                            ToolErrorCode::OutputValidationFailed,
                            format!("create {}: {error:#}", path.display()),
                        )
                    })
            }
            EnhanceOutputFormat::FlacPcm24 => {
                FlacPcm24Writer::create(path, SAMPLE_RATE, CHANNELS)
                    .map(Self::FlacPcm24)
                    .map_err(|error| {
                        ToolError::new(
                            ToolErrorCode::RuntimeUnavailable,
                            format!(
                                "create native FLAC output {}: {error:#}",
                                path.display()
                            ),
                        )
                        .with_hint(
                            "install or bundle a libav build with the native FLAC encoder, or choose a .wav output",
                        )
                    })
            }
        }
    }

    fn write(&mut self, audio: &crate::frame::AudioBuffer) -> anyhow::Result<()> {
        match self {
            Self::FloatWav(writer) => writer.write(audio),
            Self::FlacPcm24(writer) => writer.write(audio),
        }
    }

    fn finish(self) -> anyhow::Result<u64> {
        match self {
            Self::FloatWav(writer) => writer.finish(),
            Self::FlacPcm24(writer) => writer.finish(),
        }
    }
}

pub fn run(
    request: EnhanceRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<EnhanceResult>, ToolError> {
    context.validate()?;
    let output_format = validate_request(&request)?;
    FileOutputTransaction::validate_target(&request.output, request.overwrite)?;
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let transaction = FileOutputTransaction::new(&request.output, request.overwrite)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decode_started = Instant::now();
    let input_report = probe_audio_input(&request.input)?;
    let mut source =
        LibavAudioStream::open(&request.input, SAMPLE_RATE, CHANNELS).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InvalidInput,
                format!("open {} as audio: {error:#}", request.input.display()),
            )
        })?;
    let source_open_seconds = decode_started.elapsed().as_secs_f64();
    check_cancelled(context)?;

    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let opened = candidates.open(&format!("load {}", request.model.id), |model| {
        EnhancementSession::open(model, context.resources.cpu_threads)
    })?;
    let mut session = opened.session;
    let load_seconds = load_started.elapsed().as_secs_f64();
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let mut warnings = opened.warnings;
    warnings.extend(output_format.warnings());
    check_cancelled(context)?;

    let stream = session.start_stream();
    let mut pipeline = process_stream(
        &mut source,
        transaction.staging_path(),
        output_format,
        stream,
        context,
    )?;
    pipeline.decode_seconds += source_open_seconds;
    let total_samples = source.total_frames().ok_or_else(|| {
        ToolError::new(
            ToolErrorCode::Internal,
            "enhancement decoder finished without an exact frame count",
        )
    })?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    check_cancelled(context)?;
    if pipeline.stats.input_samples != total_samples
        || pipeline.stats.output_samples != total_samples
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "enhancement length mismatch: source={total_samples}, adapter input={}, adapter output={}",
                pipeline.stats.input_samples, pipeline.stats.output_samples
            ),
        ));
    }
    validate_staged_output(transaction.staging_path(), output_format, total_samples).map_err(
        |error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!(
                    "validate staged {} {}: {error:#}",
                    output_format.format_name().to_ascii_uppercase(),
                    transaction.staging_path().display()
                ),
            )
        },
    )?;
    check_cancelled(context)?;
    let output_path = transaction.commit()?;
    let audio_seconds = total_samples as f64 / f64::from(SAMPLE_RATE);
    let output_artifact = OutputArtifact {
        role: "enhanced_audio".to_owned(),
        path: output_path,
        media_type: output_format.media_type().to_owned(),
        summary: Some(
            MediaSummary {
                duration_seconds: Some(audio_seconds),
                sample_rate_hz: Some(SAMPLE_RATE),
                channels: Some(CHANNELS),
                sample_format: Some(output_format.sample_format().to_owned()),
                ..MediaSummary::default()
            }
            .with_audio_encoding(output_format.format_name(), output_format.codec_name()),
        ),
    };
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));

    Ok(ToolRun {
        result: EnhanceResult {
            outputs: vec![output_artifact.clone()],
            input_samples: total_samples,
            output_samples: pipeline.stats.output_samples,
            audio_seconds,
            processing_seconds: pipeline.stats.processing_seconds,
            real_time_factor: pipeline.stats.real_time_factor,
            real_time_multiple: pipeline.stats.real_time_multiple,
            stream_working_set_high_watermark_bytes: pipeline
                .stream_working_set_high_watermark_bytes,
        },
        report: ProcessingReport {
            operation: "enhance".to_owned(),
            parameters: normalized_parameters(&request, output_format),
            input: request.input,
            input_media_type: input_report.media_type,
            input_summary: input_report.summary,
            outputs: vec![output_artifact],
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: 0,
                audio_seconds,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds: pipeline.decode_seconds,
                preprocess_seconds: pipeline.stats.dsp_seconds,
                inference_seconds: pipeline.stats.inference_seconds,
                postprocess_seconds: 0.0,
                encode_seconds: pipeline.encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

fn process_stream<S: PcmEnhancementStream>(
    source: &mut LibavAudioStream,
    output: &Path,
    output_format: EnhanceOutputFormat,
    mut stream: S,
    context: &mut RunContext<'_>,
) -> Result<PipelineResult, ToolError> {
    let mut stream_working_set_high_watermark_bytes = 0;
    observe_stream_working_set(
        &stream,
        context,
        &mut stream_working_set_high_watermark_bytes,
    )?;

    let encode_started = Instant::now();
    let mut writer = EnhanceAudioWriter::create(output, output_format)?;
    let mut encode_seconds = encode_started.elapsed().as_secs_f64();
    let frame_count_hint = source.frame_count_hint();
    let mut decoded_samples = 0_u64;
    let mut emitted_samples = 0_u64;
    let mut decode_seconds = 0.0;

    loop {
        check_cancelled(context)?;
        emit_audio_progress(
            context,
            ToolPhase::Decoding,
            source.position(),
            frame_count_hint,
        );
        let stage = Instant::now();
        let input = source.read(STREAM_CHUNK_FRAMES).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InvalidInput,
                format!("decode audio at sample {}: {error:#}", source.position()),
            )
        })?;
        decode_seconds += stage.elapsed().as_secs_f64();
        if input.frames() == 0 {
            break;
        }
        decoded_samples = decoded_samples
            .checked_add(input.frames() as u64)
            .ok_or_else(|| {
                ToolError::new(ToolErrorCode::Internal, "decoded sample count overflow")
            })?;

        emit_audio_progress(
            context,
            ToolPhase::Inferencing,
            decoded_samples,
            frame_count_hint,
        );
        let enhanced = stream.push(&input).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InferenceFailed,
                format!("enhance audio at sample {}: {error:#}", decoded_samples),
            )
        })?;
        observe_stream_working_set(
            &stream,
            context,
            &mut stream_working_set_high_watermark_bytes,
        )?;
        emitted_samples = emitted_samples
            .checked_add(enhanced.frames() as u64)
            .ok_or_else(|| {
                ToolError::new(ToolErrorCode::Internal, "output sample count overflow")
            })?;

        emit_audio_progress(
            context,
            ToolPhase::Encoding,
            emitted_samples,
            frame_count_hint,
        );
        let stage = Instant::now();
        writer.write(&enhanced).map_err(|error| {
            ToolError::new(
                ToolErrorCode::OutputValidationFailed,
                format!("write {}: {error:#}", output.display()),
            )
        })?;
        encode_seconds += stage.elapsed().as_secs_f64();
    }

    check_cancelled(context)?;
    let total = source.total_frames().ok_or_else(|| {
        ToolError::new(
            ToolErrorCode::Internal,
            "audio decoder reached EOF without an exact frame count",
        )
    })?;
    let FinishedEnhancement { audio: tail, stats } = stream.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::InferenceFailed,
            format!("finish enhanced audio: {error:#}"),
        )
    })?;
    emitted_samples = emitted_samples
        .checked_add(tail.frames() as u64)
        .ok_or_else(|| ToolError::new(ToolErrorCode::Internal, "output sample count overflow"))?;
    let stage = Instant::now();
    writer.write(&tail).map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("write final {} samples: {error:#}", output.display()),
        )
    })?;
    if decoded_samples != total
        || stats.input_samples != decoded_samples
        || stats.output_samples != emitted_samples
    {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "stream length mismatch: source={total}, decoded={decoded_samples}, adapter input={}, emitted={emitted_samples}, adapter output={}",
                stats.input_samples, stats.output_samples
            ),
        ));
    }
    let output_frames = writer.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!("finish {}: {error:#}", output.display()),
        )
    })?;
    encode_seconds += stage.elapsed().as_secs_f64();
    if output_frames != emitted_samples {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "{} length mismatch: wrote {output_frames} frames, expected {emitted_samples}",
                output_format.format_name().to_ascii_uppercase()
            ),
        ));
    }
    Ok(PipelineResult {
        stats,
        decode_seconds,
        encode_seconds,
        stream_working_set_high_watermark_bytes,
    })
}

fn observe_stream_working_set(
    stream: &impl PcmEnhancementStream,
    context: &RunContext<'_>,
    high_watermark: &mut usize,
) -> Result<(), ToolError> {
    let current = stream.working_set_bytes();
    *high_watermark = (*high_watermark).max(current);
    let required_budget =
        MIN_AUDIO_MEMORY_BUDGET_BYTES.saturating_add(u64::try_from(current).unwrap_or(u64::MAX));
    if context.resources.audio_memory_budget_bytes < required_budget {
        return Err(ToolError::new(
            ToolErrorCode::ResourceBusy,
            format!(
                "audio memory budget is too small for streaming enhancement (observed adapter working set {} bytes; need at least {required_budget} bytes)",
                *high_watermark
            ),
        ));
    }
    Ok(())
}

fn emit_audio_progress(
    context: &mut RunContext<'_>,
    phase: ToolPhase,
    completed: u64,
    frame_count_hint: Option<u64>,
) {
    context.progress.event(ToolEvent {
        phase,
        completed: Some(completed),
        // A short VBR hint must not make progress exceed its total. Growing the display total does
        // not turn the hint into a decoding boundary.
        total: frame_count_hint.map(|total| total.max(completed)),
        message: None,
    });
}

fn validate_request(request: &EnhanceRequest) -> Result<EnhanceOutputFormat, ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    let output_format = EnhanceOutputFormat::from_path(&request.output)?;
    if paths_refer_to_same_file(&request.input, &request.output) {
        return Err(ToolError::invalid_input(
            "enhance input and output must be different files",
        ));
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "enhance supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    Ok(output_format)
}

fn normalized_parameters(
    request: &EnhanceRequest,
    output_format: EnhanceOutputFormat,
) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = BTreeMap::new();
    parameters.insert("decodeSampleRateHz".to_owned(), SAMPLE_RATE.into());
    parameters.insert("decodeChannels".to_owned(), CHANNELS.into());
    parameters.insert("decodeSampleFormat".to_owned(), "f32".into());
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
    parameters.insert(
        "outputFormat".to_owned(),
        output_format.format_name().into(),
    );
    parameters.insert(
        "outputSampleFormat".to_owned(),
        output_format.sample_format().into(),
    );
    if output_format == EnhanceOutputFormat::FlacPcm24 {
        parameters.insert(
            "outputBitsPerSample".to_owned(),
            FLAC_BITS_PER_SAMPLE.into(),
        );
        parameters.insert(
            "floatToPcmPolicy".to_owned(),
            FLAC_QUANTIZATION_POLICY.into(),
        );
    }
    parameters
}

fn validate_staged_output(
    path: &Path,
    output_format: EnhanceOutputFormat,
    expected_frames: u64,
) -> anyhow::Result<()> {
    match output_format {
        EnhanceOutputFormat::FloatWav => {
            validate_float_wav(path, SAMPLE_RATE, CHANNELS, expected_frames)
        }
        EnhanceOutputFormat::FlacPcm24 => {
            validate_flac_pcm24(path, SAMPLE_RATE, CHANNELS, expected_frames)
        }
    }
}

fn check_cancelled(context: &RunContext<'_>) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::cancelled())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;
    use crate::{frame::AudioBuffer, models::ModelManager, tools::NoopProgress};

    struct IdentityStream {
        samples: u64,
        peak_chunk: Rc<Cell<usize>>,
    }

    impl PcmEnhancementStream for IdentityStream {
        fn push(&mut self, audio: &AudioBuffer) -> AnyResult<AudioBuffer> {
            self.samples += audio.frames() as u64;
            self.peak_chunk
                .set(self.peak_chunk.get().max(audio.frames()));
            Ok(audio.clone())
        }

        fn finish(self) -> AnyResult<FinishedEnhancement> {
            let duration = self.samples as f64 / f64::from(SAMPLE_RATE);
            Ok(FinishedEnhancement {
                audio: AudioBuffer::silence(SAMPLE_RATE, CHANNELS, 0),
                stats: EnhancementStats {
                    input_samples: self.samples,
                    output_samples: self.samples,
                    processing_seconds: 0.0,
                    inference_seconds: 0.0,
                    dsp_seconds: 0.0,
                    real_time_factor: 0.0,
                    real_time_multiple: if duration > 0.0 { 1.0 } else { 0.0 },
                },
            })
        }

        fn working_set_bytes(&self) -> usize {
            4_096
        }
    }

    struct GrowingStream {
        samples: u64,
        working_set_bytes: usize,
    }

    impl PcmEnhancementStream for GrowingStream {
        fn push(&mut self, audio: &AudioBuffer) -> AnyResult<AudioBuffer> {
            self.samples += audio.frames() as u64;
            self.working_set_bytes = 2 * 1024 * 1024;
            Ok(audio.clone())
        }

        fn finish(self) -> AnyResult<FinishedEnhancement> {
            unreachable!("the growing stream must be rejected before finish")
        }

        fn working_set_bytes(&self) -> usize {
            self.working_set_bytes
        }
    }

    struct CancellingStream {
        samples: u64,
        cancellation: super::super::CancellationToken,
    }

    impl PcmEnhancementStream for CancellingStream {
        fn push(&mut self, audio: &AudioBuffer) -> AnyResult<AudioBuffer> {
            self.samples += audio.frames() as u64;
            self.cancellation.cancel();
            Ok(audio.clone())
        }

        fn finish(self) -> AnyResult<FinishedEnhancement> {
            unreachable!("the cancelled stream must not be finished")
        }

        fn working_set_bytes(&self) -> usize {
            4_096
        }
    }

    fn write_synthetic_wav(path: &Path, frames: usize) {
        let mut writer = FloatWavWriter::create(path, SAMPLE_RATE, CHANNELS).unwrap();
        let chunk = AudioBuffer {
            samples: (0..STREAM_CHUNK_FRAMES)
                .map(|index| ((index as f32 * 0.013).sin() * 0.2).clamp(-1.0, 1.0))
                .collect(),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
        };
        let mut remaining = frames;
        while remaining > 0 {
            let count = remaining.min(STREAM_CHUNK_FRAMES);
            writer
                .write(&AudioBuffer {
                    samples: chunk.samples[..count].to_vec(),
                    sample_rate: SAMPLE_RATE,
                    channels: CHANNELS,
                })
                .unwrap();
            remaining -= count;
        }
        assert_eq!(writer.finish().unwrap(), frames as u64);
    }

    #[test]
    fn synthetic_wav_pipeline_streams_and_preserves_exact_length() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let output = root.path().join("enhanced.wav");
        let samples = SAMPLE_RATE as usize * 2 + 137;
        let source = AudioBuffer {
            samples: (0..samples)
                .map(|index| ((index as f32 * 0.013).sin() * 0.2).clamp(-1.0, 1.0))
                .collect(),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
        };
        let mut input_writer = FloatWavWriter::create(&input, SAMPLE_RATE, CHANNELS).unwrap();
        input_writer.write(&source).unwrap();
        assert_eq!(input_writer.finish().unwrap(), samples as u64);

        let mut decoder = LibavAudioStream::open(&input, SAMPLE_RATE, CHANNELS).unwrap();
        let transaction = FileOutputTransaction::new(&output, false).unwrap();
        let manager = ModelManager::from_models_root(root.path().join("models"));
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        let peak_chunk = Rc::new(Cell::new(0));
        let result = process_stream(
            &mut decoder,
            transaction.staging_path(),
            EnhanceOutputFormat::FloatWav,
            IdentityStream {
                samples: 0,
                peak_chunk: Rc::clone(&peak_chunk),
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(result.stats.input_samples, samples as u64);
        assert_eq!(result.stats.output_samples, samples as u64);
        assert_eq!(peak_chunk.get(), STREAM_CHUNK_FRAMES);
        assert_eq!(result.stream_working_set_high_watermark_bytes, 4_096);
        transaction.commit().unwrap();

        let mut decoded_output = LibavAudioStream::open(&output, SAMPLE_RATE, CHANNELS).unwrap();
        let mut decoded_frames = 0_u64;
        loop {
            let chunk = decoded_output.read(997).unwrap();
            if chunk.frames() == 0 {
                break;
            }
            decoded_frames += chunk.frames() as u64;
        }
        assert_eq!(decoded_frames, samples as u64);
        assert_eq!(decoded_output.total_frames(), Some(samples as u64));
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            samples as u32 * 4
        );
    }

    #[test]
    fn synthetic_flac_pipeline_streams_preserves_length_and_reports_quantization() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let output = root.path().join("enhanced.flac");
        let samples = SAMPLE_RATE as usize + 137;
        let source = AudioBuffer {
            samples: (0..samples)
                .map(|index| ((index as f32 * 0.017).sin() * 0.25).clamp(-1.0, 1.0))
                .collect(),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
        };
        let mut input_writer = FloatWavWriter::create(&input, SAMPLE_RATE, CHANNELS).unwrap();
        input_writer.write(&source).unwrap();
        input_writer.finish().unwrap();

        let mut decoder = LibavAudioStream::open(&input, SAMPLE_RATE, CHANNELS).unwrap();
        let transaction = FileOutputTransaction::new(&output, false).unwrap();
        let manager = ModelManager::from_models_root(root.path().join("models"));
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        let peak_chunk = Rc::new(Cell::new(0));
        let result = process_stream(
            &mut decoder,
            transaction.staging_path(),
            EnhanceOutputFormat::FlacPcm24,
            IdentityStream {
                samples: 0,
                peak_chunk: Rc::clone(&peak_chunk),
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(result.stats.input_samples, samples as u64);
        assert_eq!(result.stats.output_samples, samples as u64);
        assert_eq!(peak_chunk.get(), STREAM_CHUNK_FRAMES);
        assert_eq!(result.stream_working_set_high_watermark_bytes, 4_096);
        validate_staged_output(
            transaction.staging_path(),
            EnhanceOutputFormat::FlacPcm24,
            samples as u64,
        )
        .unwrap();
        transaction.commit().unwrap();

        let mut decoded_output = LibavAudioStream::open(&output, SAMPLE_RATE, CHANNELS).unwrap();
        let mut decoded_frames = 0_u64;
        loop {
            let chunk = decoded_output.read(997).unwrap();
            if chunk.frames() == 0 {
                break;
            }
            decoded_frames += chunk.frames() as u64;
        }
        assert_eq!(decoded_frames, samples as u64);
        assert_eq!(decoded_output.total_frames(), Some(samples as u64));
        assert_eq!(
            EnhanceOutputFormat::FlacPcm24.warnings()[0].code,
            "audio_quantized_pcm24"
        );
    }

    #[test]
    fn request_requires_a_distinct_supported_audio_output() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"fixture").unwrap();
        let request = EnhanceRequest {
            input: input.clone(),
            output: input.clone(),
            model: ModelSelection::pinned_default("dpdfnet"),
            overwrite: true,
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
        let request = EnhanceRequest {
            output: root.path().join("output.FLAC"),
            ..request
        };
        assert_eq!(
            validate_request(&request).unwrap(),
            EnhanceOutputFormat::FlacPcm24
        );
        let request = EnhanceRequest {
            output: root.path().join("output.mp3"),
            ..request
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
        let request = EnhanceRequest {
            output: root.path().join("output.wav"),
            model: ModelSelection::pinned_default("birefnet"),
            ..request
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::UnsupportedAdapter
        );
    }

    #[test]
    fn growing_stream_is_rejected_at_a_chunk_boundary_and_staging_is_cleaned() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let output = root.path().join("enhanced.wav");
        write_synthetic_wav(&input, STREAM_CHUNK_FRAMES * 2);
        std::fs::write(&output, b"existing target must survive resource failure").unwrap();
        let mut decoder = LibavAudioStream::open(&input, SAMPLE_RATE, CHANNELS).unwrap();
        let transaction = FileOutputTransaction::new(&output, true).unwrap();
        let staging = transaction.staging_path().to_owned();
        let manager = ModelManager::from_models_root(root.path().join("models"));
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.audio_memory_budget_bytes = MIN_AUDIO_MEMORY_BUDGET_BYTES + 8_192;

        let error = process_stream(
            &mut decoder,
            &staging,
            EnhanceOutputFormat::FloatWav,
            GrowingStream {
                samples: 0,
                working_set_bytes: 4_096,
            },
            &mut context,
        )
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::ResourceBusy);
        assert!(
            error
                .message
                .contains("observed adapter working set 2097152")
        );
        assert!(staging.is_file());
        drop(transaction);
        assert!(!staging.exists());
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"existing target must survive resource failure"
        );
    }

    #[test]
    fn cancellation_during_a_chunk_removes_the_staged_audio_output() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        let output = root.path().join("enhanced.wav");
        write_synthetic_wav(&input, STREAM_CHUNK_FRAMES * 2);
        std::fs::write(&output, b"existing target must survive cancellation").unwrap();
        let mut decoder = LibavAudioStream::open(&input, SAMPLE_RATE, CHANNELS).unwrap();
        let transaction = FileOutputTransaction::new(&output, true).unwrap();
        let staging = transaction.staging_path().to_owned();
        let manager = ModelManager::from_models_root(root.path().join("models"));
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        let cancellation = context.cancellation.clone();

        let error = process_stream(
            &mut decoder,
            &staging,
            EnhanceOutputFormat::FloatWav,
            CancellingStream {
                samples: 0,
                cancellation,
            },
            &mut context,
        )
        .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::Cancelled);
        assert!(staging.is_file());
        drop(transaction);
        assert!(!staging.exists());
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"existing target must survive cancellation"
        );
    }

    #[test]
    #[ignore = "long synthetic host-pipeline proof; run explicitly"]
    fn fifty_hundred_and_one_hundred_fifty_seconds_keep_fixed_host_buffers() {
        for seconds in [50, 100, 150] {
            let root = tempfile::tempdir().unwrap();
            let input = root.path().join("input.wav");
            let output = root.path().join("enhanced.wav");
            let frames = seconds * SAMPLE_RATE as usize;
            write_synthetic_wav(&input, frames);
            let mut decoder = LibavAudioStream::open(&input, SAMPLE_RATE, CHANNELS).unwrap();
            let transaction = FileOutputTransaction::new(&output, false).unwrap();
            let manager = ModelManager::from_models_root(root.path().join("models"));
            let mut progress = NoopProgress;
            let mut context = RunContext::new(&manager, &mut progress);
            let peak_chunk = Rc::new(Cell::new(0));
            let result = process_stream(
                &mut decoder,
                transaction.staging_path(),
                EnhanceOutputFormat::FloatWav,
                IdentityStream {
                    samples: 0,
                    peak_chunk: Rc::clone(&peak_chunk),
                },
                &mut context,
            )
            .unwrap();

            assert_eq!(result.stats.input_samples, frames as u64);
            assert_eq!(result.stats.output_samples, frames as u64);
            assert_eq!(result.stream_working_set_high_watermark_bytes, 4_096);
            assert_eq!(peak_chunk.get(), STREAM_CHUNK_FRAMES);
        }
    }
}
