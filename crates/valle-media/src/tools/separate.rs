//! Complete-file, bounded-memory two-stem source separation.

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use anyhow::{Result as AnyResult, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    codec::{FloatWavWriter, validate_float_wav},
    frame::AudioBuffer,
    models::{
        ModelSelection,
        adapters::demucs::{
            CHANNELS, MODEL_ID, SAMPLE_RATE, SeparationSession, StemChunk, StemSink, StereoSource,
            TrackStatsBuilder,
        },
    },
};

use super::{
    MediaSummary, ModelProvenance, OutputArtifact, ProcessedMedia, ProcessingReport, RunContext,
    StageTiming, ToolError, ToolErrorCode, ToolEvent, ToolPhase, ToolRun,
    audio_workspace::AudioWorkspace,
    model_session::ModelSessionCandidates,
    output::{DirectoryOutputTransaction, paths_refer_to_same_file},
    probe_audio_input,
};

const IO_WINDOW_FRAMES: u64 = 65_536;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeparateRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub model: ModelSelection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeparateResult {
    pub outputs: Vec<OutputArtifact>,
    pub audio_seconds: f64,
    pub samples: u64,
    pub segments: usize,
    pub real_time_factor: f64,
    /// Largest normalized model input buffer reported by the adapter, in `f32` samples.
    pub max_input_samples: usize,
    /// Largest raw multi-stem model output buffer reported by the adapter, in `f32` samples.
    pub max_output_samples: usize,
    /// Largest finalized multi-stem chunk reported by the adapter, in `f32` samples.
    pub max_finalized_samples: usize,
}

pub fn run(
    request: SeparateRequest,
    context: &mut RunContext<'_>,
) -> Result<ToolRun<SeparateResult>, ToolError> {
    context.validate()?;
    validate_request(&request)?;
    DirectoryOutputTransaction::validate_target(&request.output_dir)?;
    let started = Instant::now();

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Resolving));
    check_cancelled(context)?;
    context.progress.event(ToolEvent::phase(ToolPhase::Loading));
    let load_started = Instant::now();
    let candidates = ModelSessionCandidates::resolve(context.models, request.model.clone())?;
    let opened = candidates.open("load Demucs", |model| {
        SeparationSession::open(model, context.resources.cpu_threads)
    })?;
    let mut session = opened.session;
    let load_seconds = load_started.elapsed().as_secs_f64();
    let provenance = ModelProvenance::from(&opened.model.resolved);
    let warnings = opened.warnings;
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Decoding));
    let decode_started = Instant::now();
    let input_report = probe_audio_input(&request.input)?;
    let mut workspace = AudioWorkspace::decode_spooled(
        &request.input,
        SAMPLE_RATE,
        CHANNELS,
        &context.resources,
        &context.cancellation,
    )?;
    let decode_seconds = decode_started.elapsed().as_secs_f64();
    let samples = workspace.total_frames();
    let audio_seconds = samples as f64 / f64::from(SAMPLE_RATE);
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Preprocessing));
    let preprocess_started = Instant::now();
    let normalization = track_normalization(&mut workspace, context)?;
    let preprocess_seconds = preprocess_started.elapsed().as_secs_f64();
    check_cancelled(context)?;

    let transaction = DirectoryOutputTransaction::new(&request.output_dir)?;
    let vocals_staging = transaction.staging_path().join("vocals.wav");
    let instrumental_staging = transaction.staging_path().join("instrumental.wav");
    let vocals = FloatWavWriter::create(&vocals_staging, SAMPLE_RATE, CHANNELS)
        .map_err(|error| output_error("create vocals WAV", error))?;
    let instrumental = FloatWavWriter::create(&instrumental_staging, SAMPLE_RATE, CHANNELS)
        .map_err(|error| output_error("create instrumental WAV", error))?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Inferencing));
    let mut source = WorkspaceStereoSource {
        workspace: &mut workspace,
        position: 0,
        cancellation: &context.cancellation,
    };
    let mut sink = WavStemSink::new(vocals, instrumental, &context.cancellation);
    let separation_started = Instant::now();
    let separation = session
        .separate(normalization, &mut source, &mut sink)
        .map_err(|error| {
            if context.cancellation.is_cancelled() {
                ToolError::cancelled()
            } else {
                ToolError::new(
                    ToolErrorCode::InferenceFailed,
                    format!("separate {}: {error:#}", request.input.display()),
                )
            }
        })?;
    let separation_seconds = separation_started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Encoding));
    let (vocals_frames, instrumental_frames, encode_seconds) = sink.finish()?;
    if vocals_frames != samples || instrumental_frames != samples || separation.samples != samples {
        return Err(ToolError::new(
            ToolErrorCode::OutputValidationFailed,
            format!(
                "separation length mismatch: source={samples}, adapter={}, vocals={vocals_frames}, instrumental={instrumental_frames}",
                separation.samples
            ),
        ));
    }
    check_cancelled(context)?;

    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Validating));
    validate_float_wav(&vocals_staging, SAMPLE_RATE, CHANNELS, samples)
        .map_err(|error| output_error("validate staged vocals WAV", error))?;
    validate_float_wav(&instrumental_staging, SAMPLE_RATE, CHANNELS, samples)
        .map_err(|error| output_error("validate staged instrumental WAV", error))?;
    check_cancelled(context)?;
    let output_dir = transaction.commit()?;
    let vocals_path = output_dir.join("vocals.wav");
    let instrumental_path = output_dir.join("instrumental.wav");
    let output_summary = MediaSummary {
        duration_seconds: Some(audio_seconds),
        sample_rate_hz: Some(SAMPLE_RATE),
        channels: Some(CHANNELS),
        sample_format: Some("f32".to_owned()),
        ..MediaSummary::default()
    }
    .with_audio_encoding("wav", "pcm_f32le");
    let outputs = vec![
        OutputArtifact {
            role: "vocals".to_owned(),
            path: vocals_path,
            media_type: "audio/wav".to_owned(),
            summary: Some(output_summary.clone()),
        },
        OutputArtifact {
            role: "instrumental".to_owned(),
            path: instrumental_path,
            media_type: "audio/wav".to_owned(),
            summary: Some(output_summary),
        },
    ];
    let inference_seconds = separation.inference_ms / 1_000.0;
    let postprocess_seconds = (separation_seconds - inference_seconds - encode_seconds).max(0.0);
    let total_seconds = started.elapsed().as_secs_f64();
    context
        .progress
        .event(ToolEvent::phase(ToolPhase::Completed));

    Ok(ToolRun {
        result: SeparateResult {
            outputs: outputs.clone(),
            audio_seconds,
            samples,
            segments: separation.segments,
            real_time_factor: if audio_seconds > 0.0 {
                total_seconds / audio_seconds
            } else {
                0.0
            },
            max_input_samples: separation.max_input_samples,
            max_output_samples: separation.max_output_samples,
            max_finalized_samples: separation.max_finalized_samples,
        },
        report: ProcessingReport {
            operation: "separate".to_owned(),
            parameters: normalized_parameters(&request),
            input: request.input,
            input_media_type: input_report.media_type,
            input_summary: input_report.summary,
            outputs,
            models: vec![provenance],
            processed: ProcessedMedia {
                frames: 0,
                audio_seconds,
            },
            timing: StageTiming {
                load_seconds,
                decode_seconds,
                preprocess_seconds,
                inference_seconds,
                postprocess_seconds,
                encode_seconds,
                total_seconds,
            },
        },
        warnings,
    })
}

