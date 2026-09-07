//! Typed native CPU adapters shared by Qwen3 ASR and Qwen3 ForcedAligner releases.
//!
//! This module owns model-specific session and inference semantics. It accepts already-decoded
//! 16 kHz mono PCM and never opens or writes complete media files; those file workflows belong to
//! Valle. Keeping transcription and alignment as separate session types also lets a caller drop
//! the ASR weights before loading the aligner weights.

use std::path::Path;
#[cfg(feature = "model-qwen-native")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(feature = "model-qwen-native", test))]
use crate::models::spec::Backend;
use crate::models::spec::{Artifact, ModelManifest, Route};
#[cfg(any(feature = "model-qwen-native", test))]
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const ASR_MODEL_ID: &str = "qwen3-asr-0.6b";
pub const ALIGNER_MODEL_ID: &str = "qwen3-aligner-0.6b";
pub const ASR_RELEASE_VERSION: &str = "1.0.0";
pub const ALIGNER_RELEASE_VERSION: &str = "1.0.0";
pub const ASR_ADAPTER: &str = "qwen3-asr-transcription";
pub const ALIGNER_ADAPTER: &str = "qwen3-forced-alignment";
pub const CONTRACT_VERSION: u32 = 1;
pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const MAX_ALIGNER_AUDIO_SECONDS: u32 = 300;
/// Exact release-owned license file used to complete a fully offline legacy-cache migration.
pub const APACHE_2_LICENSE_PATH: &str = "card/LICENSE";
pub const APACHE_2_LICENSE_BYTES: &[u8] = include_bytes!("../../catalog/qwen-LICENSE");

#[derive(Clone, Copy)]
pub struct OpenRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
    /// Maximum number of native CPU kernel threads available to this session.
    pub cpu_threads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A bounded, already-decoded 16 kHz mono PCM window supplied by the host.
///
/// `start_sample` is the window's position on the complete source timeline. The
/// adapter never retains this slice after a call.
#[derive(Debug, Clone, Copy)]
pub struct PcmChunk<'a> {
    samples: &'a [f32],
    start_sample: u64,
}

impl<'a> PcmChunk<'a> {
    pub const fn new(samples: &'a [f32], start_sample: u64) -> Self {
        Self {
            samples,
            start_sample,
        }
    }

    pub const fn samples(self) -> &'a [f32] {
        self.samples
    }

    pub const fn start_sample(self) -> u64 {
        self.start_sample
    }

    pub fn start_ms(self) -> u64 {
        samples_to_millis(self.start_sample)
    }
}

/// One independently transcribed chunk.
///
/// Segment timestamps are absolute on the source timeline. The host owns
/// ordering, overlap de-duplication, text joining and final track duration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkTranscription {
    pub start_sample: u64,
    pub sample_count: usize,
    pub language: String,
    pub duration_ms: u64,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcription {
    pub language: String,
    pub duration_ms: u64,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignedUnit {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignedTranscription {
    pub transcription: Transcription,
    pub units: Vec<AlignedUnit>,
}

/// Exclusive ownership of qwen-asr's process-global kernel pool.
///
/// qwen-asr 0.11 exposes one mutable global thread count and documents that its pool cannot be
/// dispatched concurrently by independent callers. Failing a competing open is preferable to
/// racing two jobs with different resource ceilings or deadlocking inside the shared pool.
#[cfg(feature = "model-qwen-native")]
struct KernelPoolLease;

#[cfg(feature = "model-qwen-native")]
static KERNEL_POOL_LEASED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "model-qwen-native")]
impl KernelPoolLease {
    fn acquire(cpu_threads: usize) -> Result<Self> {
        validate_cpu_threads(cpu_threads)?;
        KERNEL_POOL_LEASED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Qwen native kernel pool is already in use by another inference session"
                )
            })?;
        qwen_asr::kernels::set_threads(cpu_threads);
        Ok(Self)
    }
}

