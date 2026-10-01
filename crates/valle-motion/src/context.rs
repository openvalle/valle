//! Exact source and host clocks used by every Motion evaluator.

use serde::{Deserialize, Serialize};
use valle_timeline::internal::SampleTime;
use valle_timeline::{FrameRate, RationalTime};

use crate::time::{frame_at_sample_floor, frame_rate_as_f64, sample_time_at_frame};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionHostContext {
    #[cfg_attr(feature = "ts", ts(type = "{ composition: string }"))]
    pub sample: SampleTime,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub duration: RationalTime,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub progress: RationalTime,
}

impl MotionHostContext {
    /// The frame address comes from quantizing both absolute host boundaries.
    pub fn new(
        sample: SampleTime,
        duration: RationalTime,
        frame: u32,
        frames: u32,
    ) -> Option<Self> {
        if !duration.is_positive() || frames == 0 || frame >= frames {
            return None;
        }
        let progress = if frames == 1 {
            RationalTime::new(1, 2).ok()?
        } else {
            RationalTime::new(i64::from(frame), frames - 1).ok()?
        };
        Some(Self {
            sample,
            duration,
            progress,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionContext {
    pub host: MotionHostContext,
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
    let duration = sample_time_at_frame(i64::from(duration_frames), fps)
        .ok()?
        .composition();
    Some(MotionContext {
        host: MotionHostContext::new(sample, duration, local_frame, duration_frames)?,
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

/// Standalone delivery and review use the same template map as an admitted Timeline instance.
pub fn motion_context_at_host(
    role: valle_timeline::MotionRole,
    source_duration: RationalTime,
    host: MotionHostContext,
    fps: FrameRate,
) -> Option<MotionContext> {
    role.validate(source_duration, Some(host.duration)).ok()?;
    let frames = duration_frames(source_duration, fps)?;
    let mapped = role
        .overlay_time(host.sample.composition(), host.duration, source_duration)
        .ok()?;
    let end = sample_time_at_frame(i64::from(frames), fps)
        .ok()?
        .composition();
    let boundary = if matches!(role, valle_timeline::MotionRole::Overlay { .. }) {
        end.min(source_duration)
    } else {
        end
    };
    let mut ctx = if mapped >= boundary {
        motion_context_at_frame(frames - 1, frames, fps)?
    } else {
        motion_context_at_sample(SampleTime::new(mapped.max(RationalTime::ZERO)), frames, fps)?
    };
    ctx.host = host;
    Some(ctx)
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

    #[test]
    fn host_progress_has_exact_endpoints_and_preserves_authored_time() {
        let duration = RationalTime::new(23, 5).unwrap();
        let sample = SampleTime::new(RationalTime::new(-1, 1000).unwrap());
        let first = MotionHostContext::new(sample, duration, 0, 110).unwrap();
        assert_eq!(first.sample, sample);
        assert_eq!(first.duration, duration);
        assert_eq!(first.progress, RationalTime::ZERO);
        assert_eq!(
            MotionHostContext::new(sample, duration, 109, 110)
                .unwrap()
                .progress,
            RationalTime::new(1, 1).unwrap()
        );
        assert_eq!(
            MotionHostContext::new(sample, duration, 0, 1)
                .unwrap()
                .progress,
            RationalTime::new(1, 2).unwrap()
        );
        assert!(MotionHostContext::new(sample, duration, 0, 0).is_none());
        assert!(MotionHostContext::new(sample, duration, 110, 110).is_none());
        assert!(MotionHostContext::new(sample, RationalTime::ZERO, 0, 1).is_none());
    }
}
