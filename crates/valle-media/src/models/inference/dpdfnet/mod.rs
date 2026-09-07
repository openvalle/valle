//! Load-once DPDFNet speech-enhancement runtime.
//!
//! The public media boundary is deliberately codec-neutral: callers push finite mono
//! `f32` PCM sampled at 48 kHz and receive mono `f32` PCM chunks. This crate never
//! opens audio files, writes output files, or invokes another process. The adapter owns
//! every model-specific detail: centered Vorbis STFT/iSTFT, metadata-derived recurrent
//! state, four-frame attenuation-limit alignment, and fixed latency compensation.
//!
//! The backend-neutral adapter creates isolated per-job streams. With the `onnx` feature,
//! `DpdfNetSession` loads one ONNX Runtime session and reuses it across sequential jobs
//! without carrying recurrent state from one job to the next.

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use std::collections::BTreeMap;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use std::mem::size_of;
#[cfg(feature = "model-dpdfnet-onnx")]
use std::path::{Path, PathBuf};
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use std::sync::Arc;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use std::time::{Duration, Instant};

#[cfg(feature = "model-dpdfnet-onnx")]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-dpdfnet-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route, TensorSpec};
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use anyhow::{Context, Result, ensure};
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use rustfft::num_complex::Complex32;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use rustfft::{Fft, FftPlanner};
#[cfg(feature = "model-dpdfnet-onnx")]
use serde::Deserialize;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
use serde::Serialize;

pub const ADAPTER: &str = "dpdfnet-streaming-enhancement";
pub const CONTRACT_VERSION: u32 = 1;
pub const SAMPLE_RATE: u32 = 48_000;
pub const N_FFT: usize = 960;
pub const HOP_LENGTH: usize = 480;
pub const FREQ_BINS: usize = N_FFT / 2 + 1;
pub const STATE_SIZE: usize = 56_436;
pub const ERB_NORM_STATE_SIZE: usize = 481;
pub const SPEC_NORM_STATE_SIZE: usize = 96;
pub const ATTN_LIMIT_DB: f32 = 12.0;
pub const NOISY_FRAME_OFFSET: usize = 4;
pub const LATENCY_SAMPLES: usize = NOISY_FRAME_OFFSET * HOP_LENGTH;

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
const SPEC_VALUES: usize = FREQ_BINS * 2;
#[cfg(feature = "model-dpdfnet-onnx")]
const SPEC_SHAPE: [usize; 4] = [1, 1, FREQ_BINS, 2];
// Upstream appends one FFT window; centered STFT contributes the final half window.
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
const CENTERED_STREAM_TRAILING_ZERO_SAMPLES: usize = N_FFT + HOP_LENGTH;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
const OUTPUT_CLAMP_MIN: f32 = -1.0;
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
const OUTPUT_CLAMP_MAX: f32 = 1.0;

// Frame-level execution remains private so the public runtime boundary cannot bypass the
// adapter's PCM, DSP, recurrent-state, and latency contract.
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
trait FrameBackend {
    fn infer_frame(
        &mut self,
        noisy_spectrum: &[f32],
        recurrent_state: &mut Vec<f32>,
        enhanced_spectrum: &mut Vec<f32>,
    ) -> Result<()>;
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
struct DpdfNetAdapter {
    initial_state: Arc<[f32]>,
    window: Arc<[f32]>,
    overlap_normalizer: Arc<[f32]>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
impl DpdfNetAdapter {
    fn new(initial_state: Vec<f32>) -> Result<Self> {
        validate_initial_state(&initial_state)?;
        let window = vorbis_window(N_FFT);
        let overlap_normalizer = (0..HOP_LENGTH)
            .map(|index| {
                (window[index] * window[index]
                    + window[index + HOP_LENGTH] * window[index + HOP_LENGTH])
                    .max(1.0e-11)
            })
            .collect::<Vec<_>>();
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(N_FFT);
        let inverse = planner.plan_fft_inverse(N_FFT);
        Ok(Self {
            initial_state: initial_state.into(),
            window: window.into(),
            overlap_normalizer: overlap_normalizer.into(),
            forward,
            inverse,
        })
    }

    fn start_stream<'backend>(
        &self,
        backend: &'backend mut dyn FrameBackend,
    ) -> DpdfNetStream<'backend> {
        DpdfNetStream::new(self, backend)
    }

    /// Heap bytes shared by all jobs, excluding the backend/model session itself.
    fn shared_dsp_bytes(&self) -> usize {
        (self.initial_state.len() + self.window.len() + self.overlap_normalizer.len())
            * size_of::<f32>()
    }
}

/// Snapshot of bounded per-job adapter storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
pub struct StreamWorkingSet {
    /// Heap capacity owned by the stream, excluding returned PCM and backend/session memory.
    pub heap_capacity_bytes: usize,
    /// Source samples temporarily retained to construct a centered frame.
    pub buffered_input_samples: usize,
    pub recurrent_state_values: usize,
    pub noisy_history_frames: usize,
}

/// Cumulative timings and counters for one enhancement job.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
pub struct StreamStats {
    pub sample_rate: u32,
    pub input_samples: u64,
    pub output_samples: u64,
    pub frames: u64,
    pub duration_seconds: f64,
    pub processing_ms: f64,
    pub inference_ms: f64,
    pub dsp_ms: f64,
    pub real_time_factor: f64,
    pub real_time_multiple: f64,
}

/// Tail PCM and final counters returned by [`DpdfNetStream::finish`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
pub struct FinishedStream {
    /// Final bounded tail. Concatenate it after the chunks returned by `push`.
    pub samples: Vec<f32>,
    pub stats: StreamStats,
}