#[cfg(feature = "model-qwen-native")]
impl Drop for KernelPoolLease {
    fn drop(&mut self) {
        KERNEL_POOL_LEASED.store(false, Ordering::Release);
    }
}

#[cfg(feature = "model-qwen-native")]
pub struct QwenAsrSession {
    context: qwen_asr::context::QwenCtx,
    max_chunk_samples: usize,
    _kernel_pool_lease: KernelPoolLease,
}

#[cfg(feature = "model-qwen-native")]
impl QwenAsrSession {
    pub fn open(request: OpenRequest<'_>) -> Result<Self> {
        validate_open_request(request, ModelKind::Asr)?;
        let kernel_pool_lease = KernelPoolLease::acquire(request.cpu_threads)?;
        let mut context = load_context(request.artifact_root)?;
        ensure!(
            !context.config.is_aligner(),
            "Qwen ASR release resolved to forced-aligner weights"
        );
        let options: AsrRouteOptions = serde_json::from_value(request.route.options.clone())
            .context("Qwen ASR route options are invalid")?;
        ensure!(
            options.segment_seconds.is_finite() && options.segment_seconds > 0.0,
            "Qwen ASR segment_seconds must be finite and positive"
        );
        context.segment_sec = options.segment_seconds;
        let max_chunk_samples = seconds_to_samples(options.segment_seconds)?;
        Ok(Self {
            context,
            max_chunk_samples,
            _kernel_pool_lease: kernel_pool_lease,
        })
    }

    /// Maximum number of samples accepted by [`Self::transcribe_chunk`].
    pub const fn max_chunk_samples(&self) -> usize {
        self.max_chunk_samples
    }

    /// Transcribe one bounded window without retaining cross-window decode state.
    ///
    /// Language mode is applied afresh on every call. Empty tail windows are a
    /// valid no-op. Returned segment timestamps include `chunk.start_sample`.
    /// Callers that intentionally overlap windows must merge/de-duplicate the
    /// returned text and segments themselves.
    pub fn transcribe_chunk(
        &mut self,
        chunk: PcmChunk<'_>,
        language: Option<&str>,
    ) -> Result<ChunkTranscription> {
        validate_chunk(chunk, self.max_chunk_samples)?;
        let forced_language = configure_asr_language(&mut self.context, language)?;
        if chunk.samples.is_empty() {
            return Ok(empty_chunk_transcription(chunk, forced_language));
        }
        let transcription = transcribe_local(&mut self.context, chunk.samples)?;
        Ok(offset_chunk_transcription(chunk, transcription))
    }

    /// Whole-slice convenience for already bounded inputs.
    ///
    /// Long-media callers should repeatedly invoke [`Self::transcribe_chunk`] so their decoded
    /// audio workspace remains fixed-size. `language` accepts a supported English language name
    /// or ISO-639 code; `None` enables the model's language-detection path.
    pub fn transcribe(&mut self, samples: &[f32], language: Option<&str>) -> Result<Transcription> {
        validate_samples(samples)?;
        configure_asr_language(&mut self.context, language)?;
        transcribe_local(&mut self.context, samples)
    }
}

#[cfg(feature = "model-qwen-native")]
pub struct QwenAlignerSession {
    context: qwen_asr::context::QwenCtx,
    max_chunk_samples: usize,
    _kernel_pool_lease: KernelPoolLease,
}

#[cfg(feature = "model-qwen-native")]
impl QwenAlignerSession {
    pub fn open(request: OpenRequest<'_>) -> Result<Self> {
        validate_open_request(request, ModelKind::Aligner)?;
        let kernel_pool_lease = KernelPoolLease::acquire(request.cpu_threads)?;
        let context = load_context(request.artifact_root)?;
        ensure!(
            context.config.is_aligner(),
            "Qwen aligner release resolved to transcription weights"
        );
        Ok(Self {
            context,
            max_chunk_samples: MAX_ALIGNER_AUDIO_SECONDS as usize * SAMPLE_RATE_HZ as usize,
            _kernel_pool_lease: kernel_pool_lease,
        })
    }

