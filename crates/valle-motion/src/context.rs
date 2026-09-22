//! The source clock used by every Motion evaluator. Values come from the requested sample.

use serde::{Deserialize, Serialize};
use valle_timeline::internal::SampleTime;
use valle_timeline::{FrameRate, RationalTime};

use crate::time::{frame_at_sample_floor, frame_rate_as_f64, sample_time_at_frame};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionContext {
    pub local_frame: u32,
    #[cfg_attr(feature = "ts", ts(type = "{ composition: string }"))]
    pub sample: SampleTime,
    pub progress: f64,
    pub duration_frames: u32,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fps: FrameRate,
}

/// Construct the base context for a frame in [0, duration_frames).
pub fn motion_context_at_frame(
    local_frame: u32,
    duration_frames: u32,
    fps: FrameRate,
) -> Option<MotionContext> {
    let sample = sample_time_at_frame(i64::from(local_frame), fps).ok()?;
    motion_context_at_source(local_frame, sample, duration_frames, fps)
}

/// Keep the mapped source's exact subframe time with its admitted discrete frame address.
pub fn motion_context_at_source(
    local_frame: u32,
    sample: SampleTime,
    duration_frames: u32,
    fps: FrameRate,
) -> Option<MotionContext> {
    if duration_frames == 0 || local_frame >= duration_frames || sample < SampleTime::ZERO {
        return None;
    }
    let seconds = sample.composition().as_f64();
    let progress = (seconds * frame_rate_as_f64(fps) / f64::from(duration_frames)).clamp(0.0, 1.0);
    Some(MotionContext {
        local_frame,
        sample,
        progress,
        duration_frames,
        fps,
    })
}

/// Sample the same work at arbitrary exact time. The right endpoint is exclusive.
pub fn motion_context_at_sample(
    sample: SampleTime,
    duration_frames: u32,
    fps: FrameRate,
) -> Option<MotionContext> {
    let end = sample_time_at_frame(i64::from(duration_frames), fps).ok()?;
    if sample < SampleTime::ZERO || sample >= end {
        return None;
    }
    let frame = u32::try_from(frame_at_sample_floor(sample, fps).ok()?).ok()?;
    motion_context_at_source(
        frame.min(duration_frames.saturating_sub(1)),
        sample,
        duration_frames,
        fps,
    )
}

/// Quantize authored duration at the actual output rate. Zero-frame works are invalid.
pub fn duration_frames(duration: RationalTime, fps: FrameRate) -> Option<u32> {
    let frames = valle_timeline::internal::quantize::quantize_frame_boundary(duration, fps).ok()?;
    u32::try_from(frames).ok().filter(|frames| *frames > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_subframe_time_and_quantized_progress_share_one_clock() {
        let fps = FrameRate::new(24, 1).unwrap();
        let frames = duration_frames(RationalTime::new(23, 5).unwrap(), fps).unwrap();
        assert_eq!(frames, 110);
        let sample = SampleTime::new(RationalTime::new(1, 48).unwrap());
        let ctx = motion_context_at_sample(sample, frames, fps).unwrap();
        assert_eq!(ctx.local_frame, 0);
        assert_eq!(ctx.sample, sample);
        assert!((ctx.progress - 0.5 / 110.0).abs() < 1e-12);
        assert!(motion_context_at_frame(frames, frames, fps).is_none());
    }
}