/// One stateful PCM job. Drop it after an error; a fresh job always starts cleanly.
#[cfg(any(feature = "model-dpdfnet-onnx", test))]
pub struct DpdfNetStream<'backend> {
    backend: &'backend mut dyn FrameBackend,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    window: Arc<[f32]>,
    overlap_normalizer: Arc<[f32]>,
    prefix: Vec<f32>,
    pending_hop: Vec<f32>,
    analysis: Vec<f32>,
    fft_buffer: Vec<Complex32>,
    overlap: Vec<f32>,
    noisy_spectrum: Vec<f32>,
    enhanced_spectrum: Vec<f32>,
    noisy_history: Vec<Vec<f32>>,
    history_len: usize,
    history_next: usize,
    recurrent_state: Vec<f32>,
    latency: StreamingLatency,
    input_samples: usize,
    frames: usize,
    processing_time: Duration,
    inference_time: Duration,
    initialized: bool,
    poisoned: bool,
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
impl<'backend> DpdfNetStream<'backend> {
    fn new(adapter: &DpdfNetAdapter, backend: &'backend mut dyn FrameBackend) -> Self {
        Self {
            backend,
            forward: Arc::clone(&adapter.forward),
            inverse: Arc::clone(&adapter.inverse),
            window: Arc::clone(&adapter.window),
            overlap_normalizer: Arc::clone(&adapter.overlap_normalizer),
            prefix: Vec::with_capacity(HOP_LENGTH + 1),
            pending_hop: Vec::with_capacity(HOP_LENGTH),
            analysis: vec![0.0; N_FFT],
            fft_buffer: vec![Complex32::new(0.0, 0.0); N_FFT],
            overlap: vec![0.0; N_FFT],
            noisy_spectrum: vec![0.0; SPEC_VALUES],
            enhanced_spectrum: vec![0.0; SPEC_VALUES],
            noisy_history: (0..NOISY_FRAME_OFFSET)
                .map(|_| vec![0.0; SPEC_VALUES])
                .collect(),
            history_len: 0,
            history_next: 0,
            recurrent_state: adapter.initial_state.to_vec(),
            latency: StreamingLatency::new(),
            input_samples: 0,
            frames: 0,
            processing_time: Duration::ZERO,
            inference_time: Duration::ZERO,
            initialized: false,
            poisoned: false,
        }
    }