    pub const fn max_chunk_samples(&self) -> usize {
        self.max_chunk_samples
    }

    /// Align one bounded transcript window and return absolute source timestamps.
    pub fn align_chunk(
        &mut self,
        chunk: PcmChunk<'_>,
        text: &str,
        language: &str,
    ) -> Result<Vec<AlignedUnit>> {
        validate_chunk(chunk, self.max_chunk_samples)?;
        if chunk.samples.is_empty() {
            ensure!(
                text.trim().is_empty(),
                "cannot align non-empty text against an empty audio chunk"
            );
            return Ok(Vec::new());
        }
        let offset_ms = chunk.start_ms();
        Ok(self
            .align_local(chunk.samples, text, language)?
            .into_iter()
            .map(|unit| AlignedUnit {
                text: unit.text,
                start_ms: offset_ms.saturating_add(unit.start_ms),
                end_ms: offset_ms.saturating_add(unit.end_ms),
            })
            .collect())
    }

    /// Align a known transcript against 16 kHz mono `f32` PCM.
    pub fn align(
        &mut self,
        samples: &[f32],
        text: &str,
        language: &str,
    ) -> Result<Vec<AlignedUnit>> {
        validate_chunk(PcmChunk::new(samples, 0), self.max_chunk_samples)?;
        self.align_local(samples, text, language)
    }

    fn align_local(
        &mut self,
        samples: &[f32],
        text: &str,
        language: &str,
    ) -> Result<Vec<AlignedUnit>> {
        validate_samples(samples)?;
        ensure!(!text.trim().is_empty(), "alignment text must not be empty");
        let language = canonical_aligner_language(language)
            .with_context(|| format!("unsupported Qwen aligner language {language:?}"))?;
        let aligned = qwen_asr::align::forced_align(&mut self.context, samples, text, language)
            .context("Qwen forced alignment failed")?;
        let mut last_start = 0_u64;
        Ok(aligned
            .into_iter()
            .map(|unit| {
                let start_ms = f32_millis(unit.start_ms).max(last_start);
                let end_ms = f32_millis(unit.end_ms).max(start_ms);
                last_start = start_ms;
                AlignedUnit {
                    text: unit.text,
                    start_ms,
                    end_ms,
                }
            })
            .collect())
    }
}

#[cfg(feature = "model-qwen-native")]
fn configure_asr_language(
    context: &mut qwen_asr::context::QwenCtx,
    language: Option<&str>,
) -> Result<Option<&'static str>> {
    // Language detection belongs to this window; never fall back to the
    // preceding call's detected header if the current window has no header.
    context.detected_language = None;
    match language {
        Some(language) => {
            let language = canonical_language(language)
                .with_context(|| format!("unsupported Qwen ASR language {language:?}"))?;
            context
                .set_force_language(language)
                .map_err(|()| anyhow::anyhow!("qwen-asr rejected language {language:?}"))?;
            Ok(Some(language))
        }
        None => {
            // This also clears any forced language from the preceding chunk.
            context.set_multilingual(true);
            Ok(None)
        }
    }
}

#[cfg(feature = "model-qwen-native")]
fn transcribe_local(
    context: &mut qwen_asr::context::QwenCtx,
    samples: &[f32],
) -> Result<Transcription> {
    let output = qwen_asr::transcribe::transcribe_full(context, None, samples, None)
        .context("Qwen ASR produced no transcription")?;
    Ok(Transcription {
        language: output.language,
        duration_ms: output.duration_ms,
        text: output.text,
        segments: output
            .segments
            .into_iter()
            .map(|segment| TranscriptSegment {
                text: segment.text,
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
            })
            .collect(),
    })
}

#[cfg(any(feature = "model-qwen-native", test))]
fn empty_chunk_transcription(
    chunk: PcmChunk<'_>,
    forced_language: Option<&str>,
) -> ChunkTranscription {
    ChunkTranscription {
        start_sample: chunk.start_sample,
        sample_count: 0,
        language: forced_language
            .and_then(language_to_code)
            .unwrap_or_default()
            .to_owned(),
        duration_ms: 0,
        text: String::new(),
        segments: Vec::new(),
    }
}

