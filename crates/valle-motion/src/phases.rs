//! Three-phase timing: integer window allocation and per-frame [`MotionContext`] evaluation. If
//! duration D covers declared enter E and exit X, hold receives D-E-X. Otherwise enter is
//! round(E*D/(E+X)) and exit takes the remainder, with no hold. Integer half-up rounding preserves
//! exclusive coverage of [0,D). Progress is frame/phase_length, with endpoints supplied by
//! out-of-window clamping, so frame rate changes sampling density rather than the animation curve.
//! Empty clips are not evaluated.

use serde::{Deserialize, Serialize};
use valle_timeline::FrameRate;
use valle_timeline::internal::SampleTime;

use crate::context::{HoldContext, MotionContext, PhaseContext, PhaseKind};
use crate::time::{frame_at_sample_floor, sample_time_at_frame};

/// Declared phase lengths in frames. Clips may override component defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseSpec {
    /// Requested enter length before compression.
    #[serde(default)]
    pub enter_frames: u32,
    /// Requested exit length.
    #[serde(default)]
    pub exit_frames: u32,
    /// Hold cycle length in frames. `None` or zero treats the entire hold as one cycle.
    #[serde(default)]
    pub hold_cycle_frames: Option<u32>,
}
/// Resolved phase lengths, with `enter + hold + exit == duration_frames`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseLayout {
    pub duration_frames: u32,
    pub enter_frames: u32,
    pub hold_frames: u32,
    pub exit_frames: u32,
    /// Cycle configuration from [`PhaseSpec`]; it affects hold context, not window allocation.
    pub hold_cycle_frames: Option<u32>,
}

/// Deterministic editor-side result for making a Motion clip follow narration length.
/// The latest narration cue ends before the fixed exit window; all length delta is absorbed by
/// hold, so authored enter/exit frame counts keep their exact feel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NarrationRetime {
    pub last_cue_end_frame: u32,
    pub duration_frames: u32,
    pub phases: PhaseLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetimeError {
    EmptyNarration,
    DurationOverflow,
}

impl core::fmt::Display for RetimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RetimeError::EmptyNarration => write!(f, "narration has no non-empty cue window"),
            RetimeError::DurationOverflow => write!(f, "retimed duration exceeds u32 frames"),
        }
    }
}

impl std::error::Error for RetimeError {}

pub fn retime_to_narration(
    last_cue_end_frame: u32,
    spec: &PhaseSpec,
) -> Result<NarrationRetime, RetimeError> {
    if last_cue_end_frame == 0 {
        return Err(RetimeError::EmptyNarration);
    }
    let body_end = last_cue_end_frame.max(spec.enter_frames);
    let duration_frames = body_end
        .checked_add(spec.exit_frames)
        .ok_or(RetimeError::DurationOverflow)?;
    let phases = phase_windows(spec, duration_frames);
    debug_assert_eq!(phases.enter_frames, spec.enter_frames);
    debug_assert_eq!(phases.exit_frames, spec.exit_frames);
    Ok(NarrationRetime {
        last_cue_end_frame,
        duration_frames,
        phases,
    })
}

impl PhaseLayout {
    /// Hold start, equal to the enter length.
    #[inline]
    pub fn hold_start(&self) -> u32 {
        self.enter_frames
    }

    /// Exit start, equal to enter plus hold.
    #[inline]
    pub fn exit_start(&self) -> u32 {
        self.enter_frames + self.hold_frames
    }
}

/// Round an unsigned ratio with half-up ties using u128 intermediates. Panics when `den` is zero;
/// the compression branch guarantees a positive denominator.
#[inline]
pub fn round_div(num: u128, den: u128) -> u128 {
    assert!(den != 0, "round_div: den must be non-zero");
    (2 * num + den) / (2 * den)
}