    /// Push one finite mono 48 kHz PCM chunk and return all PCM now available.
    ///
    /// Chunk sizes are arbitrary, including zero. The returned vector is owned by the
    /// caller and is not retained by the stream. Keeping caller chunks bounded therefore
    /// keeps the adapter working set independent of total input duration.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        ensure!(
            !self.poisoned,
            "DPDFNet stream is unusable after a frame failure"
        );
        ensure!(
            samples.iter().all(|sample| sample.is_finite()),
            "DPDFNet PCM contains non-finite samples"
        );
        let next_input_samples = self
            .input_samples
            .checked_add(samples.len())
            .context("DPDFNet PCM stream is too long")?;
        let started = Instant::now();
        let result = self.push_inner(samples, next_input_samples);
        self.processing_time += started.elapsed();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Flush the model and return the final PCM tail and job statistics.
    ///
    /// This consumes the job. A subsequent job on the same loaded session must be started
    /// with `start_stream`, which restores the metadata-derived recurrent state.
    pub fn finish(mut self) -> Result<FinishedStream> {
        ensure!(
            !self.poisoned,
            "DPDFNet stream is unusable after a frame failure"
        );
        let started = Instant::now();
        let result = self.finish_inner();
        self.processing_time += started.elapsed();
        let samples = result?;
        let stats = self.stats();
        ensure!(
            stats.output_samples == stats.input_samples,
            "DPDFNet emitted {} samples for a {}-sample input",
            stats.output_samples,
            stats.input_samples
        );
        Ok(FinishedStream { samples, stats })
    }

    pub fn stats(&self) -> StreamStats {
        let duration_seconds = self.input_samples as f64 / SAMPLE_RATE as f64;
        let processing_ms = duration_ms(self.processing_time);
        let inference_ms = duration_ms(self.inference_time);
        let real_time_factor = if duration_seconds > 0.0 {
            processing_ms / (duration_seconds * 1_000.0)
        } else {
            0.0
        };
        StreamStats {
            sample_rate: SAMPLE_RATE,
            input_samples: self.input_samples as u64,
            output_samples: self.latency.written as u64,
            frames: self.frames as u64,
            duration_seconds,
            processing_ms,
            inference_ms,
            dsp_ms: duration_ms(self.processing_time.saturating_sub(self.inference_time)),
            real_time_factor,
            real_time_multiple: if real_time_factor > 0.0 {
                real_time_factor.recip()
            } else {
                0.0
            },
        }
    }

    pub fn working_set(&self) -> StreamWorkingSet {
        let f32_capacity = self.prefix.capacity()
            + self.pending_hop.capacity()
            + self.analysis.capacity()
            + self.overlap.capacity()
            + self.noisy_spectrum.capacity()
            + self.enhanced_spectrum.capacity()
            + self.recurrent_state.capacity()
            + self.noisy_history.iter().map(Vec::capacity).sum::<usize>();
        StreamWorkingSet {
            heap_capacity_bytes: f32_capacity * size_of::<f32>()
                + self.fft_buffer.capacity() * size_of::<Complex32>(),
            buffered_input_samples: self.prefix.len() + self.pending_hop.len(),
            recurrent_state_values: self.recurrent_state.len(),
            noisy_history_frames: self.history_len,
        }
    }

    fn push_inner(&mut self, samples: &[f32], next_input_samples: usize) -> Result<Vec<f32>> {
        self.input_samples = next_input_samples;
        let mut output = Vec::with_capacity(samples.len().saturating_add(HOP_LENGTH));
        let mut cursor = 0;

        if !self.initialized {
            let needed = HOP_LENGTH + 1 - self.prefix.len();
            let count = needed.min(samples.len());
            self.prefix.extend_from_slice(&samples[..count]);
            cursor += count;
            if self.prefix.len() == HOP_LENGTH + 1 {
                self.initialize_live(&mut output)?;
            }
        }

        while cursor < samples.len() {
            let needed = HOP_LENGTH - self.pending_hop.len();
            let count = needed.min(samples.len() - cursor);
            self.pending_hop
                .extend_from_slice(&samples[cursor..cursor + count]);
            cursor += count;
            if self.pending_hop.len() == HOP_LENGTH {
                self.advance_hop(&mut output)?;
            }
        }
        Ok(output)
    }

    fn finish_inner(&mut self) -> Result<Vec<f32>> {
        let mut output = Vec::with_capacity(LATENCY_SAMPLES + HOP_LENGTH);
        let mut tail_zeros = CENTERED_STREAM_TRAILING_ZERO_SAMPLES;
        if !self.initialized {
            self.initialize_at_end(&mut tail_zeros, &mut output)?;
        }
        while tail_zeros > 0 {
            let count = (HOP_LENGTH - self.pending_hop.len()).min(tail_zeros);
            self.pending_hop.resize(self.pending_hop.len() + count, 0.0);
            tail_zeros -= count;
            if self.pending_hop.len() == HOP_LENGTH {
                self.advance_hop(&mut output)?;
            }
        }
        self.latency.finish(self.input_samples, &mut output)?;
        Ok(output)
    }

    fn initialize_live(&mut self, output: &mut Vec<f32>) -> Result<()> {
        ensure!(
            self.prefix.len() == HOP_LENGTH + 1,
            "live initialization requires 481 source samples"
        );
        self.fill_initial_analysis();
        self.pending_hop.push(self.prefix[HOP_LENGTH]);
        self.prefix.clear();
        self.initialized = true;
        self.process_current_frame(output)
    }

    fn initialize_at_end(&mut self, tail_zeros: &mut usize, output: &mut Vec<f32>) -> Result<()> {
        ensure!(
            self.prefix.len() <= HOP_LENGTH,
            "finished stream retained too many prefix samples"
        );
        self.fill_initial_analysis();
        let missing = HOP_LENGTH - self.prefix.len();
        ensure!(
            *tail_zeros >= missing,
            "DPDFNet tail padding underflowed during centered initialization"
        );
        *tail_zeros -= missing;
        self.prefix.clear();
        self.initialized = true;
        self.process_current_frame(output)
    }

    fn fill_initial_analysis(&mut self) {
        self.analysis.fill(0.0);
        for destination in 0..HOP_LENGTH {
            let source = HOP_LENGTH - destination;
            if let Some(sample) = self.prefix.get(source) {
                self.analysis[destination] = *sample;
            }
        }
        let copied = self.prefix.len().min(HOP_LENGTH);
        self.analysis[HOP_LENGTH..HOP_LENGTH + copied].copy_from_slice(&self.prefix[..copied]);
    }

    fn advance_hop(&mut self, output: &mut Vec<f32>) -> Result<()> {
        ensure!(
            self.pending_hop.len() == HOP_LENGTH,
            "DPDFNet needs one complete hop before frame inference"
        );
        self.analysis.copy_within(HOP_LENGTH..N_FFT, 0);
        self.analysis[HOP_LENGTH..].copy_from_slice(&self.pending_hop);
        self.pending_hop.clear();
        self.process_current_frame(output)
    }

    fn process_current_frame(&mut self, output: &mut Vec<f32>) -> Result<()> {
        stft_window_into(
            &self.analysis,
            &self.window,
            self.forward.as_ref(),
            &mut self.fft_buffer,
            &mut self.noisy_spectrum,
        );
        let inference_started = Instant::now();
        let inference = self.backend.infer_frame(
            &self.noisy_spectrum,
            &mut self.recurrent_state,
            &mut self.enhanced_spectrum,
        );
        self.inference_time += inference_started.elapsed();
        inference.with_context(|| format!("DPDFNet inference failed at frame {}", self.frames))?;
        validate_frame_buffers(&self.enhanced_spectrum, &self.recurrent_state, self.frames)?;

        let alpha = 10.0_f32.powf(-ATTN_LIMIT_DB / 20.0);
        if self.history_len < NOISY_FRAME_OFFSET {
            for enhanced in &mut self.enhanced_spectrum {
                *enhanced *= 1.0 - alpha;
            }
            self.noisy_history[self.history_len].copy_from_slice(&self.noisy_spectrum);
            self.history_len += 1;
        } else {
            let aligned = &self.noisy_history[self.history_next];
            for (enhanced, noisy) in self.enhanced_spectrum.iter_mut().zip(aligned) {
                *enhanced = alpha.mul_add(*noisy, (1.0 - alpha) * *enhanced);
            }
            self.noisy_history[self.history_next].copy_from_slice(&self.noisy_spectrum);
            self.history_next = (self.history_next + 1) % NOISY_FRAME_OFFSET;
        }

        inverse_frame(
            &self.enhanced_spectrum,
            &self.window,
            self.inverse.as_ref(),
            &mut self.fft_buffer,
            &mut self.overlap,
        );
        if self.frames > 0 {
            for index in 0..HOP_LENGTH {
                self.overlap[index] /= self.overlap_normalizer[index];
            }
            self.latency
                .push(&self.overlap[..HOP_LENGTH], self.input_samples, output)?;
        }
        self.overlap.copy_within(HOP_LENGTH..N_FFT, 0);
        self.overlap[HOP_LENGTH..].fill(0.0);
        self.frames += 1;
        Ok(())
    }
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
struct StreamingLatency {
    remaining_delay: usize,
    written: usize,
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
impl StreamingLatency {
    fn new() -> Self {
        Self {
            remaining_delay: LATENCY_SAMPLES,
            written: 0,
        }
    }