#[cfg(any(feature = "model-qwen-native", test))]
fn offset_chunk_transcription(
    chunk: PcmChunk<'_>,
    transcription: Transcription,
) -> ChunkTranscription {
    let offset_ms = chunk.start_ms();
    ChunkTranscription {
        start_sample: chunk.start_sample,
        sample_count: chunk.samples.len(),
        language: transcription.language,
        duration_ms: transcription.duration_ms,
        text: transcription.text,
        segments: transcription
            .segments
            .into_iter()
            .map(|segment| TranscriptSegment {
                text: segment.text,
                start_ms: offset_ms.saturating_add(segment.start_ms),
                end_ms: offset_ms.saturating_add(segment.end_ms),
            })
            .collect(),
    }
}

/// Whole-slice convenience that guarantees the two large models are never resident together.
///
/// ASR runs to completion inside the first scope. Its session is dropped before the aligner is
/// opened, then each non-empty transcript segment is aligned and offset to the complete input.
/// Long-media hosts should instead spool bounded PCM, call `transcribe_chunk` for each window,
/// drop the ASR session, then reopen bounded segment slices for `align_chunk`.
#[cfg(feature = "model-qwen-native")]
pub fn transcribe_then_align(
    asr: OpenRequest<'_>,
    aligner: OpenRequest<'_>,
    samples: &[f32],
    language: Option<&str>,
) -> Result<AlignedTranscription> {
    let transcription = {
        let mut session = QwenAsrSession::open(asr)?;
        session.transcribe(samples, language)?
    };
    let alignment_language = language
        .and_then(canonical_language)
        .or_else(|| canonical_language(&transcription.language))
        .context("Qwen ASR did not produce a supported alignment language")?;
    let mut session = QwenAlignerSession::open(aligner)?;
    let mut units = Vec::new();
    let mut last_start = 0_u64;
    for segment in &transcription.segments {
        let text = segment.text.trim();
        if text.is_empty() || segment.end_ms <= segment.start_ms {
            continue;
        }
        let sample_start = millis_to_samples(segment.start_ms).min(samples.len());
        let sample_end = millis_to_samples(segment.end_ms).min(samples.len());
        if sample_end <= sample_start {
            continue;
        }
        for unit in session.align(&samples[sample_start..sample_end], text, alignment_language)? {
            let start_ms = segment
                .start_ms
                .saturating_add(unit.start_ms)
                .max(last_start);
            let end_ms = segment.start_ms.saturating_add(unit.end_ms).max(start_ms);
            last_start = start_ms;
            units.push(AlignedUnit {
                text: unit.text,
                start_ms,
                end_ms,
            });
        }
    }
    Ok(AlignedTranscription {
        transcription,
        units,
    })
}

#[cfg(feature = "model-qwen-native")]
#[derive(Debug, Deserialize)]
struct AsrRouteOptions {
    #[serde(default = "default_segment_seconds")]
    segment_seconds: f32,
}

#[cfg(feature = "model-qwen-native")]
fn default_segment_seconds() -> f32 {
    30.0
}

#[cfg(any(feature = "model-qwen-native", test))]
#[derive(Clone, Copy)]
enum ModelKind {
    Asr,
    Aligner,
}

#[cfg(any(feature = "model-qwen-native", test))]
fn validate_cpu_threads(cpu_threads: usize) -> Result<()> {
    ensure!(cpu_threads > 0, "Qwen native cpu_threads must be positive");
    Ok(())
}