/// Allocate phase windows; zero duration produces three empty windows.
pub fn phase_windows(spec: &PhaseSpec, duration_frames: u32) -> PhaseLayout {
    let d = duration_frames;
    let (e, x) = (spec.enter_frames, spec.exit_frames);

    let (enter, hold, exit) = if d == 0 {
        // Empty clips have no phases and are not evaluated.
        (0, 0, 0)
    } else if u64::from(d) >= u64::from(e) + u64::from(x) {
        // Keep declared lengths and give the remaining frames to hold.
        (e, d - e - x, x)
    } else {
        // Compress proportionally, rounding enter first and assigning the remainder to exit. This
        // preserves coverage without gaps or unsigned underflow.
        let den = u128::from(e) + u128::from(x);
        let enter = round_div(u128::from(e) * u128::from(d), den) as u32;
        debug_assert!(enter <= d, "compressed enter must not exceed clip duration");
        (enter, 0, d - enter)
    };

    debug_assert_eq!(enter + hold + exit, d, "phases must exactly cover [0, D)");
    PhaseLayout {
        duration_frames: d,
        enter_frames: enter,
        hold_frames: hold,
        exit_frames: exit,
        hold_cycle_frames: spec.hold_cycle_frames,
    }
}

/// Return activity, frame, and progress for a phase window. Outside the window, clamp frame to zero
/// or length. A zero-length phase is complete at its start, so empty enter is complete and empty
/// exit remains pending for every valid clip frame.
#[inline]
/// Unclamped frames since the phase start. Subtract through i64 to preserve negative values before
/// narrowing to i32.
fn elapsed(local_frame: u32, start: u32) -> i32 {
    (i64::from(local_frame) - i64::from(start)) as i32
}

fn phase_at(local_frame: u32, start: u32, len: u32) -> (bool, u32, f64) {
    if local_frame < start {
        (false, 0, 0.0)
    } else if local_frame >= start + len {
        (false, len, 1.0)
    } else {
        let frame = local_frame - start;
        (true, frame, f64::from(frame) / f64::from(len))
    }
}

/// Evaluate a clip-local frame. Return `None` outside `[0, duration_frames)`, including empty
/// clips.
pub fn motion_context_at(
    local_frame: u32,
    layout: &PhaseLayout,
    fps: FrameRate,
) -> Option<MotionContext> {
    let d = layout.duration_frames;
    if d == 0 || local_frame >= d {
        return None;
    }

    let hold_start = layout.hold_start();
    let exit_start = layout.exit_start();

    let (enter_active, enter_frame, enter_progress) = phase_at(local_frame, 0, layout.enter_frames);
    let (hold_active, hold_frame, hold_progress) =
        phase_at(local_frame, hold_start, layout.hold_frames);
    let (exit_active, exit_frame, exit_progress) =
        phase_at(local_frame, exit_start, layout.exit_frames);

    // Exclusive phase windows automatically skip empty phases.
    let current_phase = if local_frame >= exit_start {
        PhaseKind::Exit
    } else if local_frame >= hold_start {
        PhaseKind::Hold
    } else {
        PhaseKind::Enter
    };

    // After hold ends, report the last played cycle at its closed endpoint. Computing directly from
    // the clamped hold frame could jump to an unplayed cycle when the length is an exact multiple
    // of the cycle period.
    let (iteration, cycle_frame, cycle_progress) = match layout.hold_cycle_frames {
        Some(cycle) if cycle > 0 => {
            let len = layout.hold_frames;
            if hold_frame == len && len > 0 {
                let it = (len - 1) / cycle;
                let cf = len - it * cycle;
                (it, cf, f64::from(cf) / f64::from(cycle))
            } else {
                (
                    hold_frame / cycle,
                    hold_frame % cycle,
                    f64::from(hold_frame % cycle) / f64::from(cycle),
                )
            }
        }
        // Without a cycle period, the entire hold is one cycle.
        _ => (0, hold_frame, hold_progress),
    };

    Some(MotionContext {
        local_frame,
        sample: sample_time_at_frame(i64::from(local_frame), fps).ok()?,
        progress: f64::from(local_frame) / f64::from(d),
        duration_frames: d,
        fps,
        current_phase,
        enter: PhaseContext {
            active: enter_active,
            frame: enter_frame,
            elapsed_frames: elapsed(local_frame, 0),
            duration_frames: layout.enter_frames,
            progress: enter_progress,
        },
        hold: HoldContext {
            active: hold_active,
            frame: hold_frame,
            elapsed_frames: elapsed(local_frame, hold_start),
            duration_frames: layout.hold_frames,
            progress: hold_progress,
            iteration,
            cycle_frame,
            cycle_progress,
        },
        exit: PhaseContext {
            active: exit_active,
            frame: exit_frame,
            elapsed_frames: elapsed(local_frame, exit_start),
            duration_frames: layout.exit_frames,
            progress: exit_progress,
        },
    })
}

