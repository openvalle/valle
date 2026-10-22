//! The clocks every Motion evaluator reads at one sample of one host frame.

use serde::{Deserialize, Serialize};
use valle_timeline::internal::SampleTime;
use valle_timeline::{ClipClock, ExactRational, FrameRate, MotionRole, RationalTime};

/// One sample of a host frame, the whole frame or a subframe of Shutter, Echo or motion blur, with
/// every clock a scene reads there. The clock, the frame and the sample's output time decide the
/// other fields; build one with [`MotionContext::new`] and move it with [`MotionContext::at`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionContext {
    /// The clock of the host.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub clock: ClipClock,
    /// The output frame.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub frame: i64,
    /// The output time of this sample: the frame's own time, or a subframe's.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub at: RationalTime,
    /// The scene clock, `seconds`.
    #[cfg_attr(feature = "ts", ts(type = "{ composition: string }"))]
    pub sample: SampleTime,
    /// The scene time from the frame to this sample. It counts on across the seam of a declared
    /// loop, stops at a cut and stands still while the source holds.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub elapsed: RationalTime,
    /// `seconds / duration`.
    pub progress: f64,
    /// The clip clock, `clipSeconds`.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub clip_seconds: RationalTime,
    /// `clipProgress`: 0 on the host's first frame, 1 on its last.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub clip_progress: RationalTime,
    /// The source frame that names the scene time, for caches, seeds and diagnostics.
    pub local_frame: u32,
    /// How many frames the source has at the output rate.
    pub duration_frames: u32,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub fps: FrameRate,
}

impl MotionContext {
    /// Output frame `frame` of a host.
    pub fn new(clock: ClipClock, frame: i64) -> Option<Self> {
        Self::sample(clock, frame, clock.frame_time(frame).ok()?)
    }

    /// Frame `frame` of a standalone render of a source: one-shot or looping, a scene or a
    /// template over a host of `host` seconds, by default the source.
    pub fn standalone(
        role: MotionRole,
        duration: RationalTime,
        looping: bool,
        host: Option<RationalTime>,
        fps: FrameRate,
        frame: i64,
    ) -> Option<Self> {
        Self::new(
            ClipClock::standalone(fps, role, duration, looping, host).ok()?,
            frame,
        )
    }

    /// The sample of this frame at output time `at`.
    pub fn at(&self, at: RationalTime) -> Option<Self> {
        Self::sample(self.clock, self.frame, at)
    }

    /// The sample of this frame `offset` output seconds after this one.
    pub fn offset(&self, offset: RationalTime) -> Option<Self> {
        self.at(self.at.checked_add(offset).ok()?)
    }

    /// The output time of this frame.
    pub fn frame_time(&self) -> Option<RationalTime> {
        self.clock.frame_time(self.frame).ok()
    }

    /// The output duration of one frame.
    pub fn frame_duration(&self) -> Option<RationalTime> {
        self.clock.frame_time(1).ok()
    }

    fn sample(clock: ClipClock, frame: i64, at: RationalTime) -> Option<Self> {
        let sample = clock.sample(frame, at).ok()?;
        let progress = sample.seconds.as_f64() / clock.source().as_f64();
        Some(Self {
            clock,
            frame,
            at,
            sample: SampleTime::new(sample.seconds),
            elapsed: sample.elapsed,
            progress: progress.clamp(0.0, 1.0),
            clip_seconds: sample.clip_seconds,
            clip_progress: sample.clip_progress,
            local_frame: u32::try_from(clock.source_frame(sample.seconds).ok()?).ok()?,
            duration_frames: u32::try_from(clock.source_frames().ok()?).ok()?,
            fps: clock.fps(),
        })
    }
}

/// Frame `frame` of a standalone one-shot scene that lasts exactly `frames` frames.
pub fn motion_context_at_frame(frame: u32, frames: u32, fps: FrameRate) -> Option<MotionContext> {
    if frame >= frames {
        return None;
    }
    let duration = RationalTime::from_exact(
        ExactRational::new(i64::from(frames), 1)
            .ok()?
            .checked_div(fps.into_exact())
            .ok()?,
    );
    MotionContext::standalone(
        MotionRole::Clip,
        duration,
        false,
        None,
        fps,
        i64::from(frame),
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

    fn t(numerator: i64, denominator: u32) -> RationalTime {
        RationalTime::new(numerator, denominator).unwrap()
    }

    #[test]
    fn a_subframe_keeps_its_frame_and_reads_the_clock_at_its_own_time() {
        let fps = FrameRate::new(24, 1).unwrap();
        let ctx =
            MotionContext::standalone(MotionRole::Clip, t(23, 5), false, None, fps, 0).unwrap();
        assert_eq!(ctx.duration_frames, 110);
        let sample = ctx.offset(t(1, 48)).unwrap();
        assert_eq!(sample.frame, 0);
        assert_eq!(sample.local_frame, 0);
        assert_eq!(sample.sample, SampleTime::new(t(1, 48)));
        assert_eq!(sample.elapsed, t(1, 48));
        assert!((sample.progress - (1.0 / 48.0) / 4.6).abs() < 1e-12);

        // The last frame shows the end: progress 1 and the last source frame.
        let last =
            MotionContext::standalone(MotionRole::Clip, t(23, 5), false, None, fps, 109).unwrap();
        assert_eq!(last.sample, SampleTime::new(t(23, 5)));
        assert_eq!(last.progress, 1.0);
        assert_eq!(last.local_frame, 109);
        assert_eq!(last.clip_progress, RationalTime::ONE);
        assert!(motion_context_at_frame(110, 110, fps).is_none());
    }

    #[test]
    fn the_clip_clock_keeps_the_authored_start() {
        let fps = FrameRate::new(30, 1).unwrap();
        let clock = ClipClock::new(
            fps,
            t(1, 100),
            t(2, 1),
            valle_timeline::ClipMap::Clip {
                trim: RationalTime::ZERO,
                rate: valle_timeline::RationalRate::ONE,
                end: valle_timeline::SourceEnd::Hold,
            },
            t(3, 1),
            false,
        )
        .unwrap();
        let first = MotionContext::new(clock, 0).unwrap();
        assert_eq!(first.clip_seconds, t(-1, 100));
        assert_eq!(first.clip_progress, RationalTime::ZERO);
        assert_eq!(first.sample, SampleTime::ZERO);
        let last = MotionContext::new(clock, 59).unwrap();
        assert_eq!(last.clip_progress, RationalTime::ONE);
        assert_eq!(last.clock.duration(), t(2, 1));
    }
}