#[cfg(any(feature = "model-qwen-native", test))]
fn validate_open_request(request: OpenRequest<'_>, kind: ModelKind) -> Result<()> {
    validate_cpu_threads(request.cpu_threads)?;
    request.manifest.validate()?;
    let (model_id, task, adapter) = match kind {
        ModelKind::Asr => (ASR_MODEL_ID, "speech-transcription", ASR_ADAPTER),
        ModelKind::Aligner => (ALIGNER_MODEL_ID, "forced-alignment", ALIGNER_ADAPTER),
    };
    ensure!(
        request.manifest.model.id == model_id && request.manifest.model.task == task,
        "manifest is not the expected {model_id} {task} release"
    );
    ensure!(
        request.manifest.contract.adapter == adapter
            && request.manifest.contract.version == CONTRACT_VERSION,
        "unsupported Qwen adapter contract {}@{}",
        request.manifest.contract.adapter,
        request.manifest.contract.version
    );
    validate_contract_shape(request.manifest, kind)?;
    ensure!(
        request.route.backend == Backend::NativeCpu,
        "Qwen native adapter requires native-cpu, got {}",
        request.route.backend
    );
    ensure!(
        request.route.artifact == request.artifact.id,
        "route and artifact disagree"
    );
    ensure!(
        request.artifact.format == "safetensors-directory"
            && request.artifact.precision == "bf16"
            && request.artifact.entrypoint == "model.safetensors",
        "Qwen native adapter requires the pinned BF16 safetensors directory artifact"
    );
    for required in [
        "config.json",
        "merges.txt",
        "model.safetensors",
        "vocab.json",
    ] {
        ensure!(
            request
                .artifact
                .files
                .iter()
                .any(|file| file.path == required),
            "Qwen artifact does not declare required file {required:?}"
        );
        ensure!(
            request.artifact_root.join(required).is_file(),
            "Qwen artifact file is missing: {}",
            request.artifact_root.join(required).display()
        );
    }
    Ok(())
}

#[cfg(any(feature = "model-qwen-native", test))]
fn validate_contract_shape(manifest: &ModelManifest, kind: ModelKind) -> Result<()> {
    let contract = &manifest.contract;
    type ExpectedTensor = (&'static str, &'static str, &'static str);
    let (expected_inputs, expected_outputs): (&[ExpectedTensor], &[ExpectedTensor]) = match kind {
        ModelKind::Asr => (
            &[
                ("audio", "f32", "mono-samples"),
                ("language", "utf8", "scalar"),
            ],
            &[
                ("text", "utf8", "scalar"),
                ("segments", "record", "text-start_ms-end_ms"),
            ],
        ),
        ModelKind::Aligner => (
            &[
                ("audio", "f32", "mono-samples"),
                ("text", "utf8", "scalar"),
                ("language", "utf8", "scalar"),
            ],
            &[("units", "record", "text-start_ms-end_ms")],
        ),
    };
    ensure!(
        tensor_contract_matches(&contract.inputs, expected_inputs)
            && tensor_contract_matches(&contract.outputs, expected_outputs),
        "Qwen manifest tensor contract does not match the adapter"
    );
    let preprocess: PcmContract = serde_json::from_value(contract.preprocess.clone())
        .context("Qwen PCM preprocess contract is invalid")?;
    ensure!(
        preprocess.sample_rate == SAMPLE_RATE_HZ
            && preprocess.channels == 1
            && preprocess.sample_format == "f32"
            && preprocess.sample_range == [-1.0, 1.0],
        "Qwen adapter requires 16 kHz mono f32 PCM in [-1, 1]"
    );
    if matches!(kind, ModelKind::Aligner) {
        ensure!(
            preprocess.maximum_audio_seconds == Some(300),
            "Qwen aligner contract must preserve the five-minute input limit"
        );
    }
    Ok(())
}

#[cfg(any(feature = "model-qwen-native", test))]
fn tensor_contract_matches(
    actual: &[crate::models::spec::TensorSpec],
    expected: &[(&str, &str, &str)],
) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(tensor, (name, dtype, layout))| {
                tensor.name == *name && tensor.dtype == *dtype && tensor.layout == *layout
            })
}

#[cfg(any(feature = "model-qwen-native", test))]
#[derive(Deserialize)]
struct PcmContract {
    sample_rate: u32,
    channels: u16,
    sample_format: String,
    sample_range: [f32; 2],
    #[serde(default)]
    maximum_audio_seconds: Option<u32>,
}