    fn push(
        &mut self,
        samples: &[f32],
        known_input_samples: usize,
        output: &mut Vec<f32>,
    ) -> Result<()> {
        let skip = self.remaining_delay.min(samples.len());
        self.remaining_delay -= skip;
        let committed = &samples[skip..];
        ensure!(
            self.written + committed.len() <= known_input_samples,
            "DPDFNet output advanced beyond the available PCM input"
        );
        output.extend(
            committed
                .iter()
                .map(|sample| sample.clamp(OUTPUT_CLAMP_MIN, OUTPUT_CLAMP_MAX)),
        );
        self.written += committed.len();
        Ok(())
    }

    fn finish(&mut self, target_len: usize, output: &mut Vec<f32>) -> Result<()> {
        ensure!(
            self.written <= target_len,
            "DPDFNet produced {} samples for a {target_len}-sample input",
            self.written
        );
        let missing = target_len - self.written;
        output.resize(output.len() + missing, 0.0);
        self.written = target_len;
        Ok(())
    }
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn stft_window_into(
    samples: &[f32],
    window: &[f32],
    forward: &dyn Fft<f32>,
    fft_buffer: &mut [Complex32],
    spectrum: &mut [f32],
) {
    for index in 0..N_FFT {
        fft_buffer[index] = Complex32::new(samples[index] * window[index], 0.0);
    }
    forward.process(fft_buffer);
    for (frequency, bin) in fft_buffer[..FREQ_BINS].iter().enumerate() {
        spectrum[frequency * 2] = bin.re;
        spectrum[frequency * 2 + 1] = bin.im;
    }
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn inverse_frame(
    spectrum: &[f32],
    window: &[f32],
    inverse: &dyn Fft<f32>,
    fft_buffer: &mut [Complex32],
    overlap: &mut [f32],
) {
    for frequency in 0..FREQ_BINS {
        fft_buffer[frequency] =
            Complex32::new(spectrum[frequency * 2], spectrum[frequency * 2 + 1]);
    }
    for frequency in 1..(FREQ_BINS - 1) {
        fft_buffer[N_FFT - frequency] = fft_buffer[frequency].conj();
    }
    inverse.process(fft_buffer);
    let inverse_scale = 1.0 / N_FFT as f32;
    for index in 0..N_FFT {
        overlap[index] += fft_buffer[index].re * inverse_scale * window[index];
    }
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn vorbis_window(length: usize) -> Vec<f32> {
    let half = length as f64 / 2.0;
    (0..length)
        .map(|index| {
            let phase = 0.5 * std::f64::consts::PI * (index as f64 + 0.5) / half;
            (0.5 * std::f64::consts::PI * phase.sin().powi(2)).sin() as f32
        })
        .collect()
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn validate_initial_state(state: &[f32]) -> Result<()> {
    ensure!(
        state.len() == STATE_SIZE,
        "DPDFNet initial state has {} values, expected {STATE_SIZE}",
        state.len()
    );
    ensure!(
        state.iter().all(|value| value.is_finite()),
        "DPDFNet initial state contains non-finite values"
    );
    ensure!(
        state.iter().any(|value| *value != 0.0),
        "DPDFNet metadata initial state must not be all zero"
    );
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn validate_frame_buffers(enhanced: &[f32], state: &[f32], frame: usize) -> Result<()> {
    ensure!(
        enhanced.len() == SPEC_VALUES,
        "DPDFNet enhanced frame {frame} has {} values, expected {SPEC_VALUES}",
        enhanced.len()
    );
    ensure!(
        state.len() == STATE_SIZE,
        "DPDFNet state at frame {frame} has {} values, expected {STATE_SIZE}",
        state.len()
    );
    ensure!(
        enhanced.iter().all(|value| value.is_finite()),
        "DPDFNet enhanced frame {frame} contains non-finite values"
    );
    ensure!(
        state.iter().all(|value| value.is_finite()),
        "DPDFNet state at frame {frame} contains non-finite values"
    );
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
#[derive(Debug)]
struct ModelMetadata {
    initial_state: Vec<f32>,
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn parse_model_metadata(metadata: &BTreeMap<String, String>) -> Result<ModelMetadata> {
    ensure_metadata(metadata, "model_type", "dpdfnet")?;
    ensure_metadata(metadata, "sample_rate", &SAMPLE_RATE.to_string())?;
    ensure_metadata(metadata, "n_fft", &N_FFT.to_string())?;
    ensure_metadata(metadata, "hop_length", &HOP_LENGTH.to_string())?;
    ensure_metadata(metadata, "window_length", &N_FFT.to_string())?;
    ensure_metadata(metadata, "window_type", "vorbis")?;
    ensure_metadata(metadata, "normalized", "0")?;
    ensure_metadata(metadata, "center", "1")?;
    ensure_metadata(metadata, "pad_mode", "reflect")?;
    ensure_metadata(metadata, "freq_bins", &FREQ_BINS.to_string())?;
    ensure_metadata(metadata, "state_size", &STATE_SIZE.to_string())?;
    ensure_metadata(
        metadata,
        "erb_norm_state_size",
        &ERB_NORM_STATE_SIZE.to_string(),
    )?;
    ensure_metadata(
        metadata,
        "spec_norm_state_size",
        &SPEC_NORM_STATE_SIZE.to_string(),
    )?;

    let erb = parse_metadata_floats(metadata, "erb_norm_init", ERB_NORM_STATE_SIZE)?;
    let spec = parse_metadata_floats(metadata, "spec_norm_init", SPEC_NORM_STATE_SIZE)?;
    let mut initial_state = vec![0.0; STATE_SIZE];
    initial_state[..ERB_NORM_STATE_SIZE].copy_from_slice(&erb);
    initial_state[ERB_NORM_STATE_SIZE..ERB_NORM_STATE_SIZE + SPEC_NORM_STATE_SIZE]
        .copy_from_slice(&spec);
    validate_initial_state(&initial_state)?;
    Ok(ModelMetadata { initial_state })
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn ensure_metadata(metadata: &BTreeMap<String, String>, key: &str, expected: &str) -> Result<()> {
    let actual = metadata
        .get(key)
        .with_context(|| format!("DPDFNet ONNX metadata is missing {key:?}"))?;
    ensure!(
        actual == expected,
        "DPDFNet ONNX metadata {key:?} is {actual:?}, expected {expected:?}"
    );
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn parse_metadata_floats(
    metadata: &BTreeMap<String, String>,
    key: &str,
    expected: usize,
) -> Result<Vec<f32>> {
    let encoded = metadata
        .get(key)
        .with_context(|| format!("DPDFNet ONNX metadata is missing {key:?}"))?;
    let values = encoded
        .split(',')
        .map(|value| {
            value
                .parse::<f32>()
                .with_context(|| format!("DPDFNet metadata {key:?} contains {value:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        values.len() == expected,
        "DPDFNet metadata {key:?} has {} values, expected {expected}",
        values.len()
    );
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "DPDFNet metadata {key:?} contains non-finite values"
    );
    Ok(values)
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn validate_contract(manifest: &ModelManifest, route: &Route, artifact: &Artifact) -> Result<()> {
    manifest.validate()?;
    ensure!(manifest.model.id == "dpdfnet", "manifest is not DPDFNet");
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported DPDFNet adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        route.artifact == artifact.id,
        "DPDFNet route {:?} selects artifact {:?}, got {:?}",
        route.id,
        route.artifact,
        artifact.id
    );
    ensure!(
        route.backend == Backend::OnnxCpu,
        "DPDFNet adapter only implements onnx-cpu, got {}",
        route.backend
    );
    ensure!(
        artifact.format == "onnx" && matches!(artifact.precision.as_str(), "fp32" | "int8-dynamic"),
        "DPDFNet requires the fp32 or linear-only dynamic-int8 ONNX artifact"
    );
    ensure!(
        manifest.contract.inputs.len() == 2 && manifest.contract.outputs.len() == 2,
        "DPDFNet contract must expose two inputs and two outputs"
    );
    validate_tensor(
        &manifest.contract.inputs[0],
        "spec",
        "ncf2",
        &[1, 1, FREQ_BINS as u64, 2],
    )?;
    validate_tensor(
        &manifest.contract.inputs[1],
        "state_in",
        "flat",
        &[STATE_SIZE as u64],
    )?;
    validate_tensor(
        &manifest.contract.outputs[0],
        "spec_e",
        "ncf2",
        &[1, 1, FREQ_BINS as u64, 2],
    )?;
    validate_tensor(
        &manifest.contract.outputs[1],
        "state_out",
        "flat",
        &[STATE_SIZE as u64],
    )?;

    let preprocess = &manifest.contract.preprocess;
    ensure_json_field(preprocess, "sample_rate", serde_json::json!(SAMPLE_RATE))?;
    ensure_json_field(preprocess, "channels", serde_json::json!(1))?;
    ensure_json_field(preprocess, "fft_size", serde_json::json!(N_FFT))?;
    ensure_json_field(preprocess, "hop_length", serde_json::json!(HOP_LENGTH))?;
    ensure_json_field(preprocess, "window", serde_json::json!("vorbis"))?;
    ensure_json_field(preprocess, "center", serde_json::json!(true))?;
    ensure_json_field(preprocess, "pad_mode", serde_json::json!("reflect"))?;
    ensure_json_field(
        preprocess,
        "input_tail_padding_samples",
        serde_json::json!(N_FFT),
    )?;
    ensure_json_field(preprocess, "stateful", serde_json::json!(true))?;
    ensure_json_field(
        preprocess,
        "serial_frame_inference",
        serde_json::json!(true),
    )?;
    ensure_json_field(
        preprocess,
        "attenuation_limit_db",
        serde_json::json!(ATTN_LIMIT_DB as u64),
    )?;
    ensure_json_field(
        preprocess,
        "noisy_frame_offset",
        serde_json::json!(NOISY_FRAME_OFFSET),
    )?;

    let postprocess = &manifest.contract.postprocess;
    ensure_json_field(
        postprocess,
        "latency_samples",
        serde_json::json!(LATENCY_SAMPLES),
    )?;
    ensure_json_field(
        postprocess,
        "drop_leading_samples",
        serde_json::json!(LATENCY_SAMPLES),
    )?;
    ensure_json_field(
        postprocess,
        "append_trailing_zeros",
        serde_json::json!(LATENCY_SAMPLES),
    )?;
    ensure_json_field(
        postprocess,
        "output_length",
        serde_json::json!("exact-input-sample-count"),
    )?;
    ensure_json_field(
        postprocess,
        "clamp",
        serde_json::json!([OUTPUT_CLAMP_MIN as i64, OUTPUT_CLAMP_MAX as i64]),
    )?;
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn ensure_json_field(
    value: &serde_json::Value,
    field: &str,
    expected: serde_json::Value,
) -> Result<()> {
    let actual = value
        .get(field)
        .with_context(|| format!("DPDFNet contract is missing {field:?}"))?;
    ensure!(
        actual == &expected,
        "unsupported DPDFNet {field} contract: got {actual}, expected {expected}"
    );
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn validate_tensor(tensor: &TensorSpec, name: &str, layout: &str, shape: &[u64]) -> Result<()> {
    let expected_shape = shape
        .iter()
        .copied()
        .map(Dimension::Fixed)
        .collect::<Vec<_>>();
    ensure!(
        tensor.name == name
            && tensor.dtype == "f32"
            && tensor.layout == layout
            && tensor.shape == expected_shape,
        "unsupported DPDFNet tensor contract for {name:?}"
    );
    Ok(())
}

#[cfg(any(feature = "model-dpdfnet-onnx", test))]
fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// Manifest-selected model load request. It contains no media paths.
#[cfg(feature = "model-dpdfnet-onnx")]
pub struct SessionLoadRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
}

#[cfg(feature = "model-dpdfnet-onnx")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnnxRouteOptions {
    intra_threads: Option<usize>,
    memory_pattern: Option<bool>,
    prepacking: Option<bool>,
}

#[cfg(feature = "model-dpdfnet-onnx")]
struct OnnxFrameBackend {
    session: OnnxSession,
    input_spec: String,
    input_state: String,
    output_spec: String,
    output_state: String,
}

#[cfg(feature = "model-dpdfnet-onnx")]
impl FrameBackend for OnnxFrameBackend {
    fn infer_frame(
        &mut self,
        noisy_spectrum: &[f32],
        recurrent_state: &mut Vec<f32>,
        enhanced_spectrum: &mut Vec<f32>,
    ) -> Result<()> {
        let mut outputs = self.session.run_f32_many(
            &[
                TensorInput::borrowed(&self.input_spec, SPEC_SHAPE, noisy_spectrum),
                TensorInput::borrowed(&self.input_state, [STATE_SIZE], recurrent_state.as_slice()),
            ],
            &[&self.output_spec, &self.output_state],
        )?;
        ensure!(outputs.len() == 2, "DPDFNet must return two ONNX outputs");
        let state = outputs.pop().expect("output count was checked");
        let enhanced = outputs.pop().expect("output count was checked");
        ensure!(
            enhanced.name == self.output_spec && enhanced.shape == SPEC_SHAPE,
            "DPDFNet enhanced spectrum has name/shape {:?}/{:?}, expected {:?}/{:?}",
            enhanced.name,
            enhanced.shape,
            self.output_spec,
            SPEC_SHAPE
        );
        ensure!(
            state.name == self.output_state && state.shape == [STATE_SIZE],
            "DPDFNet state has name/shape {:?}/{:?}, expected {:?}/{:?}",
            state.name,
            state.shape,
            self.output_state,
            [STATE_SIZE]
        );
        *enhanced_spectrum = enhanced.data;
        *recurrent_state = state.data;
        Ok(())
    }
}

/// Load-once ONNX session plus immutable DPDFNet DSP plan.
#[cfg(feature = "model-dpdfnet-onnx")]
pub struct DpdfNetSession {
    adapter: DpdfNetAdapter,
    backend: OnnxFrameBackend,
    model_path: PathBuf,
    load_time: Duration,
}

#[cfg(feature = "model-dpdfnet-onnx")]
impl DpdfNetSession {
    pub fn load(request: SessionLoadRequest<'_>) -> Result<Self> {
        validate_contract(request.manifest, request.route, request.artifact)?;
        let model_path = request.artifact_root.join(&request.artifact.entrypoint);
        ensure!(
            model_path.is_file(),
            "model artifact is missing: {}",
            model_path.display()
        );
        let route_options: OnnxRouteOptions = if request.route.options.is_null() {
            OnnxRouteOptions::default()
        } else {
            serde_json::from_value(request.route.options.clone())
                .context("DPDFNet ONNX route options are invalid")?
        };
        let options = OnnxSessionOptions {
            intra_threads: route_options.intra_threads.or(Some(1)),
            memory_pattern: route_options.memory_pattern.unwrap_or(true),
            prepacking: route_options.prepacking.unwrap_or(true),
        };
        let started = Instant::now();
        let session = OnnxSession::load_with_options(&model_path, &options)?;
        let metadata = parse_model_metadata(&session.custom_metadata()?)?;
        let adapter = DpdfNetAdapter::new(metadata.initial_state)?;
        let load_time = started.elapsed();
        Ok(Self {
            adapter,
            backend: OnnxFrameBackend {
                session,
                input_spec: request.manifest.contract.inputs[0].name.clone(),
                input_state: request.manifest.contract.inputs[1].name.clone(),
                output_spec: request.manifest.contract.outputs[0].name.clone(),
                output_state: request.manifest.contract.outputs[1].name.clone(),
            },
            model_path,
            load_time,
        })
    }

    /// Start a clean job while retaining the loaded model session and DSP plans.
    pub fn start_stream(&mut self) -> DpdfNetStream<'_> {
        self.adapter.start_stream(&mut self.backend)
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    pub fn load_ms(&self) -> f64 {
        duration_ms(self.load_time)
    }

    pub fn shared_dsp_bytes(&self) -> usize {
        self.adapter.shared_dsp_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    struct FakeBackend {
        mutate_state: bool,
        calls: usize,
    }

    impl FakeBackend {
        fn identity() -> Self {
            Self {
                mutate_state: false,
                calls: 0,
            }
        }

        fn stateful() -> Self {
            Self {
                mutate_state: true,
                calls: 0,
            }
        }
    }

    impl FrameBackend for FakeBackend {
        fn infer_frame(
            &mut self,
            noisy_spectrum: &[f32],
            recurrent_state: &mut Vec<f32>,
            enhanced_spectrum: &mut Vec<f32>,
        ) -> Result<()> {
            enhanced_spectrum.copy_from_slice(noisy_spectrum);
            if self.mutate_state {
                let gain = 1.0 + recurrent_state[1_000] * 0.000_1;
                for value in enhanced_spectrum {
                    *value *= gain;
                }
                recurrent_state[1_000] += 1.0;
            }
            self.calls += 1;
            Ok(())
        }
    }

    fn test_initial_state() -> Vec<f32> {
        let mut state = vec![0.0; STATE_SIZE];
        state[0] = 1.0;
        state
    }

    fn test_adapter() -> DpdfNetAdapter {
        DpdfNetAdapter::new(test_initial_state()).unwrap()
    }

    fn signal(length: usize) -> Vec<f32> {
        (0..length)
            .map(|index| {
                (index as f32 * 0.017).sin() * 0.08 + (index as f32 * 0.003_1).cos() * 0.02
            })
            .collect()
    }

    fn run_chunks(
        adapter: &DpdfNetAdapter,
        input: &[f32],
        chunks: &[usize],
    ) -> (Vec<f32>, StreamStats) {
        let mut backend = FakeBackend::identity();
        let mut stream = adapter.start_stream(&mut backend);
        let mut output = Vec::new();
        let mut cursor = 0;
        for &chunk in chunks {
            if cursor == input.len() {
                break;
            }
            let end = (cursor + chunk).min(input.len());
            output.extend(stream.push(&input[cursor..end]).unwrap());
            cursor = end;
        }
        if cursor < input.len() {
            output.extend(stream.push(&input[cursor..]).unwrap());
        }
        let finished = stream.finish().unwrap();
        output.extend_from_slice(&finished.samples);
        (output, finished.stats)
    }

    fn release_manifest() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-dpdfnet-1.0.0.json")).unwrap()
    }

    fn release_route<'a>(manifest: &'a ModelManifest, id: &str) -> (&'a Route, &'a Artifact) {
        let route = manifest.routes.iter().find(|route| route.id == id).unwrap();
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        (route, artifact)
    }

    #[test]
    fn exact_release_contract_is_accepted_and_core_drift_is_rejected() {
        let manifest = release_manifest();
        for route_id in ["onnx-int8-cpu-macos-arm64", "onnx-fp32-cpu-macos-arm64"] {
            let (route, artifact) = release_route(&manifest, route_id);
            validate_contract(&manifest, route, artifact).unwrap();
        }

        for (section, field, replacement) in [
            ("preprocess", "hop_length", serde_json::json!(481)),
            ("postprocess", "clamp", serde_json::json!([-1, 0.5])),
        ] {
            let mut value: serde_json::Value =
                serde_json::from_str(include_str!("../../catalog/release-dpdfnet-1.0.0.json"))
                    .unwrap();
            value["contract"][section][field] = replacement;
            let changed: ModelManifest = serde_json::from_value(value).unwrap();
            let (route, artifact) = release_route(&changed, "onnx-int8-cpu-macos-arm64");
            assert!(validate_contract(&changed, route, artifact).is_err());
        }
    }

    #[test]
    fn arbitrary_pcm_chunking_preserves_exact_output_and_frame_count() {
        let adapter = test_adapter();
        for length in [
            0, 1, 17, 479, 480, 481, 777, 959, 960, 961, 1_439, 1_440, 1_441, 8_123,
        ] {
            let input = signal(length);
            let (whole, whole_stats) = run_chunks(&adapter, &input, &[length.max(1)]);
            let (chunked, chunked_stats) =
                run_chunks(&adapter, &input, &[1, 7, 479, 2, 997, 31, 480, 3]);
            assert_eq!(whole, chunked, "chunking changed {length}-sample output");
            assert_eq!(whole.len(), length);
            assert_eq!(
                whole_stats.frames,
                (length + N_FFT) as u64 / HOP_LENGTH as u64 + 1
            );
            assert_eq!(whole_stats.input_samples, length as u64);
            assert_eq!(whole_stats.output_samples, length as u64);
            assert_eq!(chunked_stats.frames, whole_stats.frames);
        }
    }

    #[test]
    fn sequential_jobs_reset_recurrent_and_overlap_state() {
        let adapter = test_adapter();
        let input = signal(6_731);
        let mut backend = FakeBackend::stateful();

        let first = {
            let mut stream = adapter.start_stream(&mut backend);
            let mut output = stream.push(&input[..2_111]).unwrap();
            output.extend(stream.push(&input[2_111..]).unwrap());
            output.extend(stream.finish().unwrap().samples);
            output
        };
        let second = {
            let mut stream = adapter.start_stream(&mut backend);
            let mut output = stream.push(&input).unwrap();
            output.extend(stream.finish().unwrap().samples);
            output
        };
        assert_eq!(first, second);
    }

    #[test]
    #[ignore = "long fixed-working-set proof; run explicitly in release mode"]
    fn fifty_hundred_and_one_hundred_fifty_seconds_keep_fixed_working_set() {
        let adapter = test_adapter();
        assert_eq!(adapter.shared_dsp_bytes(), 231_504);
        for seconds in [50, 100, 150] {
            let mut backend = FakeBackend::identity();
            let mut stream = adapter.start_stream(&mut backend);
            let initial = stream.working_set().heap_capacity_bytes;
            let hop = signal(HOP_LENGTH);
            let mut emitted = 0_usize;
            for _ in 0..seconds * (SAMPLE_RATE as usize / HOP_LENGTH) {
                emitted += stream.push(&hop).unwrap().len();
                assert_eq!(stream.working_set().heap_capacity_bytes, initial);
                assert!(stream.working_set().buffered_input_samples <= HOP_LENGTH);
            }
            let finished = stream.finish().unwrap();
            emitted += finished.samples.len();
            assert_eq!(emitted, seconds * SAMPLE_RATE as usize);
            assert_eq!(finished.stats.output_samples as usize, emitted);
            assert!(
                initial < 320 * 1024,
                "unexpected job working set: {initial}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_pcm_without_consuming_the_job() {
        let adapter = test_adapter();
        let mut backend = FakeBackend::identity();
        let mut stream = adapter.start_stream(&mut backend);
        let error = stream.push(&[f32::NAN]).unwrap_err();
        assert!(error.to_string().contains("non-finite"));
        assert_eq!(stream.stats().input_samples, 0);
        assert_eq!(stream.push(&[0.0; 481]).unwrap().len(), 0);
    }

    #[test]
    fn backend_failure_poisons_only_the_current_job() {
        struct FailingBackend {
            fail: bool,
        }
        impl FrameBackend for FailingBackend {
            fn infer_frame(
                &mut self,
                noisy_spectrum: &[f32],
                _state: &mut Vec<f32>,
                enhanced: &mut Vec<f32>,
            ) -> Result<()> {
                if self.fail {
                    self.fail = false;
                    bail!("injected failure")
                }
                enhanced.copy_from_slice(noisy_spectrum);
                Ok(())
            }
        }

        let adapter = test_adapter();
        let mut backend = FailingBackend { fail: true };
        {
            let mut stream = adapter.start_stream(&mut backend);
            assert!(stream.push(&[0.0; 481]).is_err());
            assert!(
                stream
                    .push(&[0.0])
                    .unwrap_err()
                    .to_string()
                    .contains("unusable")
            );
        }
        let mut replacement = adapter.start_stream(&mut backend);
        replacement.push(&[0.0; 481]).unwrap();
        replacement.finish().unwrap();
    }

    #[test]
    fn vorbis_window_and_frozen_centered_stft_bins_match_reference() {
        let window = vorbis_window(N_FFT);
        assert!((window[0] - 4.205_491_7e-6).abs() < 1.0e-12);
        assert!((window[479] - 1.0).abs() < 1.0e-7);
        let maximum_power_error = (0..HOP_LENGTH)
            .map(|index| (window[index].powi(2) + window[index + HOP_LENGTH].powi(2) - 1.0).abs())
            .fold(0.0_f32, f32::max);
        assert!(maximum_power_error < 2.0e-7);

        let source = (0..17).map(|value| value as f32 / 16.0).collect::<Vec<_>>();
        let mut analysis = vec![0.0; N_FFT];
        for (destination, sample) in analysis.iter_mut().take(HOP_LENGTH).enumerate() {
            *sample = source.get(HOP_LENGTH - destination).copied().unwrap_or(0.0);
        }
        analysis[HOP_LENGTH..HOP_LENGTH + source.len()].copy_from_slice(&source);
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(N_FFT);
        let mut fft = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut spectrum = vec![0.0; SPEC_VALUES];
        stft_window_into(
            &analysis,
            &window,
            forward.as_ref(),
            &mut fft,
            &mut spectrum,
        );
        for (bin, real, imaginary) in [
            (0, 16.999_94, 0.0),
            (1, -16.950_453, -7.742_187e-7),
            (7, -14.649_24, -5.049_088e-6),
            (100, -3.504_871, -5.897_53e-7),
            (480, 0.999_989_15, 0.0),
        ] {
            assert!((spectrum[bin * 2] - real).abs() < 3.0e-5);
            assert!((spectrum[bin * 2 + 1] - imaginary).abs() < 3.0e-5);
        }
    }

    #[test]
    fn metadata_builds_the_required_nonzero_state_prefix() {
        let mut metadata = BTreeMap::from([
            ("model_type".to_string(), "dpdfnet".to_string()),
            ("sample_rate".to_string(), SAMPLE_RATE.to_string()),
            ("n_fft".to_string(), N_FFT.to_string()),
            ("hop_length".to_string(), HOP_LENGTH.to_string()),
            ("window_length".to_string(), N_FFT.to_string()),
            ("window_type".to_string(), "vorbis".to_string()),
            ("normalized".to_string(), "0".to_string()),
            ("center".to_string(), "1".to_string()),
            ("pad_mode".to_string(), "reflect".to_string()),
            ("freq_bins".to_string(), FREQ_BINS.to_string()),
            ("state_size".to_string(), STATE_SIZE.to_string()),
            (
                "erb_norm_state_size".to_string(),
                ERB_NORM_STATE_SIZE.to_string(),
            ),
            (
                "spec_norm_state_size".to_string(),
                SPEC_NORM_STATE_SIZE.to_string(),
            ),
        ]);
        metadata.insert(
            "erb_norm_init".into(),
            (0..ERB_NORM_STATE_SIZE)
                .map(|index| (index as f32 + 1.0).to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
        metadata.insert(
            "spec_norm_init".into(),
            (0..SPEC_NORM_STATE_SIZE)
                .map(|index| (-(index as f32) - 1.0).to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
        let parsed = parse_model_metadata(&metadata).unwrap();
        assert_eq!(parsed.initial_state[0], 1.0);
        assert_eq!(parsed.initial_state[ERB_NORM_STATE_SIZE - 1], 481.0);
        assert_eq!(parsed.initial_state[ERB_NORM_STATE_SIZE], -1.0);
        assert_eq!(
            parsed.initial_state[ERB_NORM_STATE_SIZE + SPEC_NORM_STATE_SIZE],
            0.0
        );
    }

    #[cfg(all(
        feature = "model-dpdfnet-onnx",
        any(target_os = "macos", target_os = "windows")
    ))]
    #[test]
    #[ignore = "requires a local DPDFNet INT8 artifact and pinned ORT_DYLIB_PATH"]
    fn local_int8_artifact_load_once_smoke() {
        let root = std::env::var_os("VALLE_DPDFNET_ARTIFACT_ROOT")
            .expect("VALLE_DPDFNET_ARTIFACT_ROOT must name the installed DPDFNet artifact");
        let root = Path::new(&root);
        let manifest = release_manifest();
        let (route, artifact) = release_route(&manifest, "onnx-int8-cpu-macos-arm64");
        let entrypoint = root.join(&artifact.entrypoint);
        assert!(
            entrypoint.is_file(),
            "local DPDFNet INT8 artifact is missing: {}",
            entrypoint.display()
        );
        let mut session = DpdfNetSession::load(SessionLoadRequest {
            manifest: &manifest,
            route,
            artifact,
            artifact_root: root,
        })
        .unwrap();
        assert!(session.load_ms() > 0.0);
        let input = signal(SAMPLE_RATE as usize / 10);
        let mut first_output = None;
        for _ in 0..2 {
            let mut stream = session.start_stream();
            let mut output = stream.push(&input).unwrap();
            let finished = stream.finish().unwrap();
            output.extend(finished.samples);
            assert_eq!(output.len(), input.len());
            assert!(output.iter().all(|sample| sample.is_finite()));
            if let Some(first) = &first_output {
                assert_eq!(&output, first, "reused session leaked prior job state");
            } else {
                first_output = Some(output);
            }
        }
    }
}
