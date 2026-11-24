//! Speech to text with Qwen3-ASR, and each word's time with Qwen3-ForcedAligner, on the CPU.

use std::path::Path;

use valle_asr::models::qwen3::Qwen3;
use valle_asr::{AsrEngine, TimestampMode, TranscribeOptions};
pub use valle_asr::{SAMPLE_RATE, Segment, Transcript, Word};

use crate::models::{self, ModelSpec};

/// The models a transcript needs, by ID: the aligner too where it has `words`.
pub fn needs(words: bool) -> &'static [&'static str] {
    match words {
        true => &[models::ASR, models::ALIGNER],
        false => &[models::ASR],
    }
}

/// Those of the models a transcript needs that are not installed under `root`.
pub fn missing(root: &Path, words: bool) -> Vec<ModelSpec> {
    needs(words)
        .iter()
        .map(|id| models::find(id).expect("the catalog has the models media runs"))
        .filter(|spec| !models::installed(root, spec))
        .collect()
}

/// The transcript of `samples`, mono at `SAMPLE_RATE`, with each word's time where `words` is
/// set. `language` is an English name or an ISO code; without it, each part's is detected. The
/// models must be installed under `root`.
pub fn transcribe(
    root: &Path,
    mut samples: Vec<f32>,
    language: Option<String>,
    words: bool,
) -> Result<Transcript, String> {
    if let Some(spec) = missing(root, words).first() {
        return Err(format!("{} is not installed", spec.id));
    }
    let dir = |id: &str| models::dir(root, &models::find(id).expect("in the catalog"));
    let aligner = words.then(|| dir(models::ALIGNER));
    let mut engine = AsrEngine::new();
    let model = Qwen3::load(models::ASR, dir(models::ASR), aligner).map_err(message)?;
    engine.register(model).map_err(message)?;
    // A downmix can pass full scale, and the models take samples within it.
    for sample in &mut samples {
        *sample = match sample.is_finite() {
            true => sample.clamp(-1.0, 1.0),
            false => 0.0,
        };
    }
    let options = TranscribeOptions {
        language,
        timestamps: match words {
            true => TimestampMode::Word,
            false => TimestampMode::None,
        },
        ..TranscribeOptions::default()
    };
    engine
        .transcribe(models::ASR, &samples, &options)
        .map_err(message)
}

/// An error with each of its causes.
fn message(e: valle_asr::Error) -> String {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::Folder;

    #[test]
    fn missing_models_are_named_before_anything_loads() {
        let root = Folder::new("transcribe-missing");
        let ids = |words| {
            missing(&root.0, words)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(true), [models::ASR, models::ALIGNER]);
        assert_eq!(ids(false), [models::ASR]);
        let e = transcribe(&root.0, vec![0.0; 16], None, false).unwrap_err();
        assert_eq!(e, format!("{} is not installed", models::ASR));
    }

    /// With the models installed under `$VALLE_MEDIA_TEST_MODELS`: a second of silence, and of
    /// full-scale noise past it, transcribe without an error.
    #[test]
    fn installed_models_take_samples_past_full_scale() {
        let Some(root) = std::env::var_os("VALLE_MEDIA_TEST_MODELS") else {
            return;
        };
        let mut samples = vec![0.0; SAMPLE_RATE as usize];
        samples.extend((0..SAMPLE_RATE).map(|i| if i % 2 == 0 { 1.4 } else { -1.4 }));
        samples.push(f32::NAN);
        let transcript = transcribe(Path::new(&root), samples, Some("en".into()), true).unwrap();
        assert_eq!(transcript.model, models::ASR);
        assert_eq!(transcript.duration_ms, 2001);
    }
}
