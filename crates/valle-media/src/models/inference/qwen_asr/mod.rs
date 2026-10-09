//! Typed native CPU adapters shared by Qwen3 ASR and Qwen3 ForcedAligner releases.
//!
//! This module owns model-specific session and inference semantics. It accepts already-decoded
//! 16 kHz mono PCM and never opens or writes complete media files; those file workflows belong to
//! Valle. Keeping transcription and alignment as separate session types also lets a caller drop
//! the ASR weights before loading the aligner weights.

use std::path::Path;
#[cfg(feature = "model-qwen-native")]
use valle_asr::{
    AsrModel, Audio, CancellationToken, TimestampMode, TranscribeOptions, models::qwen3::Qwen3,
};

#[cfg(any(feature = "model-qwen-native", test))]
use crate::models::spec::Backend;
use crate::models::spec::{Artifact, ModelManifest, Route};
#[cfg(any(feature = "model-qwen-native", test))]
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const ASR_MODEL_ID: &str = "qwen3-asr-0.6b";
pub const ALIGNER_MODEL_ID: &str = "qwen3-aligner-0.6b";
pub const ASR_RELEASE_VERSION: &str = "1.2.0";
pub const ALIGNER_RELEASE_VERSION: &str = "1.2.0";
pub const ASR_ADAPTER: &str = "qwen3-asr-transcription";
pub const ALIGNER_ADAPTER: &str = "qwen3-forced-alignment";
pub const CONTRACT_VERSION: u32 = 1;
pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const MAX_ALIGNER_AUDIO_SECONDS: u32 = 30;
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

/// One independently owned valle-asr backend and a per-session CPU thread pool.
#[cfg(feature = "model-qwen-native")]
pub struct QwenAsrSession {
    model: Qwen3,
    pool: rayon::ThreadPool,
    max_chunk_samples: usize,
    cancellation: CancellationToken,
}

#[cfg(feature = "model-qwen-native")]
impl QwenAsrSession {
    pub fn open(request: OpenRequest<'_>) -> Result<Self> {
        validate_open_request(request, ModelKind::Asr)?;
        let options: AsrRouteOptions = serde_json::from_value(request.route.options.clone())
            .context("Qwen ASR route options are invalid")?;
        ensure!(
            options.segment_seconds.is_finite()
                && options.segment_seconds > 0.0
                && options.segment_seconds <= 30.0,
            "Qwen ASR segment_seconds must be within (0, 30]"
        );
        Ok(Self {
            model: Qwen3::load(ASR_MODEL_ID, request.artifact_root, None)?,
            pool: cpu_pool(request.cpu_threads)?,
            max_chunk_samples: seconds_to_samples(options.segment_seconds)?,
            cancellation: CancellationToken::default(),
        })
    }

    pub fn set_cancellation(&mut self, cancellation: CancellationToken) {
        self.cancellation = cancellation;
    }

    pub const fn max_chunk_samples(&self) -> usize {
        self.max_chunk_samples
    }

    pub fn transcribe_chunk(
        &mut self,
        chunk: PcmChunk<'_>,
        language: Option<&str>,
    ) -> Result<ChunkTranscription> {
        self.cancellation.check()?;
        validate_chunk(chunk, self.max_chunk_samples)?;
        let forced_language = language
            .map(|value| {
                canonical_language(value)
                    .with_context(|| format!("unsupported ASR language {value:?}"))
            })
            .transpose()?;
        if chunk.samples.is_empty() {
            return Ok(empty_chunk_transcription(chunk, forced_language));
        }
        let transcription = self.transcribe(chunk.samples, language)?;
        Ok(offset_chunk_transcription(chunk, transcription))
    }