fn track_normalization(
    workspace: &mut AudioWorkspace,
    context: &RunContext<'_>,
) -> Result<crate::models::inference::demucs::TrackNormalization, ToolError> {
    let mut stats = TrackStatsBuilder::new(workspace.sample_rate(), workspace.channels())
        .map_err(|error| ToolError::new(ToolErrorCode::InvalidInput, error.to_string()))?;
    let mut position = 0_u64;
    while position < workspace.total_frames() {
        check_cancelled(context)?;
        let end = (position + IO_WINDOW_FRAMES).min(workspace.total_frames());
        let buffer = workspace.read_frames(position, end)?;
        stats.push_interleaved(&buffer.samples).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InvalidInput,
                format!("compute Demucs track statistics: {error:#}"),
            )
        })?;
        position = end;
    }
    stats.finish().map_err(|error| {
        ToolError::new(
            ToolErrorCode::InvalidInput,
            format!("compute Demucs track statistics: {error:#}"),
        )
    })
}

struct WorkspaceStereoSource<'a> {
    workspace: &'a mut AudioWorkspace,
    position: u64,
    cancellation: &'a super::CancellationToken,
}

impl StereoSource for WorkspaceStereoSource<'_> {
    fn read_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> AnyResult<usize> {
        ensure!(
            left.len() == right.len(),
            "stereo destination lengths differ"
        );
        ensure!(!self.cancellation.is_cancelled(), "media tool cancelled");
        let count = left.len().min(usize::try_from(
            self.workspace.total_frames() - self.position,
        )?);
        if count == 0 {
            return Ok(0);
        }
        let end = self.position + count as u64;
        let buffer = self
            .workspace
            .read_frames(self.position, end)
            .map_err(anyhow::Error::new)?;
        for (index, frame) in buffer.samples.chunks_exact(2).enumerate() {
            left[index] = frame[0];
            right[index] = frame[1];
        }
        self.position = end;
        Ok(count)
    }
}