#[cfg(feature = "model-qwen-native")]
fn load_context(root: &Path) -> Result<qwen_asr::context::QwenCtx> {
    let root = root
        .to_str()
        .with_context(|| format!("Qwen artifact root is not UTF-8: {}", root.display()))?;
    qwen_asr::context::QwenCtx::load(root)
        .with_context(|| format!("failed to load Qwen model from {root}"))
}

#[cfg(any(feature = "model-qwen-native", test))]
fn validate_samples(samples: &[f32]) -> Result<()> {
    ensure!(!samples.is_empty(), "Qwen input audio is empty");
    ensure!(
        samples
            .iter()
            .all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample)),
        "Qwen input must be finite 16 kHz mono f32 PCM in [-1, 1]"
    );
    Ok(())
}

#[cfg(any(feature = "model-qwen-native", test))]
fn validate_chunk(chunk: PcmChunk<'_>, max_chunk_samples: usize) -> Result<()> {
    ensure!(
        chunk.samples.len() <= max_chunk_samples,
        "Qwen audio chunk has {} samples; the selected route allows at most {max_chunk_samples}",
        chunk.samples.len()
    );
    if chunk.samples.is_empty() {
        return Ok(());
    }
    validate_samples(chunk.samples)
}

#[cfg(feature = "model-qwen-native")]
fn seconds_to_samples(seconds: f32) -> Result<usize> {
    let samples = f64::from(seconds) * f64::from(SAMPLE_RATE_HZ);
    ensure!(
        samples.is_finite() && samples > 0.0 && samples <= usize::MAX as f64,
        "Qwen ASR segment_seconds cannot be represented as a sample count"
    );
    Ok(samples.ceil() as usize)
}

fn samples_to_millis(samples: u64) -> u64 {
    samples
        .saturating_mul(1_000)
        .checked_div(u64::from(SAMPLE_RATE_HZ))
        .unwrap_or(0)
}

#[cfg(any(feature = "model-qwen-native", test))]
fn canonical_language(language: &str) -> Option<&'static str> {
    let normalized = language.trim().to_ascii_lowercase();
    LANGUAGE_TABLE
        .iter()
        .find(|(name, code)| name.eq_ignore_ascii_case(language.trim()) || *code == normalized)
        .map(|(name, _)| *name)
}

#[cfg(any(feature = "model-qwen-native", test))]
fn canonical_aligner_language(language: &str) -> Option<&'static str> {
    let language = canonical_language(language)?;
    ALIGNER_LANGUAGES.contains(&language).then_some(language)
}

#[cfg(any(feature = "model-qwen-native", test))]
fn language_to_code(language: &str) -> Option<&'static str> {
    LANGUAGE_TABLE
        .iter()
        .find(|(name, _)| *name == language)
        .map(|(_, code)| *code)
}

#[cfg(feature = "model-qwen-native")]
fn f32_millis(value: f32) -> u64 {
    if value.is_finite() && value > 0.0 {
        value.round() as u64
    } else {
        0
    }
}

#[cfg(feature = "model-qwen-native")]
fn millis_to_samples(milliseconds: u64) -> usize {
    milliseconds
        .saturating_mul(u64::from(SAMPLE_RATE_HZ))
        .div_ceil(1_000)
        .try_into()
        .unwrap_or(usize::MAX)
}

#[cfg(any(feature = "model-qwen-native", test))]
const LANGUAGE_TABLE: &[(&str, &str)] = &[
    ("Chinese", "zh"),
    ("English", "en"),
    ("Cantonese", "yue"),
    ("Arabic", "ar"),
    ("German", "de"),
    ("French", "fr"),
    ("Spanish", "es"),
    ("Portuguese", "pt"),
    ("Indonesian", "id"),
    ("Italian", "it"),
    ("Korean", "ko"),
    ("Russian", "ru"),
    ("Thai", "th"),
    ("Vietnamese", "vi"),
    ("Japanese", "ja"),
    ("Turkish", "tr"),
    ("Hindi", "hi"),
    ("Malay", "ms"),
    ("Dutch", "nl"),
    ("Swedish", "sv"),
    ("Danish", "da"),
    ("Finnish", "fi"),
    ("Polish", "pl"),
    ("Czech", "cs"),
    ("Filipino", "fil"),
    ("Persian", "fa"),
    ("Greek", "el"),
    ("Romanian", "ro"),
    ("Hungarian", "hu"),
    ("Macedonian", "mk"),
];

