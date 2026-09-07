//! Closed component time context with always-present enter, hold, and exit subcontexts. Clamp phase
//! values outside their windows so completed entry and pending exit remain directly readable.
//! External inputs arrive through props and signals; components do not read a wall clock or global
//! time.

use serde::{Deserialize, Serialize};
use valle_timeline::FrameRate;
use valle_timeline::internal::SampleTime;

/// Exclusive current phase. Exactly one phase covers each valid local frame, consistent with the
/// phase active flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PhaseKind {
    Enter,
    Hold,
    Exit,
}

/// Entry or exit phase state. Before its window, frame/progress are zero; inside, progress is
/// frame/duration; after, they reach duration/one. Zero-length entry is complete, while zero-length
/// exit remains pending for every valid clip frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseContext {
    /// Whether the local frame lies inside this phase window.
    pub active: bool,
    /// Phase-local frame clamped to [0, duration_frames].
    pub frame: u32,
    /// Unclamped frames since phase start, negative before entry and increasing after completion.
    /// Springs use this field so they can continue settling beyond a phase window.
    pub elapsed_frames: i32,
    /// Allocated phase duration after proportional short-clip compression.
    pub duration_frames: u32,
    /// Normalized phase progress, clamped before and after the phase window.
    pub progress: f64,
}

/// Hold phase with cycle state. Without a positive cycle duration, the complete hold window forms
/// one cycle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HoldContext {
    pub active: bool,
    pub frame: u32,
    /// Unclamped frames since the hold phase began.
    pub elapsed_frames: i32,
    pub duration_frames: u32,
    pub progress: f64,
    /// Completed cycle index, remaining on the final played cycle after hold ends.
    pub iteration: u32,
    /// Frame within the current cycle; at hold completion it may equal the final cycle length.
    pub cycle_frame: u32,
    /// Normalized cycle progress, right-open while active and possibly 1 at completion.
    pub cycle_progress: f64,
}

/// Complete component-local time context.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionContext {
    /// Clip-local frame in the half-open duration interval. Discrete content may read it; Glass
    /// track-affecting bindings must not.
    pub local_frame: u32,
    /// Absolute clip-local composition sample. Cache identity for Glass tracks.
    #[cfg_attr(feature = "ts", ts(type = "{ composition: string }"))]
    pub sample: SampleTime,
    /// Continuous clip progress `local_time / duration` in `[0, 1)`. FPS-invariant.
    pub progress: f64,
    /// Clip duration in frames.
    pub duration_frames: u32,
    /// Exact rational frame rate using the canonical num/den string wire form.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fps: FrameRate,
    pub current_phase: PhaseKind,
    pub enter: PhaseContext,
    pub hold: HoldContext,
    pub exit: PhaseContext,
}
