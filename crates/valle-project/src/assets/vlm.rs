//! Shot-based visual analysis: combine shot windows, optional transcripts, and extracted
//! representative frames into an external batch request. Validate typed output and required synonym
//! arrays, retry once on failure, and publish only valid results.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::assets::analysis::{self, Analyzer, AnalyzerInput, AnalyzerOutput};
use crate::assets::home::Home;
use crate::assets::kind::AssetKind;
use crate::assets::report::{AssetsError, Result};
use crate::assets::transport;

/// Injected frame extraction. Return cache paths aligned with `times_ms`; individual unavailable
/// frames are `None`.
pub trait FrameExtractor {
    fn extract(
        &self,
        home: &Home,
        hash: &str,
        media_path: &Path,
        times_ms: &[i64],
    ) -> Result<Vec<Option<String>>>;
}

/// Extractor without frame support; analysis may still use transcript-only windows.
pub struct NoFrames;
impl FrameExtractor for NoFrames {
    fn extract(
        &self,
        _home: &Home,
        _hash: &str,
        _media_path: &Path,
        times_ms: &[i64],
    ) -> Result<Vec<Option<String>>> {
        Ok(vec![None; times_ms.len()])
    }
}

/// Visual analysis orchestration using the external analyzer protocol. The analysis command owns
/// the sidecar lifecycle.
pub struct VlmAnalyzer {
    pub argv: Vec<String>,
    pub timeout: Duration,
    /// Model configuration included in the cache key. Batch content is excluded; use a version
    /// change or force to refresh changed dependencies.
    pub params: Value,
    pub extractor: Box<dyn FrameExtractor>,
}

impl Analyzer for VlmAnalyzer {
    fn name(&self) -> &'static str {
        "vlm"
    }
    fn version(&self) -> u32 {
        1
    }
    fn accepts(&self, kind: AssetKind) -> bool {
        matches!(kind, AssetKind::Video | AssetKind::Image)
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["shots"]
    }
    fn default_params(&self) -> Value {
        self.params.clone()
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> Result<AnalyzerOutput> {
        let home = &input.home;
        // Video requires shot windows; an image uses a single whole-image window.
        let windows: Vec<(i64, i64)> = match input.kind {
            AssetKind::Image => vec![(0, 0)],
            _ => {
                let shots =
                    analysis::load_slot(home, input.hash, "shots", 1)?.ok_or_else(|| {
                        AssetsError::analyzer_failed("missing required shots analysis slot")
                            .with_hint("--with vlm schedules shots automatically; resolve shots failures first")
                    })?;
                shots
                    .items
                    .iter()
                    .filter_map(|it| {
                        Some((it.get("start_ms")?.as_i64()?, it.get("end_ms")?.as_i64()?))
                    })
                    .collect()
            }
        };
        // Optionally consume an existing ASR slot; new transcription is produced by `valle media
        // transcribe`.
        let asr = analysis::load_slot(home, input.hash, "asr", 1)?;

        // Use a midpoint frame for short shots, or start, middle, and end frames for shots longer
        // than two seconds.
        let mut batch = Vec::new();
        for (s, e) in &windows {
            let times: Vec<i64> = if e - s > 2000 {
                vec![*s, (s + e) / 2, (e - 1).max(*s)]
            } else {
                vec![(s + e) / 2]
            };
            let frames = self
                .extractor
                .extract(home, input.hash, input.path, &times)?;
            let transcript = asr
                .as_ref()
                .map(|slot| window_transcript(&slot.items, *s, *e))
                .unwrap_or_default();
            batch.push(json!({
                "start_ms": s,
                "end_ms": e,
                "frames": frames.into_iter().flatten().collect::<Vec<_>>(),
                "transcript": transcript,
            }));
        }

        let request = json!({
            "analyzer": "vlm",
            "version": 1,
            "hash": input.hash,
            "path": input.path.to_string_lossy(),
            "kind": input.kind.as_str(),
            "params": input.params,
            "batch": batch,
        });

        // Call and validate the batch, retrying once on failure.
        let mut last_err = None;
        for attempt in 0..2 {
            match transport::run_external(&self.argv, &request, self.timeout) {
                Ok(out) => match validate_items(&out.items, &windows) {
                    Ok(()) => return Ok(out),
                    Err(e) => {
                        last_err = Some(e.with_hint(format!(
                            "output validation failed on attempt {}",
                            attempt + 1
                        )));
                    }
                },
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("both attempts failed"))
    }
}

/// Collect ASR text overlapping a time window.
pub fn window_transcript(asr_items: &[Value], start_ms: i64, end_ms: i64) -> String {
    // Reassemble word-level ASR tokens. Insert spaces only between two multi-character tokens,
    // preserving contiguous CJK characters, punctuation, and individually aligned Latin letters.
    let mut prev_len = 0usize;
    let mut out = String::new();
    for it in asr_items {
        let (Some(s), Some(e)) = (
            it.get("start_ms").and_then(|v| v.as_i64()),
            it.get("end_ms").and_then(|v| v.as_i64()),
        ) else {
            continue;
        };
        // Include any overlapping token so cuts within a sentence retain the audible text on both
        // sides.
        if s < end_ms && e > start_ms {
            if let Some(t) = it.get("text").and_then(|v| v.as_str()) {
                let cur_len = t.chars().count();
                if prev_len >= 2 && cur_len >= 2 {
                    out.push(' ');
                }
                out.push_str(t);
                prev_len = cur_len;
            }
        }
    }
    out
}

/// Validate item counts, window bounds, summary strings, and synonym arrays. Reject the whole batch
/// before publishing invalid data.
fn validate_items(items: &[Value], windows: &[(i64, i64)]) -> Result<()> {
    if items.len() != windows.len() {
        return Err(AssetsError::analyzer_failed(format!(
            "item count ({}) does not match window count ({})",
            items.len(),
            windows.len()
        )));
    }
    for (i, it) in items.iter().enumerate() {
        let ok = it.get("start_ms").map(|v| v.is_i64()).unwrap_or(false)
            && it.get("end_ms").map(|v| v.is_i64()).unwrap_or(false)
            && it.get("summary").map(|v| v.is_string()).unwrap_or(false)
            && it
                .get("synonyms")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().all(|x| x.is_string()))
                .unwrap_or(false);
        if !ok {
            return Err(AssetsError::analyzer_failed(format!(
                "item[{i}] violates the schema: expected integer start_ms/end_ms, string summary, and string-array synonyms"
            )));
        }
    }
    Ok(())
}