    /// In-memory convenience; file tools supply bounded windows instead.
    pub fn transcribe(&mut self, samples: &[f32], language: Option<&str>) -> Result<Transcription> {
        validate_samples(samples)?;
        let options = TranscribeOptions {
            language: language
                .map(|value| {
                    canonical_language(value)
                        .map(str::to_owned)
                        .with_context(|| format!("unsupported ASR language {value:?}"))
                })
                .transpose()?,
            timestamps: TimestampMode::None,
            cancellation: self.cancellation.clone(),
            ..Default::default()
        };
        let audio = Audio::from_mono(samples.to_vec(), SAMPLE_RATE_HZ)?;
        let model = &mut self.model;
        let output = self.pool.install(|| model.transcribe(&audio, &options))?;
        Ok(Transcription {
            language: language_to_code(&output.language)
                .unwrap_or(&output.language)
                .to_owned(),
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
}

#[cfg(feature = "model-qwen-native")]
pub struct QwenAlignerSession {
    model: Qwen3,
    pool: rayon::ThreadPool,
    cancellation: CancellationToken,
}

#[cfg(feature = "model-qwen-native")]
impl QwenAlignerSession {
    /// The ASR directory supplies the tokenizer/configuration required by valle-asr's
    /// combined backend. Its neural weights remain unloaded during standalone alignment.
    pub fn open(asr: OpenRequest<'_>, aligner: OpenRequest<'_>) -> Result<Self> {
        validate_open_request(asr, ModelKind::Asr)?;
        validate_open_request(aligner, ModelKind::Aligner)?;
        Ok(Self {
            model: Qwen3::load(
                ASR_MODEL_ID,
                asr.artifact_root,
                Some(aligner.artifact_root.to_owned()),
            )?,
            pool: cpu_pool(aligner.cpu_threads)?,
            cancellation: CancellationToken::default(),
        })
    }

    pub fn set_cancellation(&mut self, cancellation: CancellationToken) {
        self.cancellation = cancellation;
    }

    pub const fn max_chunk_samples(&self) -> usize {
        MAX_ALIGNER_AUDIO_SECONDS as usize * SAMPLE_RATE_HZ as usize
    }

    pub fn align_chunk(
        &mut self,
        chunk: PcmChunk<'_>,
        text: &str,
        language: &str,
    ) -> Result<Vec<AlignedUnit>> {
        self.cancellation.check()?;
        validate_chunk(chunk, self.max_chunk_samples())?;
        if chunk.samples.is_empty() {
            ensure!(
                text.trim().is_empty(),
                "cannot align text against an empty audio chunk"
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

    pub fn align(
        &mut self,
        samples: &[f32],
        text: &str,
        language: &str,
    ) -> Result<Vec<AlignedUnit>> {
        validate_chunk(PcmChunk::new(samples, 0), self.max_chunk_samples())?;
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
        let audio = Audio::from_mono(samples.to_vec(), SAMPLE_RATE_HZ)?;
        let model = &mut self.model;
        let cancellation = &self.cancellation;
        let aligned = self
            .pool
            .install(|| model.align_with_cancellation(&audio, text, language, cancellation))?;
        let mut last_start = 0_u64;
        let mut units = aligned
            .into_iter()
            .map(|unit| {
                let start_ms = unit.start_ms.max(last_start);
                let end_ms = unit.end_ms.max(start_ms);
                last_start = start_ms;
                AlignedUnit {
                    text: unit.text,
                    start_ms,
                    end_ms,
                }
            })
            .collect::<Vec<_>>();
        restore_aligned_text(text, &mut units)?;
        Ok(units)
    }
}

/// The aligner strips separators before assigning timestamps. Canonical Valle words carry
/// source text spans so concatenation preserves spaces and sentence-ending punctuation.
#[cfg(any(feature = "model-qwen-native", test))]
fn restore_aligned_text(text: &str, units: &mut [AlignedUnit]) -> Result<()> {
    if units.is_empty() {
        return Ok(());
    }
    let text = text.trim();
    let significant = |character: char| character.is_alphanumeric() || character == '\'';
    let mut source = text.char_indices();
    let mut ranges = Vec::with_capacity(units.len());
    for unit in units.iter() {
        let mut start = None;
        let mut end = 0;
        for expected in unit.text.chars() {
            let (index, actual) = source
                .by_ref()
                .find(|(_, character)| significant(*character))
                .context("aligned words extend past the source transcript")?;
            ensure!(
                actual == expected,
                "aligned word does not match the source transcript"
            );
            start.get_or_insert(index);
            end = index + actual.len_utf8();
        }
        ranges.push((start.context("aligned word is empty")?, end));
    }
    ensure!(
        !source.any(|(_, character)| significant(character)),
        "aligned words do not cover the source transcript"
    );
    let mut start = 0;
    for (index, unit) in units.iter_mut().enumerate() {
        let end = if let Some(next) = ranges.get(index + 1) {
            let previous_end = ranges[index].1;
            text[previous_end..next.0]
                .find(char::is_whitespace)
                .map_or(next.0, |offset| previous_end + offset)
        } else {
            text.len()
        };
        unit.text = text[start..end].to_owned();
        start = end;
    }
    Ok(())
}

#[cfg(feature = "model-qwen-native")]
fn cpu_pool(cpu_threads: usize) -> Result<rayon::ThreadPool> {
    validate_cpu_threads(cpu_threads)?;
    rayon::ThreadPoolBuilder::new()
        .num_threads(cpu_threads)
        .build()
        .context("create ASR session CPU thread pool")
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
    let mut session = QwenAlignerSession::open(asr, aligner)?;
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
        "tokenizer_config.json",
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
            preprocess.maximum_audio_seconds == Some(MAX_ALIGNER_AUDIO_SECONDS),
            "Qwen aligner contract must declare the 30-second input limit"
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
pub(crate) fn canonical_language(language: &str) -> Option<&'static str> {
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
pub(crate) fn language_to_code(language: &str) -> Option<&'static str> {
    LANGUAGE_TABLE
        .iter()
        .find(|(name, _)| *name == language)
        .map(|(_, code)| *code)
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
    "Portuguese",
    "Russian",
    "Spanish",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_preserves_source_spacing_punctuation_and_timestamps() {
        for (source, words, expected) in [
            (
                "Hello, world!",
                vec!["Hello", "world"],
                vec!["Hello,", " world!"],
            ),
            (
                "你好，世界。 Hello!",
                vec!["你", "好", "世", "界", "Hello"],
                vec!["你", "好，", "世", "界。", " Hello!"],
            ),
            (
                "«L'école» -- well-known!",
                vec!["L'école", "wellknown"],
                vec!["«L'école»", " -- well-known!"],
            ),
        ] {
            let mut units = words
                .iter()
                .enumerate()
                .map(|(index, word)| AlignedUnit {
                    text: (*word).into(),
                    start_ms: index as u64 * 100,
                    end_ms: index as u64 * 100 + 50,
                })
                .collect::<Vec<_>>();
            restore_aligned_text(source, &mut units).unwrap();
            assert_eq!(
                units
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                units
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<String>(),
                source
            );
            assert!(
                units
                    .iter()
                    .enumerate()
                    .all(|(index, word)| word.start_ms == index as u64 * 100
                        && word.end_ms == index as u64 * 100 + 50)
            );
        }
    }

    #[test]
    fn alignment_text_drift_is_rejected_instead_of_publishing_a_wrong_transcript() {
        for word in ["Hi", "Helloagain", ""] {
            let mut units = vec![AlignedUnit {
                text: word.into(),
                start_ms: 0,
                end_ms: 100,
            }];
            assert!(restore_aligned_text("Hello world.", &mut units).is_err());
        }
    }

    #[test]
    fn native_cpu_thread_budget_must_be_positive() {
        assert!(validate_cpu_threads(1).is_ok());
        assert!(validate_cpu_threads(0).is_err());
    }

    #[cfg(feature = "model-qwen-native")]
    #[test]
    fn native_sessions_have_independent_cpu_thread_budgets() {
        let first = cpu_pool(2).unwrap();
        let second = cpu_pool(1).unwrap();
        assert_eq!(first.install(rayon::current_num_threads), 2);
        assert_eq!(second.install(rayon::current_num_threads), 1);
        assert_eq!(first.install(rayon::current_num_threads), 2);
    }

    #[test]
    fn language_names_and_iso_codes_resolve_to_one_contract() {
        assert_eq!(canonical_language("zh"), Some("Chinese"));
        assert_eq!(canonical_language("English"), Some("English"));
        assert_eq!(canonical_language("YUE"), Some("Cantonese"));
        assert_eq!(canonical_language("unknown"), None);
        assert_eq!(canonical_aligner_language("ja"), None);
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
            "../../catalog/release-qwen3-asr-0.6b-1.2.0.json"
        ))
        .unwrap();
        let aligner: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-aligner-0.6b-1.2.0.json"
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
            "../../catalog/release-qwen3-asr-0.6b-1.2.0.json"
        ))
        .unwrap();
        let aligner: ModelManifest = serde_json::from_str(include_str!(
            "../../catalog/release-qwen3-aligner-0.6b-1.2.0.json"
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
