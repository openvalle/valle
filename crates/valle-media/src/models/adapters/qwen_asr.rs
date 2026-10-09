//! Product bridge for Valle's Qwen3 ASR and forced-aligner inference modules.
//!
//! Local inference owns model identity, tensor/file contracts, language configuration and
//! native inference. Media tools own complete-file decoding, bounded audio spooling, presentation and
//! reporting. The two session types intentionally remain separate so a transcription job can
//! release the ASR weights before loading the aligner weights.

use crate::models::inference::qwen_asr::{
    AlignedUnit, OpenRequest, PcmChunk, QwenAlignerSession as SharedAlignerSession,
    QwenAsrSession as SharedAsrSession, TranscriptSegment,
};
use anyhow::{Context, Result, anyhow};

use crate::models::manager::ResolvedModel;

pub(crate) const ASR_MODEL_ID: &str = crate::models::inference::qwen_asr::ASR_MODEL_ID;
pub(crate) const ALIGNER_MODEL_ID: &str = crate::models::inference::qwen_asr::ALIGNER_MODEL_ID;
pub(crate) const ASR_RELEASE_VERSION: &str =
    crate::models::inference::qwen_asr::ASR_RELEASE_VERSION;
pub(crate) const ALIGNER_RELEASE_VERSION: &str =
    crate::models::inference::qwen_asr::ALIGNER_RELEASE_VERSION;

pub(crate) type QwenSegment = TranscriptSegment;
pub(crate) type QwenAlignedUnit = AlignedUnit;

#[derive(Debug, Clone)]
pub(crate) struct QwenChunkTranscription {
    pub text: String,
    pub language: String,
    pub segments: Vec<QwenSegment>,
}

pub(crate) struct QwenAsrSession {
    inner: SharedAsrSession,
    language: Option<String>,
}

impl QwenAsrSession {
    pub(crate) fn open(
        model: &ResolvedModel,
        language: Option<&str>,
        cpu_threads: usize,
        cancellation: &crate::tools::CancellationToken,
    ) -> Result<Self> {
        let mut inner = SharedAsrSession::open(open_request(model, cpu_threads))
            .with_context(|| format!("open formal {} release", ASR_MODEL_ID))?;
        inner.set_cancellation(valle_asr::CancellationToken::from_shared_flag(
            cancellation.shared_flag(),
        ));
        Ok(Self {
            inner,
            language: language.map(str::to_owned),
        })
    }

    pub(crate) fn transcribe(
        &mut self,
        samples: &[f32],
        start_sample: u64,
    ) -> Result<QwenChunkTranscription> {
        let output = self.inner.transcribe_chunk(
            PcmChunk::new(samples, start_sample),
            self.language.as_deref(),
        )?;
        Ok(QwenChunkTranscription {
            text: output.text.trim().to_owned(),
            language: output.language,
            segments: output.segments,
        })
    }

    pub(crate) fn max_chunk_samples(&self) -> usize {
        self.inner.max_chunk_samples()
    }
}

pub(crate) struct QwenAlignerSession {
    inner: SharedAlignerSession,
}

impl QwenAlignerSession {
    pub(crate) fn open(
        asr: &ResolvedModel,
        model: &ResolvedModel,
        cpu_threads: usize,
        cancellation: &crate::tools::CancellationToken,
    ) -> Result<Self> {
        let mut inner = SharedAlignerSession::open(
            open_request(asr, cpu_threads),
            open_request(model, cpu_threads),
        )
        .with_context(|| format!("open formal {} release", ALIGNER_MODEL_ID))?;
        inner.set_cancellation(valle_asr::CancellationToken::from_shared_flag(
            cancellation.shared_flag(),
        ));
        Ok(Self { inner })
    }

    pub(crate) fn align(
        &mut self,
        samples: &[f32],
        text: &str,
        language: &str,
    ) -> Result<Vec<QwenAlignedUnit>> {
        self.inner.align(samples, text, language)
    }
}

pub(crate) fn resolve_alignment_language(
    requested: Option<&str>,
    transcript: &str,
) -> Result<(&'static str, &'static str)> {
    resolve_lang(requested, transcript)
}

fn open_request(model: &ResolvedModel, cpu_threads: usize) -> OpenRequest<'_> {
    OpenRequest {
        manifest: &model.manifest,
        route: &model.route,
        artifact: &model.artifact,
        artifact_root: &model.resolved.root,
        cpu_threads,
    }
}

fn resolve_lang(lang: Option<&str>, transcript: &str) -> Result<(&'static str, &'static str)> {
    use crate::models::inference::qwen_asr::{canonical_language, language_to_code};
    let language = match lang {
        Some(value) => {
            canonical_language(value).ok_or_else(|| anyhow!("unsupported language {value:?}"))?
        }
        None if transcript.chars().any(is_cjk) => "Chinese",
        None => "English",
    };
    Ok((
        language,
        language_to_code(language).expect("canonical languages have ISO codes"),
    ))
}

fn is_cjk(character: char) -> bool {
    matches!(u32::from(character),
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_resolution_is_stable() {
        assert_eq!(resolve_lang(Some("zh"), "").unwrap(), ("Chinese", "zh"));
        assert_eq!(
            resolve_lang(Some("English"), "").unwrap(),
            ("English", "en")
        );
        assert_eq!(resolve_lang(Some("fr"), "").unwrap(), ("French", "fr"));
        assert!(resolve_lang(Some("unknown"), "").is_err());
        assert_eq!(resolve_lang(None, "你好 world").unwrap().1, "zh");
        assert_eq!(resolve_lang(None, "hello world").unwrap().1, "en");
    }
}