/// Subframe evaluation: discrete `local_frame` is the covering output frame, while
/// `sample` / `progress` keep the exact rational time. Glass tracks read the latter.
pub fn motion_context_at_sample(
    local: SampleTime,
    layout: &PhaseLayout,
    fps: FrameRate,
) -> Option<MotionContext> {
    let duration = sample_time_at_frame(i64::from(layout.duration_frames), fps).ok()?;
    if local < SampleTime::ZERO || local >= duration {
        return None;
    }
    let seconds = local.composition().as_f64();
    let frame = u32::try_from(frame_at_sample_floor(local, fps).ok()?).ok()?;
    let frame = frame.min(layout.duration_frames.saturating_sub(1));
    let mut context = motion_context_at(frame, layout, fps)?;
    context.sample = local;
    let duration_seconds = duration.composition().as_f64();
    context.progress = if duration_seconds > 0.0 {
        seconds / duration_seconds
    } else {
        0.0
    };
    Some(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_div_ties_go_up_not_bankers() {
        assert_eq!(round_div(1, 2), 1);
        assert_eq!(round_div(3, 2), 2);
        assert_eq!(round_div(5, 2), 3);
        assert_eq!(round_div(4, 3), 1);
        assert_eq!(round_div(5, 3), 2);
        assert_eq!(round_div(0, 7), 0);
    }

    #[test]
    fn subframe_sample_keeps_canonical_progress_across_fps() {
        let spec = PhaseSpec {
            enter_frames: 0,
            exit_frames: 0,
            hold_cycle_frames: None,
        };
        let layout_30 = phase_windows(&spec, 30);
        let layout_60 = phase_windows(&spec, 60);
        let time = SampleTime::new(valle_timeline::RationalTime::new(1, 2).unwrap());
        let a = motion_context_at_sample(time, &layout_30, FrameRate::new(30, 1).unwrap()).unwrap();
        let b = motion_context_at_sample(time, &layout_60, FrameRate::new(60, 1).unwrap()).unwrap();
        assert_eq!(a.sample, b.sample);
        assert!((a.progress - 0.5).abs() < 1e-12);
        assert!((b.progress - 0.5).abs() < 1e-12);
    }

    #[test]
    fn round_div_survives_u64_overflow_range() {
        let big = u128::from(u32::MAX);
        assert_eq!(round_div(big * big, big), big);
    }

    #[test]
    fn narration_retime_changes_only_hold_and_keeps_enter_exit_exact() {
        let spec = PhaseSpec {
            enter_frames: 12,
            exit_frames: 9,
            hold_cycle_frames: None,
        };
        let short = retime_to_narration(30, &spec).unwrap();
        let long = retime_to_narration(90, &spec).unwrap();
        assert_eq!((short.duration_frames, short.phases.hold_frames), (39, 18));
        assert_eq!((long.duration_frames, long.phases.hold_frames), (99, 78));
        for result in [short, long] {
            assert_eq!(result.phases.enter_frames, 12);
            assert_eq!(result.phases.exit_frames, 9);
        }
        assert_eq!(
            retime_to_narration(0, &spec),
            Err(RetimeError::EmptyNarration)
        );
        assert_eq!(
            retime_to_narration(
                u32::MAX,
                &PhaseSpec {
                    exit_frames: 1,
                    ..PhaseSpec::default()
                }
            ),
            Err(RetimeError::DurationOverflow)
        );
    }
}