struct WavStemSink<'a> {
    vocals: FloatWavWriter,
    instrumental: FloatWavWriter,
    cancellation: &'a super::CancellationToken,
    encode_seconds: f64,
}

impl<'a> WavStemSink<'a> {
    fn new(
        vocals: FloatWavWriter,
        instrumental: FloatWavWriter,
        cancellation: &'a super::CancellationToken,
    ) -> Self {
        Self {
            vocals,
            instrumental,
            cancellation,
            encode_seconds: 0.0,
        }
    }

    fn finish(self) -> Result<(u64, u64, f64), ToolError> {
        let started = Instant::now();
        let vocals = self
            .vocals
            .finish()
            .map_err(|error| output_error("finish vocals WAV", error))?;
        let instrumental = self
            .instrumental
            .finish()
            .map_err(|error| output_error("finish instrumental WAV", error))?;
        Ok((
            vocals,
            instrumental,
            self.encode_seconds + started.elapsed().as_secs_f64(),
        ))
    }
}

impl StemSink for WavStemSink<'_> {
    fn write_stems(&mut self, chunk: StemChunk<'_>) -> AnyResult<()> {
        ensure!(!self.cancellation.is_cancelled(), "media tool cancelled");
        ensure!(
            self.vocals.frames_written() == chunk.start_sample
                && self.instrumental.frames_written() == chunk.start_sample,
            "Demucs emitted a non-contiguous stem chunk"
        );
        let started = Instant::now();
        self.vocals
            .write(&stereo_buffer(chunk.vocals.left, chunk.vocals.right))?;
        self.instrumental.write(&stereo_buffer(
            chunk.instrumental.left,
            chunk.instrumental.right,
        ))?;
        self.encode_seconds += started.elapsed().as_secs_f64();
        Ok(())
    }
}

fn stereo_buffer(left: &[f32], right: &[f32]) -> AudioBuffer {
    let mut samples = Vec::with_capacity(left.len().saturating_mul(2));
    for (&left, &right) in left.iter().zip(right) {
        samples.push(left);
        samples.push(right);
    }
    AudioBuffer {
        samples,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
    }
}

fn validate_request(request: &SeparateRequest) -> Result<(), ToolError> {
    if !request.input.is_file() {
        return Err(ToolError::invalid_input(format!(
            "input does not exist or is not a file: {}",
            request.input.display()
        )));
    }
    if paths_refer_to_same_file(&request.input, &request.output_dir) {
        return Err(ToolError::invalid_input(
            "separation input and output directory must differ",
        ));
    }
    if request.model.id != MODEL_ID {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedAdapter,
            format!(
                "separate supports model {MODEL_ID:?}, got {:?}",
                request.model.id
            ),
        ));
    }
    Ok(())
}

fn normalized_parameters(request: &SeparateRequest) -> BTreeMap<String, serde_json::Value> {
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
    parameters
}

