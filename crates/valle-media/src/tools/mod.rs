//! File-level offline media workflows.
//!
//! Tool implementations own decode/infer/encode orchestration while model-specific tensor and
//! preprocessing semantics remain in `crate::models::adapters`.

#[cfg(any(feature = "tool-inpaint", feature = "tool-upscale", test))]
mod bounded_pipeline;
mod context;
mod error;
#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-matte",
    feature = "tool-segment",
    feature = "tool-separate",
    feature = "tool-shots",
    all(feature = "tool-transcribe", not(target_os = "windows")),
    feature = "tool-upscale"
))]
mod model_session;
mod output;
mod report;

#[cfg(any(
    all(feature = "tool-transcribe", not(target_os = "windows")),
    feature = "tool-separate"
))]
mod audio_workspace;

#[cfg(feature = "tool-enhance")]
pub mod enhance;
#[cfg(feature = "tool-inpaint")]
pub mod inpaint;
#[cfg(feature = "tool-interpolate")]
pub mod interpolate;
#[cfg(feature = "tool-matte")]
pub mod matte;
#[cfg(feature = "tool-segment")]
pub mod segment;
#[cfg(feature = "tool-separate")]
pub mod separate;
#[cfg(feature = "tool-shots")]
pub mod shots;
#[cfg(all(feature = "tool-transcribe", not(target_os = "windows")))]
pub mod transcribe;
#[cfg(feature = "tool-upscale")]
pub mod upscale;

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-matte",
    feature = "tool-segment",
    feature = "tool-shots",
    feature = "tool-upscale"
))]
mod video_sequence;

pub use context::{
    CancellationToken, NoopProgress, ProgressSink, ResourcePolicy, RunContext, ToolEvent, ToolPhase,
};
pub use error::{ToolError, ToolErrorCode};
#[cfg(any(
    feature = "tool-enhance",
    feature = "tool-separate",
    all(feature = "tool-transcribe", not(target_os = "windows"))
))]
pub(crate) use report::probe_audio_input;
pub use report::{
    AlphaMode, MediaRunEnvelope, MediaSummary, ModelProvenance, OutputArtifact, PipelineUsage,
    PreparedRunReport, ProcessedMedia, ProcessingReport, StageTiming, ToolRun, ToolWarning,
};

use serde::{Deserialize, Serialize};

/// A half-open source-local time range in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: f64,
    pub end: Option<f64>,
}

impl TimeRange {
    pub fn new(start: f64, end: Option<f64>) -> Result<Self, ToolError> {
        if !start.is_finite() || start < 0.0 {
            return Err(ToolError::invalid_input(
                "range start must be a finite, non-negative number",
            ));
        }
        if let Some(end) = end
            && (!end.is_finite() || end <= start)
        {
            return Err(ToolError::invalid_input(
                "range end must be finite and greater than range start",
            ));
        }
        Ok(Self { start, end })
    }
}