#[cfg(any(feature = "model-qwen-native", test))]
const ALIGNER_LANGUAGES: &[&str] = &[
    "Chinese",
    "English",
    "Cantonese",
    "French",
    "German",
    "Italian",
    "Japanese",
    "Korean",
    "Portuguese",
    "Russian",
    "Spanish",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_cpu_thread_budget_must_be_positive() {
        assert!(validate_cpu_threads(1).is_ok());
        assert!(validate_cpu_threads(0).is_err());
    }

    #[cfg(feature = "model-qwen-native")]
    #[test]
    fn native_kernel_pool_has_one_exclusive_thread_budget_lease() {
        let first = KernelPoolLease::acquire(2).unwrap();
        assert_eq!(qwen_asr::kernels::get_num_threads(), 2);

        let overlap = match KernelPoolLease::acquire(1) {
            Ok(_) => panic!("a second Qwen session must not share the global kernel pool"),
            Err(error) => error,
        };
        assert!(overlap.to_string().contains("already in use"));

        drop(first);
        let second = KernelPoolLease::acquire(1).unwrap();
        assert_eq!(qwen_asr::kernels::get_num_threads(), 1);
        drop(second);
    }

    #[test]
    fn language_names_and_iso_codes_resolve_to_one_contract() {
        assert_eq!(canonical_language("zh"), Some("Chinese"));
        assert_eq!(canonical_language("English"), Some("English"));
        assert_eq!(canonical_language("YUE"), Some("Cantonese"));
        assert_eq!(canonical_language("unknown"), None);
        assert_eq!(canonical_aligner_language("ja"), Some("Japanese"));
        assert_eq!(canonical_aligner_language("ar"), None);
    }

    #[test]
    fn samples_must_obey_the_published_pcm_contract() {
        assert!(validate_samples(&[0.0, -1.0, 1.0]).is_ok());
        assert!(validate_samples(&[]).is_err());
        assert!(validate_samples(&[f32::NAN]).is_err());
        assert!(validate_samples(&[1.01]).is_err());
    }

    #[test]
    fn chunks_enforce_bounds_but_accept_an_empty_tail() {
        let samples = [0.0_f32; 4];
        assert!(validate_chunk(PcmChunk::new(&samples, 16_000), 4).is_ok());
        assert!(validate_chunk(PcmChunk::new(&samples, 16_000), 3).is_err());

        let tail = PcmChunk::new(&[], 32_000);
        assert!(validate_chunk(tail, 4).is_ok());
        let output = empty_chunk_transcription(tail, Some("English"));
        assert_eq!(output.start_sample, 32_000);
        assert_eq!(output.sample_count, 0);
        assert_eq!(output.language, "en");
        assert_eq!(output.duration_ms, 0);
        assert!(output.text.is_empty());
        assert!(output.segments.is_empty());
    }

    #[test]
    fn chunk_offsets_are_absolute_and_do_not_retain_previous_window_state() {
        let local = Transcription {
            language: "en".into(),
            duration_ms: 500,
            text: "hello".into(),
            segments: vec![TranscriptSegment {
                text: "hello".into(),
                start_ms: 100,
                end_ms: 400,
            }],
        };
        let samples = [0.0_f32; 8_000];
        let first = offset_chunk_transcription(PcmChunk::new(&samples, 0), local.clone());
        let second = offset_chunk_transcription(PcmChunk::new(&samples, 32_000), local.clone());
        let repeated = offset_chunk_transcription(PcmChunk::new(&samples, 0), local);

        assert_eq!(first, repeated);
        assert_eq!(first.segments[0].start_ms, 100);
        assert_eq!(second.segments[0].start_ms, 2_100);
        assert_eq!(second.segments[0].end_ms, 2_400);
        assert_eq!(second.start_sample, 32_000);
        assert_eq!(second.sample_count, samples.len());
    }

    #[test]
    fn both_committed_release_contracts_match_the_shared_adapter() {
        let asr: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-asr-0.6b-1.0.0.json"
        ))
        .unwrap();
        let aligner: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-aligner-0.6b-1.0.0.json"
        ))
        .unwrap();
        assert_eq!(asr.model.version, ASR_RELEASE_VERSION);
        assert_eq!(aligner.model.version, ALIGNER_RELEASE_VERSION);
        assert_contract_shape(&asr, ModelKind::Asr);
        assert_contract_shape(&aligner, ModelKind::Aligner);
    }

    #[cfg(feature = "model-qwen-native")]
    #[test]
    #[ignore = "requires VALLE_QWEN_ASR_DIR, VALLE_QWEN_ALIGNER_DIR and VALLE_QWEN_FIXTURE_WAV"]
    fn real_fixture_transcribes_then_aligns_with_sequential_sessions() {
        let asr_root = std::env::var_os("VALLE_QWEN_ASR_DIR")
            .expect("VALLE_QWEN_ASR_DIR must name the pinned ASR artifact root");
        let aligner_root = std::env::var_os("VALLE_QWEN_ALIGNER_DIR")
            .expect("VALLE_QWEN_ALIGNER_DIR must name the pinned aligner artifact root");
        let wav = std::env::var_os("VALLE_QWEN_FIXTURE_WAV")
            .expect("VALLE_QWEN_FIXTURE_WAV must name a 16 kHz mono float WAV");
        let mut reader = hound::WavReader::open(wav).unwrap();
        assert_eq!(reader.spec().sample_rate, SAMPLE_RATE_HZ);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
        let samples: Vec<f32> = reader
            .samples::<f32>()
            .map(|sample| sample.unwrap())
            .collect();

        let asr: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-asr-0.6b-1.0.0.json"
        ))
        .unwrap();
        let aligner: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-aligner-0.6b-1.0.0.json"
        ))
        .unwrap();
        let asr_route = &asr.routes[0];
        let aligner_route = &aligner.routes[0];
        let output = transcribe_then_align(
            OpenRequest {
                manifest: &asr,
                route: asr_route,
                artifact: asr.artifact(&asr_route.artifact).unwrap(),
                artifact_root: Path::new(&asr_root),
                cpu_threads: 1,
            },
            OpenRequest {
                manifest: &aligner,
                route: aligner_route,
                artifact: aligner.artifact(&aligner_route.artifact).unwrap(),
                artifact_root: Path::new(&aligner_root),
                cpu_threads: 1,
            },
            &samples,
            Some("en"),
        )
        .unwrap();
        assert!(!output.transcription.text.trim().is_empty());
        assert!(!output.transcription.segments.is_empty());
        assert!(!output.units.is_empty());
        assert!(
            output
                .units
                .windows(2)
                .all(|pair| pair[0].start_ms <= pair[1].start_ms)
        );
        eprintln!("transcript: {}", output.transcription.text);
        eprintln!("aligned units: {}", output.units.len());
    }

    fn assert_contract_shape(manifest: &ModelManifest, kind: ModelKind) {
        manifest.validate_publishable().unwrap();
        let route = &manifest.routes[0];
        let artifact = manifest.artifact(&route.artifact).unwrap();
        let root =
            std::env::temp_dir().join(format!("valle-qwen-contract-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for file in &artifact.files {
            std::fs::write(root.join(&file.path), b"fixture").unwrap();
        }
        validate_open_request(
            OpenRequest {
                manifest,
                route,
                artifact,
                artifact_root: &root,
                cpu_threads: 1,
            },
            kind,
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