fn output_error(action: &str, error: anyhow::Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::OutputValidationFailed,
        format!("{action}: {error:#}"),
    )
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
    use super::*;
    use crate::models::inference::demucs::{
        AdapterOptions, DemucsAdapter, SEGMENT_SAMPLES, STEMS, STRIDE_SAMPLES, SegmentBackend,
        SegmentOutput, SeparationSummary,
    };

    struct GeneratedStereoSource {
        remaining: usize,
        max_request: usize,
    }

    impl StereoSource for GeneratedStereoSource {
        fn read_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> AnyResult<usize> {
            ensure!(left.len() == right.len());
            self.max_request = self.max_request.max(left.len());
            let count = self.remaining.min(left.len());
            left[..count].fill(0.0);
            right[..count].fill(0.0);
            self.remaining -= count;
            Ok(count)
        }
    }

    struct ZeroBackend;

    impl SegmentBackend for ZeroBackend {
        fn infer_segment(&mut self, _mix: &[f32], _shape: [usize; 3]) -> AnyResult<SegmentOutput> {
            Ok(SegmentOutput {
                shape: vec![1, STEMS, CHANNELS as usize, SEGMENT_SAMPLES],
                data: vec![0.0; STEMS * CHANNELS as usize * SEGMENT_SAMPLES],
            })
        }
    }

    #[derive(Default)]
    struct CountingSink {
        samples: u64,
        max_chunk_samples: usize,
    }

    impl StemSink for CountingSink {
        fn write_stems(&mut self, chunk: StemChunk<'_>) -> AnyResult<()> {
            ensure!(chunk.start_sample == self.samples);
            ensure!(chunk.vocals.left.len() == chunk.vocals.right.len());
            ensure!(chunk.vocals.left.len() == chunk.instrumental.left.len());
            ensure!(chunk.vocals.left.len() == chunk.instrumental.right.len());
            self.samples += chunk.vocals.left.len() as u64;
            self.max_chunk_samples = self.max_chunk_samples.max(chunk.vocals.left.len());
            Ok(())
        }
    }

    fn zero_normalization(samples: usize) -> crate::models::inference::demucs::TrackNormalization {
        let mut stats = TrackStatsBuilder::new(SAMPLE_RATE, CHANNELS).unwrap();
        let zeros = [0.0_f32; 4_096];
        let mut remaining = samples;
        while remaining > 0 {
            let count = remaining.min(zeros.len());
            stats.push(&zeros[..count], &zeros[..count]).unwrap();
            remaining -= count;
        }
        stats.finish().unwrap()
    }

    fn synthetic_separation(samples: usize) -> (SeparationSummary, usize, CountingSink) {
        let adapter = DemucsAdapter::new(AdapterOptions {
            read_chunk_samples: 4_093,
        })
        .unwrap();
        let mut source = GeneratedStereoSource {
            remaining: samples,
            max_request: 0,
        };
        let mut backend = ZeroBackend;
        let mut sink = CountingSink::default();
        let summary = adapter
            .separate(
                &mut backend,
                zero_normalization(samples),
                &mut source,
                &mut sink,
            )
            .unwrap();
        (summary, source.max_request, sink)
    }

    #[test]
    fn stereo_interleaving_preserves_channel_order() {
        let buffer = stereo_buffer(&[1.0, 2.0], &[-1.0, -2.0]);
        assert_eq!(buffer.samples, vec![1.0, -1.0, 2.0, -2.0]);
        assert_eq!(buffer.sample_rate, 44_100);
        assert_eq!(buffer.channels, 2);
    }

    #[test]
    fn existing_output_directory_is_rejected_before_model_work() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"fixture").unwrap();
        let request = SeparateRequest {
            input,
            output_dir: root.path().to_owned(),
            model: ModelSelection::pinned_default("demucs"),
        };
        assert_eq!(
            DirectoryOutputTransaction::new(&request.output_dir)
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidInput
        );
    }

    #[test]
    fn request_rejects_a_catalog_model_without_the_demucs_adapter_contract() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.wav");
        std::fs::write(&input, b"fixture").unwrap();
        let request = SeparateRequest {
            input,
            output_dir: root.path().join("stems"),
            model: ModelSelection::pinned_default("birefnet"),
        };

        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ToolErrorCode::UnsupportedAdapter
        );
    }

    #[test]
    fn adapter_buffer_high_watermarks_do_not_grow_with_track_length() {
        let (short, short_request, short_sink) = synthetic_separation(STRIDE_SAMPLES + 1);
        let (long, long_request, long_sink) = synthetic_separation(STRIDE_SAMPLES * 3 + 17);

        assert!(long.segments > short.segments);
        assert_eq!(short.max_input_samples, long.max_input_samples);
        assert_eq!(short.max_output_samples, long.max_output_samples);
        assert_eq!(short.max_finalized_samples, long.max_finalized_samples);
        assert_eq!(short_request, long_request);
        assert_eq!(short_request, 4_093);
        assert_eq!(short_sink.max_chunk_samples, STRIDE_SAMPLES);
        assert_eq!(long_sink.max_chunk_samples, STRIDE_SAMPLES);
        assert_eq!(short_sink.samples, short.samples);
        assert_eq!(long_sink.samples, long.samples);
    }

    #[test]
    #[ignore = "long synthetic adapter proof; run explicitly"]
    fn fifty_hundred_and_one_hundred_fifty_seconds_keep_fixed_demucs_buffers() {
        let expected = (
            CHANNELS as usize * SEGMENT_SAMPLES,
            STEMS * CHANNELS as usize * SEGMENT_SAMPLES,
            STEMS * CHANNELS as usize * STRIDE_SAMPLES,
        );
        for seconds in [50, 100, 150] {
            let samples = seconds * SAMPLE_RATE as usize;
            let (summary, max_request, sink) = synthetic_separation(samples);
            assert_eq!(summary.samples, samples as u64);
            assert_eq!(sink.samples, samples as u64);
            assert_eq!(max_request, 4_093);
            assert_eq!(sink.max_chunk_samples, STRIDE_SAMPLES);
            assert_eq!(
                (
                    summary.max_input_samples,
                    summary.max_output_samples,
                    summary.max_finalized_samples,
                ),
                expected
            );
        }
    }
}
