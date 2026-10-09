//! Model contracts for the published valle-tts reference voice cloning backend.

use std::path::Path;

use anyhow::{Result, ensure};
use valle_tts::{TtsEngine, models::qwen3::Qwen3};

use crate::models::spec::{Artifact, Backend, ModelManifest, Route};

pub use valle_tts::{AudioChunk, ReferenceVoice, SynthesisOptions, SynthesisSummary};

pub const DEFAULT_MODEL_ID: &str = "qwen3-tts-0.6b-base-q8";
pub const RELEASE_VERSION: &str = "1.1.0";
pub const ADAPTER: &str = "qwen3-tts-voice-cloning";
pub const SAMPLE_RATE_HZ: u32 = 24_000;
pub const MAX_REFERENCE_SECONDS: u32 = 15;

#[derive(Clone, Copy)]
pub struct OpenRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
}

pub struct QwenTtsSession {
    engine: TtsEngine,
    model: String,
}

impl QwenTtsSession {
    pub fn open(request: OpenRequest<'_>) -> Result<Self> {
        request.manifest.validate()?;
        let id = request.manifest.model.id.as_str();
        ensure!(
            matches!(id, DEFAULT_MODEL_ID | "qwen3-tts-0.6b-base-q4"),
            "unsupported Qwen TTS model {id:?}"
        );
        ensure!(
            request.manifest.model.task == "speech-synthesis",
            "Qwen TTS requires a speech-synthesis release"
        );
        ensure!(
            request.manifest.contract.adapter == ADAPTER && request.manifest.contract.version == 1,
            "unsupported Qwen TTS contract"
        );
        ensure!(
            request.route.backend == Backend::NativeCpu
                && request.route.artifact == request.artifact.id,
            "Qwen TTS requires its declared native-cpu route"
        );
        ensure!(
            request.artifact.format == "gguf",
            "Qwen TTS requires GGUF weights"
        );
        let codec = request.artifact.metadata["codec"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("TTS artifact must declare its codec file"))?;
        for path in [request.artifact.entrypoint.as_str(), codec] {
            ensure!(
                request.artifact.files.iter().any(|file| file.path == path),
                "TTS file {path:?} is not declared by the artifact"
            );
            ensure!(
                request.artifact_root.join(path).is_file(),
                "TTS file {path:?} is missing"
            );
        }
        #[cfg(target_arch = "x86_64")]
        ensure!(
            std::is_x86_feature_detected!("avx2")
                && std::is_x86_feature_detected!("fma")
                && std::is_x86_feature_detected!("f16c"),
            "Qwen TTS on x64 requires AVX2, FMA and F16C"
        );
        let mut engine = TtsEngine::new();
        engine.register(Qwen3::load(
            id,
            request.artifact_root.join(&request.artifact.entrypoint),
            request.artifact_root.join(codec),
        )?)?;
        Ok(Self {
            engine,
            model: id.to_owned(),
        })
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        reference: &ReferenceVoice,
        options: &SynthesisOptions,
        sink: &mut (dyn FnMut(AudioChunk) -> Result<()> + Send),
    ) -> Result<SynthesisSummary> {
        self.engine
            .synthesize(&self.model, text, reference, options, sink)
    }
}
