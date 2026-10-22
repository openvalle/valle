//! The clock of one host instance: a Timeline clip, a template on a Timeline, or a standalone
//! render. It maps output time to the scene clock, the source's own seconds, and gives the clip
//! clock, for whole frames and for the subframes of Shutter, Echo and motion blur alike.
//!
//! A piecewise linear clip map takes output time to source time: trim and rate, or the intro,
//! middle and outro of a template. A one-shot source then stays within `[0, duration]` and
//! reaches `duration` on its last displayed frame; a looping source wraps. Every sample of a
//! frame stays in the frame's segment and takes the segment's limit past a cut. Cuts are the
//! start and end of the host, the jumps of a template map and the wraps of a one-shot source in a
//! looping clip. A looping source in a looping clip has none: its samples wrap through the ends
//! of the host as through every other seam.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::internal::quantize::{quantize_frame_boundary, quantize_frame_interval};
use crate::{
    ExactRational, FrameRate, MotionRole, OverlayHold, RationalRate, RationalTime, TimeError,
};

/// What a Timeline clip does once its source has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceEnd {
    /// Show the end until the clip ends.
    Hold,
    /// Play the source again from its start.
    Loop,
    /// A frame past the end fails.
    Error,
}

/// How output time maps to source time, before the end of the source applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ClipMap {
    /// `trim + (τ − start) × rate`: a Timeline clip, or the standalone render of a scene.
    Clip {
        trim: RationalTime,
        rate: RationalRate,
        end: SourceEnd,
    },
    /// A template over its host: the intro and the outro play at speed, the outro ends with the
    /// host, and the middle between them plays once, loops or stretches.
    Template {
        intro: RationalTime,
        outro: RationalTime,
        hold: OverlayHold,
    },
}

impl ClipMap {
    /// The map of a template role; `None` for an ordinary clip.
    pub fn template(role: MotionRole) -> Option<Self> {
        role.template_timing()
            .map(|(intro, outro, hold)| Self::Template { intro, outro, hold })
    }
}

/// One sample of a host's clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// The scene clock, within `[0, duration]`.
    pub seconds: RationalTime,
    /// The scene time that elapses from the frame to the sample: it keeps counting across the
    /// wrap of a declared loop, stops at the segment's limit past a cut, and does not move while
    /// the source holds.
    pub elapsed: RationalTime,
    /// The clip clock `τ − start`, never clamped.
    pub clip_seconds: RationalTime,
    /// 0 on the first frame of the host and 1 on its last, linear between, clamped outside.
    pub clip_progress: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClockError {
    #[error(transparent)]
    Time(#[from] TimeError),
    #[error("a source needs a positive duration")]
    SourceDuration,
    #[error("a host needs a positive duration")]
    HostDuration,
    #[error(
        "template intro {intro} s and outro {outro} s must be non-negative and fit both the template and its host"
    )]
    TemplateTiming {
        intro: RationalTime,
        outro: RationalTime,
    },
}

type Result<T> = std::result::Result<T, ClockError>;

/// The whole time map of one host instance, built once per placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ClockWire", into = "ClockWire")]
pub struct ClipClock {
    wire: ClockWire,
    first_frame: i64,
    frames: u32,
    rule: EndRule,
    /// A template jumps where its outro starts.
    exit_cut: bool,
    end: End,
    unsampled_end: bool,
}

/// The inputs of a clock; everything else derives from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClockWire {
    fps: FrameRate,
    start: RationalTime,
    duration: RationalTime,
    map: ClipMap,
    source: RationalTime,
    looping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndRule {
    /// A one-shot source clamps, and steps to its end on the last displayed frame.
    Once,
    /// A looping source wraps. `cut` makes every play its own segment: a one-shot source in a
    /// looping clip.
    Wrap { cut: bool },
    /// A looping source in a hold clip stops on its last played frame.
    HoldLast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The host ends first, the source wraps, or no frame shows the source before its end.
    Open,
    /// The step of a one-shot source over `[from, at]` inside segment `segment`: from `value`
    /// to the end, reached on the last displayed frame `frame` at `at`. `slope` is scene seconds
    /// per output second and `unwrapped` the unwrapped clock at `from`.
    Step {
        frame: i64,
        segment: i64,
        from: RationalTime,
        at: RationalTime,
        value: RationalTime,
        slope: RationalTime,
        unwrapped: RationalTime,
    },
    /// A looping source in a hold clip stops at `value`, its time on frame `frame` at `at`.
    Hold {
        frame: i64,
        at: RationalTime,
        value: RationalTime,
    },
    /// A looping source in a hold clip that never plays: every sample shows its end.
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    index: i64,
    from: RationalTime,
    to: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The segment `[from, to)` a frame at this time belongs to.
    Right,
    /// The segment `(from, to]` whose end a limit from below reaches.
    Left,
}

impl ClipClock {
    /// A host on a Timeline: it starts at `start` and lasts `duration` on an output grid of `fps`,
    /// and its frames lie between the nearest frame boundaries of both ends.
    pub fn new(
        fps: FrameRate,
        start: RationalTime,
        duration: RationalTime,
        map: ClipMap,
        source: RationalTime,
        looping: bool,
    ) -> Result<Self> {
        Self::try_from(ClockWire {
            fps,
            start,
            duration,
            map,
            source,
            looping,
        })
    }

    /// A standalone render: the host starts at 0 and lasts `host`, by default the source. A scene
    /// plays once and holds its end, or loops; a template maps its timing onto the host. This is
    /// the clock of a one-clip Timeline that holds a one-shot scene or loops a looping one.
    pub fn standalone(
        fps: FrameRate,
        role: MotionRole,
        source: RationalTime,
        looping: bool,
        host: Option<RationalTime>,
    ) -> Result<Self> {
        let map = ClipMap::template(role).unwrap_or(ClipMap::Clip {
            trim: RationalTime::ZERO,
            rate: RationalRate::ONE,
            end: if looping {
                SourceEnd::Loop
            } else {
                SourceEnd::Hold
            },
        });
        Self::new(
            fps,
            RationalTime::ZERO,
            host.unwrap_or(source),
            map,
            source,
            looping,
        )
    }

    pub const fn fps(&self) -> FrameRate {
        self.wire.fps
    }

    /// Where the host starts on the output timeline.
    pub const fn start(&self) -> RationalTime {
        self.wire.start
    }

    /// The authored length of the host: `clipDuration`.
    pub const fn duration(&self) -> RationalTime {
        self.wire.duration
    }

    /// The length of the source: `duration`.
    pub const fn source(&self) -> RationalTime {
        self.wire.source
    }

    pub const fn map(&self) -> ClipMap {
        self.wire.map
    }

    pub const fn looping(&self) -> bool {
        self.wire.looping
    }

    /// The first output frame of the host.
    pub const fn first_frame(&self) -> i64 {
        self.first_frame
    }

    /// How many output frames the host covers.
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// The last frame that shows a one-shot source before it ends, which shows its end; for a
    /// looping source in a hold clip, the frame it stops on.
    pub const fn last_displayed_frame(&self) -> Option<i64> {
        match self.end {
            End::Step { frame, .. } | End::Hold { frame, .. } => Some(frame),
            End::Open | End::Ended => None,
        }
    }

    /// A one-shot source ends inside a segment of the host that no frame samples, such as a
    /// template outro shorter than a frame: no frame shows its end.
    pub const fn unsampled_end(&self) -> bool {
        self.unsampled_end
    }

    /// The output time of a frame.
    pub fn frame_time(&self, frame: i64) -> Result<RationalTime> {
        frame_time(frame, self.wire.fps)
    }

    /// How many frames the source has at the output rate, at least one.
    pub fn source_frames(&self) -> Result<i64> {
        Ok(quantize_frame_boundary(self.wire.source, self.wire.fps)?.max(1))
    }

    /// The source frame that identifies a scene time: `floor(seconds × fps)` within the source's
    /// frames. It names caches and diagnostics and never feeds the clock.
    pub fn source_frame(&self, seconds: RationalTime) -> Result<i64> {
        let frames = seconds
            .into_exact()
            .checked_mul(self.wire.fps.into_exact())?;
        let frame = floor(RationalTime::from_exact(frames));
        Ok(frame.clamp(0, self.source_frames()? - 1))
    }

    /// The fastest the scene clock runs against output time, apart from the end step: the rate
    /// of a clip, or the fastest part of a template.
    pub fn max_speed(&self) -> Result<RationalTime> {
        match self.wire.map {
            ClipMap::Clip { rate, .. } => Ok(RationalTime::from_exact(rate.into_exact())),
            ClipMap::Template { intro, outro, hold } => {
                let exit = self.wire.duration.checked_sub(outro)?;
                let length = self.middle(intro, outro)?;
                let mut speed = if intro.is_positive() || outro.is_positive() {
                    RationalTime::ONE
                } else {
                    RationalTime::ZERO
                };
                if exit > intro && length.is_positive() {
                    speed = speed.max(match hold {
                        OverlayHold::Stretch => length.checked_div(exit.checked_sub(intro)?)?,
                        OverlayHold::Once | OverlayHold::Loop => RationalTime::ONE,
                    });
                }
                Ok(speed)
            }
        }
    }

    /// Both clocks at output time `at` for output frame `frame`. A whole frame passes its own
    /// time; a subframe passes its sample time, and the frame decides which segment it reads.
    pub fn sample(&self, frame: i64, at: RationalTime) -> Result<ClockSample> {
        let time = self.frame_time(frame)?;
        let (seconds, unwrapped) = self.scene(time, at)?;
        let elapsed = if at == time {
            RationalTime::ZERO
        } else {
            unwrapped.checked_sub(self.scene(time, time)?.1)?
        };
        let first = self.frame_time(self.first_frame)?;
        let clip_progress = if self.frames <= 1 {
            RationalTime::new(1, 2)?
        } else {
            let last = self.frame_time(self.first_frame + i64::from(self.frames) - 1)?;
            at.checked_sub(first)?
                .checked_div(last.checked_sub(first)?)?
                .clamp(RationalTime::ZERO, RationalTime::ONE)
        };
        Ok(ClockSample {
            seconds,
            elapsed,
            clip_seconds: at.checked_sub(self.wire.start)?,
            clip_progress,
        })
    }

    fn host_end(&self) -> Result<RationalTime> {
        Ok(self.wire.start.checked_add(self.wire.duration)?)
    }

    /// Whether samples wrap through both ends of the host: a looping source in a looping clip.
    fn periodic(&self) -> bool {
        self.rule == EndRule::Wrap { cut: false } && matches!(self.wire.map, ClipMap::Clip { .. })
    }

    /// The scene clock and its unwrapped value at `at`, for the frame at output time `time`.
    fn scene(&self, time: RationalTime, at: RationalTime) -> Result<(RationalTime, RationalTime)> {
        let host_end = self.host_end()?;
        if self.periodic() {
            let mapped = self
                .map_at(Segment::whole(self.wire.start, host_end), at)?
                .0;
            return Ok((euclid_mod(mapped, self.wire.source)?, mapped));
        }
        let segment = self.segment(time.clamp(self.wire.start, host_end), Side::Right)?;
        self.value(segment, at.clamp(segment.from, segment.to))
    }

    /// The segment that holds output time `t` of the host, between the cuts around it.
    fn segment(&self, t: RationalTime, side: Side) -> Result<Segment> {
        let start = self.wire.start;
        let host_end = self.host_end()?;
        match (self.wire.map, self.rule) {
            (ClipMap::Clip { trim, rate, .. }, EndRule::Wrap { cut: true }) => {
                let source = self.wire.source;
                let mapped = self.map_at(Segment::whole(start, host_end), t)?.0;
                let ratio = mapped.checked_div(source)?;
                let play = match side {
                    Side::Right => floor(ratio),
                    Side::Left => ceil(ratio) - 1,
                };
                let wrap = |play: i64| -> Result<RationalTime> {
                    let at = RationalTime::new(play, 1)?
                        .checked_mul(source)?
                        .checked_sub(trim)?;
                    Ok(start.checked_add(divide(at, rate)?)?)
                };
                Ok(Segment {
                    index: play,
                    from: wrap(play)?.max(start),
                    to: wrap(play.checked_add(1).ok_or(TimeError::Overflow)?)?.min(host_end),
                })
            }
            (ClipMap::Template { outro, .. }, _) if self.exit_cut => {
                let exit = host_end.checked_sub(outro)?;
                Ok(if t < exit || side == Side::Left && t == exit {
                    Segment {
                        index: 0,
                        from: start,
                        to: exit,
                    }
                } else {
                    Segment {
                        index: 1,
                        from: exit,
                        to: host_end,
                    }
                })
            }
            _ => Ok(Segment::whole(start, host_end)),
        }
    }

    /// The scene clock and its unwrapped value at `tau` inside `segment`.
    fn value(&self, segment: Segment, tau: RationalTime) -> Result<(RationalTime, RationalTime)> {
        let (mapped, unwrapped) = self.map_at(segment, tau)?;
        let source = self.wire.source;
        match self.rule {
            EndRule::Wrap { cut: true } => {
                // One play of a one-shot source: the segment is the play.
                let offset = RationalTime::new(segment.index, 1)?.checked_mul(source)?;
                Ok((mapped.checked_sub(offset)?, unwrapped))
            }
            EndRule::Wrap { cut: false } => Ok((euclid_mod(mapped, source)?, unwrapped)),
            EndRule::HoldLast => Ok(match self.end {
                End::Ended => (source, source),
                End::Hold { at, value, .. } if tau >= at => (value, value),
                _ => (euclid_mod(mapped, source)?, unwrapped),
            }),
            EndRule::Once => {
                if let End::Step {
                    segment: index,
                    from,
                    at,
                    value,
                    slope,
                    unwrapped: base,
                    ..
                } = self.end
                    && segment.index == index
                    && tau >= from
                {
                    let seconds = if tau >= at {
                        source
                    } else {
                        value.checked_add(slope.checked_mul(tau.checked_sub(from)?)?)?
                    };
                    return Ok((seconds, base.checked_add(seconds.checked_sub(value)?)?));
                }
                let seconds = mapped.clamp(RationalTime::ZERO, source);
                Ok(match self.wire.map {
                    ClipMap::Clip { .. } => (seconds, seconds),
                    ClipMap::Template { .. } => (seconds, unwrapped),
                })
            }
        }
    }

    /// The clip map and its unwrapped value at `tau` inside `segment`. A template's loop middle
    /// unwraps to host time; every other map is its own unwrapped value.
    fn map_at(&self, segment: Segment, tau: RationalTime) -> Result<(RationalTime, RationalTime)> {
        let local = tau.checked_sub(self.wire.start)?;
        match self.wire.map {
            ClipMap::Clip { trim, rate, .. } => {
                let mapped = trim.checked_add(local.checked_scale(rate)?)?;
                Ok((mapped, mapped))
            }
            ClipMap::Template { hold, .. } => {
                let mapped = self.template_at(local, self.exit_cut && segment.index == 0)?;
                Ok((
                    mapped,
                    if hold == OverlayHold::Loop {
                        local
                    } else {
                        mapped
                    },
                ))
            }
        }
    }

    /// A template's source time at host time `u` within `[0, duration]`. `left` reads the
    /// middle's limit where the outro starts.
    fn template_at(&self, u: RationalTime, left: bool) -> Result<RationalTime> {
        let ClipMap::Template { intro, outro, hold } = self.wire.map else {
            unreachable!("a template map");
        };
        let source = self.wire.source;
        let exit = self.wire.duration.checked_sub(outro)?;
        if u >= exit && !left {
            return Ok(source
                .checked_sub(outro)?
                .checked_add(u.checked_sub(exit)?)?);
        }
        if u < intro || exit <= intro {
            return Ok(u.min(intro));
        }
        let length = self.middle(intro, outro)?;
        if length == RationalTime::ZERO {
            return Ok(intro);
        }
        let elapsed = u.checked_sub(intro)?;
        let offset = match hold {
            OverlayHold::Once => elapsed.min(length),
            OverlayHold::Loop => {
                let plays = floor(elapsed.checked_div(length)?);
                let offset =
                    elapsed.checked_sub(RationalTime::new(plays, 1)?.checked_mul(length)?)?;
                if left && offset == RationalTime::ZERO && plays > 0 {
                    // Whole plays fill the middle: its limit is the end of a play.
                    length
                } else {
                    offset
                }
            }
            OverlayHold::Stretch => elapsed
                .checked_mul(length)?
                .checked_div(exit.checked_sub(intro)?)?,
        };
        Ok(intro.checked_add(offset)?)
    }

    /// The length of a template's own middle.
    fn middle(&self, intro: RationalTime, outro: RationalTime) -> Result<RationalTime> {
        Ok(self.wire.source.checked_sub(intro)?.checked_sub(outro)?)
    }

    fn find_exit_cut(&self) -> Result<bool> {
        let ClipMap::Template { outro, .. } = self.wire.map else {
            return Ok(false);
        };
        let exit = self.wire.duration.checked_sub(outro)?;
        Ok(self.template_at(exit, true)? != self.wire.source.checked_sub(outro)?)
    }

    /// Where the source arrives at its end inside one segment of the host, if it does there.
    fn arrival(&self) -> Result<Option<RationalTime>> {
        let source = self.wire.source;
        let start = self.wire.start;
        let host_end = self.host_end()?;
        match self.wire.map {
            ClipMap::Clip { trim, rate, .. } => {
                if trim >= source {
                    return Ok(None);
                }
                let at = start.checked_add(divide(source.checked_sub(trim)?, rate)?)?;
                Ok((at <= host_end).then_some(at))
            }
            ClipMap::Template { intro, outro, hold } => {
                let host = self.wire.duration;
                let length = self.middle(intro, outro)?;
                let local = if outro.is_positive() {
                    Some(host)
                } else if length == RationalTime::ZERO {
                    Some(intro)
                } else {
                    match hold {
                        OverlayHold::Once => {
                            let at = intro.checked_add(length)?;
                            (at <= host).then_some(at)
                        }
                        OverlayHold::Stretch => Some(host),
                        OverlayHold::Loop => {
                            // The plays of a loop middle wrap without arriving; only a last play
                            // that ends with the host does.
                            let elapsed = host.checked_sub(intro)?;
                            let plays = floor(elapsed.checked_div(length)?);
                            (plays >= 1
                                && RationalTime::new(plays, 1)?.checked_mul(length)? == elapsed)
                                .then_some(host)
                        }
                    }
                };
                Ok(match local {
                    Some(local) => Some(start.checked_add(local)?),
                    None => None,
                })
            }
        }
    }

    /// The end of the source on this host, and whether no frame shows a one-shot end.
    fn find_end(&self) -> Result<(End, bool)> {
        if matches!(self.rule, EndRule::Wrap { .. }) {
            return Ok((End::Open, false));
        }
        let hold_last = self.rule == EndRule::HoldLast;
        if let ClipMap::Clip { trim, .. } = self.wire.map
            && trim >= self.wire.source
        {
            // The whole host lies past the end: a one-shot source clamps there.
            return Ok((if hold_last { End::Ended } else { End::Open }, false));
        }
        let Some(arrival) = self.arrival()? else {
            return Ok((End::Open, false));
        };
        let start = self.wire.start;
        let segment = self.segment(arrival, Side::Left)?;
        let last = self.first_frame + i64::from(self.frames) - 1;
        let before = last_frame_before(arrival, self.wire.fps)?;
        let frame = before.min(last);
        let at = self.frame_time(frame)?.max(start);
        if self.frames == 0 || frame < self.first_frame || at < segment.from {
            if hold_last {
                return Ok((End::Ended, false));
            }
            // Frames from the arrival on still show the end, unless the segment has none.
            let next = before
                .checked_add(1)
                .ok_or(TimeError::Overflow)?
                .max(self.first_frame);
            let shown = next <= last && self.frame_time(next)?.max(start) < segment.to;
            return Ok((End::Open, !shown));
        }
        if hold_last {
            let value = self.map_at(segment, at)?.0;
            return Ok((End::Hold { frame, at, value }, false));
        }
        let from = at.checked_sub(self.frame_time(1)?)?.max(segment.from);
        let (value, unwrapped) = self.value(segment, from)?;
        let slope = if at > from {
            self.wire
                .source
                .checked_sub(value)?
                .checked_div(at.checked_sub(from)?)?
        } else {
            RationalTime::ZERO
        };
        Ok((
            End::Step {
                frame,
                segment: segment.index,
                from,
                at,
                value,
                slope,
                unwrapped,
            },
            false,
        ))
    }
}

impl TryFrom<ClockWire> for ClipClock {
    type Error = ClockError;

    fn try_from(wire: ClockWire) -> Result<Self> {
        if !wire.source.is_positive() {
            return Err(ClockError::SourceDuration);
        }
        if !wire.duration.is_positive() {
            return Err(ClockError::HostDuration);
        }
        if let ClipMap::Template { intro, outro, .. } = wire.map {
            let fixed = intro.checked_add(outro)?;
            if intro.is_negative()
                || outro.is_negative()
                || fixed > wire.source
                || fixed > wire.duration
            {
                return Err(ClockError::TemplateTiming { intro, outro });
            }
        }
        let interval = quantize_frame_interval(wire.start, wire.duration, wire.fps)?;
        let frames =
            u32::try_from(interval.duration_frames.max(0)).map_err(|_| TimeError::Overflow)?;
        let rule = match wire.map {
            ClipMap::Clip {
                end: SourceEnd::Loop,
                ..
            } => EndRule::Wrap { cut: !wire.looping },
            ClipMap::Clip {
                end: SourceEnd::Hold,
                ..
            } if wire.looping => EndRule::HoldLast,
            ClipMap::Template { .. } if wire.looping => EndRule::Wrap { cut: false },
            ClipMap::Clip { .. } | ClipMap::Template { .. } => EndRule::Once,
        };
        let mut clock = Self {
            wire,
            first_frame: interval.start_frame,
            frames,
            rule,
            exit_cut: false,
            end: End::Open,
            unsampled_end: false,
        };
        clock.exit_cut = clock.find_exit_cut()?;
        (clock.end, clock.unsampled_end) = clock.find_end()?;
        Ok(clock)
    }
}

impl From<ClipClock> for ClockWire {
    fn from(clock: ClipClock) -> Self {
        clock.wire
    }
}

impl Segment {
    const fn whole(from: RationalTime, to: RationalTime) -> Self {
        Self { index: 0, from, to }
    }
}

fn frame_time(frame: i64, fps: FrameRate) -> Result<RationalTime> {
    Ok(RationalTime::from_exact(
        ExactRational::new(frame, 1)?.checked_div(fps.into_exact())?,
    ))
}

/// The last frame strictly before `time`.
fn last_frame_before(time: RationalTime, fps: FrameRate) -> Result<i64> {
    let frames = time.into_exact().checked_mul(fps.into_exact())?;
    Ok(ceil(RationalTime::from_exact(frames)) - 1)
}

fn divide(time: RationalTime, rate: RationalRate) -> Result<RationalTime> {
    Ok(RationalTime::from_exact(
        time.into_exact().checked_div(rate.into_exact())?,
    ))
}

fn floor(value: RationalTime) -> i64 {
    value.numerator().div_euclid(i64::from(value.denominator()))
}

fn ceil(value: RationalTime) -> i64 {
    let denominator = i64::from(value.denominator());
    value.numerator().div_euclid(denominator)
        + i64::from(value.numerator().rem_euclid(denominator) != 0)
}

/// `value mod period` in `[0, period)` for a positive period.
fn euclid_mod(value: RationalTime, period: RationalTime) -> Result<RationalTime> {
    let plays = floor(value.checked_div(period)?);
    Ok(value.checked_sub(RationalTime::new(plays, 1)?.checked_mul(period)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(numerator: i64, denominator: u32) -> RationalTime {
        RationalTime::new(numerator, denominator).unwrap()
    }

    fn fps(rate: i64) -> FrameRate {
        FrameRate::new(rate, 1).unwrap()
    }

    fn clip(trim: RationalTime, rate: RationalRate, end: SourceEnd) -> ClipMap {
        ClipMap::Clip { trim, rate, end }
    }

    fn hold() -> ClipMap {
        clip(RationalTime::ZERO, RationalRate::ONE, SourceEnd::Hold)
    }

    fn overlay(intro: RationalTime, outro: RationalTime, hold: OverlayHold) -> MotionRole {
        MotionRole::Overlay { intro, outro, hold }
    }

    fn seconds(clock: &ClipClock, frame: i64) -> RationalTime {
        let time = clock.frame_time(frame).unwrap();
        clock.sample(frame, time).unwrap().seconds
    }

    fn at(clock: &ClipClock, frame: i64, offset: RationalTime) -> ClockSample {
        let time = clock.frame_time(frame).unwrap();
        clock
            .sample(frame, time.checked_add(offset).unwrap())
            .unwrap()
    }

    #[test]
    fn a_standalone_scene_ends_on_its_last_frame() {
        let clock = ClipClock::standalone(fps(30), MotionRole::Clip, t(3, 1), false, None).unwrap();
        assert_eq!(clock.frames(), 90);
        assert_eq!(seconds(&clock, 0), RationalTime::ZERO);
        assert_eq!(seconds(&clock, 88), t(88, 30));
        assert_eq!(seconds(&clock, 89), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(89));
        assert!(!clock.unsampled_end());

        // 4.6 s at 24 fps has 110 frames; the last one shows the end, not 4.542.
        let clock =
            ClipClock::standalone(fps(24), MotionRole::Clip, t(23, 5), false, None).unwrap();
        assert_eq!(clock.frames(), 110);
        assert_eq!(seconds(&clock, 108), t(9, 2));
        assert_eq!(seconds(&clock, 109), t(23, 5));
        assert_eq!(clock.source_frame(t(23, 5)).unwrap(), 109);
        assert_eq!(clock.source_frame(t(9, 2)).unwrap(), 108);
        assert_eq!(clock.source_frame(t(-1, 10)).unwrap(), 0);
    }

    #[test]
    fn a_clip_reaches_the_end_on_the_last_frame_before_it_arrives() {
        // Rate 2 over 1.5 s.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(3, 2),
            clip(
                RationalTime::ZERO,
                RationalRate::new(2, 1).unwrap(),
                SourceEnd::Hold,
            ),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(clock.frames(), 45);
        assert_eq!(seconds(&clock, 43), t(43, 15));
        assert_eq!(seconds(&clock, 44), t(3, 1));

        // Rate 0.5 over 6 s: the frame before L keeps its time.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(6, 1),
            clip(
                RationalTime::ZERO,
                RationalRate::new(1, 2).unwrap(),
                SourceEnd::Hold,
            ),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 178), t(89, 30));
        assert_eq!(seconds(&clock, 179), t(3, 1));

        // Trim 0.02 over 3.5 s, hold: frame 89 would show 2.987.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(7, 2),
            clip(t(1, 50), RationalRate::ONE, SourceEnd::Hold),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 88), t(443, 150));
        assert_eq!(seconds(&clock, 89), t(3, 1));
        assert_eq!(seconds(&clock, 90), t(3, 1));
        assert_eq!(seconds(&clock, 104), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(89));
        // A held frame does not move within its exposure.
        assert_eq!(at(&clock, 100, t(1, 120)).elapsed, RationalTime::ZERO);
        assert_eq!(at(&clock, 100, t(-1, 120)).elapsed, RationalTime::ZERO);
        // The exposure of L runs along the step before it and stops at the end after it.
        let before = at(&clock, 89, t(-1, 120));
        assert_eq!(before.seconds, t(1793, 600));
        assert_eq!(before.elapsed, t(-7, 600));
        assert_eq!(at(&clock, 89, t(1, 120)).elapsed, RationalTime::ZERO);
    }

    #[test]
    fn without_an_arrival_the_source_only_clamps() {
        // Trim 3 starts past the end: every frame is the end.
        let past = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(2, 1),
            clip(t(3, 1), RationalRate::ONE, SourceEnd::Hold),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(seconds(&past, 0), t(3, 1));
        assert_eq!(seconds(&past, 59), t(3, 1));
        assert_eq!(past.last_displayed_frame(), None);

        // A 2 s clip ends before the source does.
        let short =
            ClipClock::new(fps(30), RationalTime::ZERO, t(2, 1), hold(), t(3, 1), false).unwrap();
        assert_eq!(seconds(&short, 59), t(59, 30));
        assert_eq!(short.last_displayed_frame(), None);
        assert!(!short.unsampled_end());

        // A looping source never takes its end.
        let looping =
            ClipClock::standalone(fps(30), MotionRole::Clip, t(3, 1), true, None).unwrap();
        assert_eq!(seconds(&looping, 89), t(89, 30));
        assert_eq!(looping.last_displayed_frame(), None);
    }

    #[test]
    fn a_template_plays_its_intro_and_outro_at_speed_and_maps_its_middle() {
        let role = |hold| overlay(t(1, 2), t(1, 2), hold);
        for hold in [OverlayHold::Once, OverlayHold::Loop, OverlayHold::Stretch] {
            for host in [t(1, 1), t(3, 2), t(2, 1), t(4, 1)] {
                let clock =
                    ClipClock::standalone(fps(8), role(hold), t(2, 1), false, Some(host)).unwrap();
                let last = i64::from(clock.frames()) - 1;
                assert_eq!(seconds(&clock, 2), t(1, 4));
                assert_eq!(seconds(&clock, last - 1), t(7, 4));
                assert_eq!(seconds(&clock, last), t(2, 1));
            }
        }
        for (hold, middle) in [
            (OverlayHold::Once, t(3, 2)),
            (OverlayHold::Loop, t(1, 2)),
            (OverlayHold::Stretch, t(7, 6)),
        ] {
            let clock =
                ClipClock::standalone(fps(8), role(hold), t(2, 1), false, Some(t(4, 1))).unwrap();
            assert_eq!(seconds(&clock, 20), middle);
        }
        let clock = ClipClock::standalone(
            fps(8),
            role(OverlayHold::Stretch),
            t(2, 1),
            false,
            Some(t(3, 2)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 6), t(1, 1));

        // Exact at fractional frame times.
        let ntsc = FrameRate::new(30000, 1001).unwrap();
        for hold in [OverlayHold::Once, OverlayHold::Loop, OverlayHold::Stretch] {
            let clock =
                ClipClock::standalone(ntsc, role(hold), t(2, 1), false, Some(t(10, 1))).unwrap();
            assert_eq!(seconds(&clock, 298), t(58298, 30000));
            assert_eq!(
                seconds(&clock, 19),
                if hold == OverlayHold::Stretch {
                    t(139019, 270000)
                } else {
                    t(19019, 30000)
                }
            );
        }

        // Without a middle the template holds the end of its intro until the outro.
        let clock = ClipClock::standalone(
            fps(8),
            overlay(t(1, 2), t(1, 2), OverlayHold::Once),
            t(1, 1),
            false,
            Some(t(3, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 12), t(1, 2));
        // A caption presenter times like a once overlay.
        assert_eq!(
            ClipMap::template(MotionRole::CaptionPresenter {
                intro: t(3, 20),
                outro: t(3, 20),
            }),
            ClipMap::template(overlay(t(3, 20), t(3, 20), OverlayHold::Once))
        );
    }

    #[test]
    fn a_template_can_arrive_early_and_jump_into_its_outro() {
        // Source 1 s on a 3 s host, no intro or outro, once: frame 29 is L, the rest hold.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(RationalTime::ZERO, RationalTime::ZERO, OverlayHold::Once),
            t(1, 1),
            false,
            Some(t(3, 1)),
        )
        .unwrap();
        assert_eq!(clock.frames(), 90);
        assert_eq!(seconds(&clock, 28), t(28, 30));
        assert_eq!(seconds(&clock, 29), t(1, 1));
        assert_eq!(seconds(&clock, 30), t(1, 1));
        assert_eq!(seconds(&clock, 89), t(1, 1));
        assert_eq!(clock.last_displayed_frame(), Some(29));

        // Source 3 s on a 2 s host with 0.5 s intro and outro, once: the middle is cut at 1.5 s.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(t(1, 2), t(1, 2), OverlayHold::Once),
            t(3, 1),
            false,
            Some(t(2, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 44), t(22, 15));
        assert_eq!(seconds(&clock, 45), t(5, 2));
        assert_eq!(seconds(&clock, 58), t(44, 15));
        assert_eq!(seconds(&clock, 59), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(59));
        // An Echo sample of frame 46 before the cut takes the start of the outro.
        let echo = at(&clock, 46, t(-2, 15));
        assert_eq!(echo.seconds, t(5, 2));
        assert_eq!(echo.elapsed, t(-1, 30));
        // A sample of frame 44 past the cut takes the middle's limit.
        let late = at(&clock, 44, t(1, 20));
        assert_eq!(late.seconds, t(3, 2));
        assert_eq!(late.elapsed, t(1, 30));
    }

    #[test]
    fn an_outro_that_starts_on_l_steps_in_no_time_and_a_shorter_one_is_never_shown() {
        // 25 fps, 2 s host, outro 0.04 s: frame 49 starts the outro and shows the end.
        let clock = ClipClock::standalone(
            fps(25),
            overlay(RationalTime::ZERO, t(1, 25), OverlayHold::Once),
            t(3, 1),
            false,
            Some(t(2, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 48), t(48, 25));
        assert_eq!(seconds(&clock, 49), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(49));
        assert_eq!(at(&clock, 49, t(-1, 100)).seconds, t(3, 1));
        assert_eq!(at(&clock, 49, t(-1, 100)).elapsed, RationalTime::ZERO);
        assert_eq!(at(&clock, 49, t(1, 100)).seconds, t(3, 1));

        // 30 fps, outro 0.01 s: no frame lies in the outro.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(RationalTime::ZERO, t(1, 100), OverlayHold::Once),
            t(3, 1),
            false,
            Some(t(2, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 59), t(59, 30));
        assert_eq!(clock.last_displayed_frame(), None);
        assert!(clock.unsampled_end());

        // An exact 1/30 s outro starts on the last frame: a zero-length step.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(RationalTime::ZERO, t(1, 30), OverlayHold::Once),
            t(3, 1),
            false,
            Some(t(2, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 58), t(58, 30));
        assert_eq!(seconds(&clock, 59), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(59));
    }

    #[test]
    fn declared_loops_wrap_through_samples_and_cuts_stop_them() {
        // A standalone loop is continuous across its seam: 120 px/s at 180° gives −1 px, not +359.
        let clock = ClipClock::standalone(fps(30), MotionRole::Clip, t(3, 1), true, None).unwrap();
        let back = at(&clock, 0, t(-1, 120));
        assert_eq!(back.seconds, t(359, 120));
        assert_eq!(back.elapsed, t(-1, 120));
        let ahead = at(&clock, 89, t(1, 20));
        assert_eq!(ahead.seconds, t(1, 60));
        assert_eq!(ahead.elapsed, t(1, 20));

        // A looping source in a looping clip wraps inside the clip and through its ends.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(6, 1),
            clip(RationalTime::ZERO, RationalRate::ONE, SourceEnd::Loop),
            t(3, 1),
            true,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 90), RationalTime::ZERO);
        assert_eq!(at(&clock, 90, t(-1, 120)).seconds, t(359, 120));
        assert_eq!(at(&clock, 90, t(-1, 120)).elapsed, t(-1, 120));
        assert_eq!(at(&clock, 0, t(-1, 120)).seconds, t(359, 120));
        assert_eq!(at(&clock, 0, t(-1, 120)).elapsed, t(-1, 120));
        assert_eq!(seconds(&clock, 179), t(89, 30));

        // A one-shot source in a looping clip: every wrap is a cut, and no play takes the end.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(6, 1),
            clip(RationalTime::ZERO, RationalRate::ONE, SourceEnd::Loop),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 89), t(89, 30));
        assert_eq!(seconds(&clock, 90), RationalTime::ZERO);
        assert_eq!(seconds(&clock, 179), t(89, 30));
        assert_eq!(clock.last_displayed_frame(), None);
        assert_eq!(at(&clock, 0, t(-1, 120)).seconds, RationalTime::ZERO);
        assert_eq!(at(&clock, 90, t(-1, 120)).seconds, RationalTime::ZERO);
        assert_eq!(at(&clock, 90, t(-1, 120)).elapsed, RationalTime::ZERO);
        assert_eq!(at(&clock, 89, t(1, 20)).seconds, t(3, 1));
        assert_eq!(at(&clock, 89, t(1, 20)).elapsed, t(1, 30));
    }

    #[test]
    fn a_template_loop_middle_wraps_without_a_cut() {
        // Middle 2 s on a 5 s host: two whole plays, so the outro follows without a cut.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(t(1, 2), t(1, 2), OverlayHold::Loop),
            t(3, 1),
            false,
            Some(t(5, 1)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 75), t(1, 2));
        let wrap = at(&clock, 75, t(-1, 120));
        assert_eq!(wrap.seconds, t(299, 120));
        assert_eq!(wrap.elapsed, t(-1, 120));
        assert_eq!(seconds(&clock, 134), t(37, 15));
        assert_eq!(seconds(&clock, 135), t(5, 2));
        assert_eq!(seconds(&clock, 149), t(3, 1));
        assert_eq!(clock.last_displayed_frame(), Some(149));

        // On a 4.7 s host the last play is cut where the outro starts.
        let clock = ClipClock::standalone(
            fps(30),
            overlay(t(1, 2), t(1, 2), OverlayHold::Loop),
            t(3, 1),
            false,
            Some(t(47, 10)),
        )
        .unwrap();
        assert_eq!(seconds(&clock, 125), t(13, 6));
        assert_eq!(seconds(&clock, 126), t(5, 2));
        let late = at(&clock, 125, t(1, 20));
        assert_eq!(late.seconds, t(11, 5));
        assert_eq!(late.elapsed, t(1, 30));
        let early = at(&clock, 126, t(-1, 120));
        assert_eq!(early.seconds, t(5, 2));
        assert_eq!(early.elapsed, RationalTime::ZERO);
    }

    #[test]
    fn a_looping_source_in_a_hold_clip_stops_on_its_last_played_frame() {
        let clock =
            ClipClock::new(fps(30), RationalTime::ZERO, t(4, 1), hold(), t(3, 1), true).unwrap();
        assert_eq!(seconds(&clock, 88), t(88, 30));
        assert_eq!(seconds(&clock, 89), t(89, 30));
        assert_eq!(seconds(&clock, 100), t(89, 30));
        assert_eq!(clock.last_displayed_frame(), Some(89));
        assert_eq!(at(&clock, 100, t(1, 120)).elapsed, RationalTime::ZERO);
        // An Echo sample from before the hold shows the motion before it.
        assert_eq!(at(&clock, 100, t(-1, 2)).seconds, t(17, 6));

        // Starting past the end, nothing plays: the end shows throughout.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(2, 1),
            clip(t(3, 1), RationalRate::ONE, SourceEnd::Hold),
            t(3, 1),
            true,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 0), t(3, 1));
        assert_eq!(seconds(&clock, 59), t(3, 1));
    }

    #[test]
    fn an_error_clip_ends_like_a_one_shot_source() {
        // Admission rejects frames past the end; the last frame still shows it.
        let clock = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(3, 1),
            clip(RationalTime::ZERO, RationalRate::ONE, SourceEnd::Error),
            t(3, 1),
            true,
        )
        .unwrap();
        assert_eq!(seconds(&clock, 88), t(88, 30));
        assert_eq!(seconds(&clock, 89), t(3, 1));
        assert_eq!(at(&clock, 89, t(1, 20)).seconds, t(3, 1));
    }

    #[test]
    fn clip_progress_runs_from_the_first_frame_to_the_last() {
        let clock =
            ClipClock::new(fps(30), RationalTime::ZERO, t(2, 1), hold(), t(3, 1), false).unwrap();
        assert_eq!(
            at(&clock, 0, RationalTime::ZERO).clip_progress,
            RationalTime::ZERO
        );
        assert_eq!(at(&clock, 30, RationalTime::ZERO).clip_progress, t(30, 59));
        assert_eq!(
            at(&clock, 59, RationalTime::ZERO).clip_progress,
            RationalTime::ONE
        );
        assert_eq!(at(&clock, 59, t(1, 60)).clip_progress, RationalTime::ONE);
        let before = at(&clock, 0, t(-1, 60));
        assert_eq!(before.clip_progress, RationalTime::ZERO);
        assert_eq!(before.clip_seconds, t(-1, 60));
        assert_eq!(before.seconds, RationalTime::ZERO);

        // A clip that starts between frames: its first frame reads slightly before the start.
        let clock = ClipClock::new(fps(30), t(1, 100), t(2, 1), hold(), t(3, 1), false).unwrap();
        assert_eq!(clock.first_frame(), 0);
        assert_eq!(at(&clock, 0, RationalTime::ZERO).clip_seconds, t(-1, 100));
        assert_eq!(seconds(&clock, 0), RationalTime::ZERO);

        let single = ClipClock::new(
            fps(30),
            RationalTime::ZERO,
            t(1, 30),
            hold(),
            t(3, 1),
            false,
        )
        .unwrap();
        assert_eq!(single.frames(), 1);
        assert_eq!(at(&single, 0, RationalTime::ZERO).clip_progress, t(1, 2));
    }

    #[test]
    fn a_stretched_template_ends_on_the_last_frame_of_its_window() {
        let clock = ClipClock::standalone(
            fps(30),
            MotionRole::Transition,
            t(2, 1),
            false,
            Some(t(1, 1)),
        )
        .unwrap();
        assert_eq!(clock.frames(), 30);
        assert_eq!(seconds(&clock, 28), t(28, 15));
        assert_eq!(seconds(&clock, 29), t(2, 1));
        assert_eq!(clock.max_speed().unwrap(), t(2, 1));
    }

    #[test]
    fn a_clock_travels_as_its_inputs() {
        let clock = ClipClock::new(
            FrameRate::new(30000, 1001).unwrap(),
            t(1, 2),
            t(7, 2),
            clip(t(1, 50), RationalRate::new(3, 2).unwrap(), SourceEnd::Hold),
            t(3, 1),
            false,
        )
        .unwrap();
        let wire = serde_json::to_value(clock).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "fps": "30000/1001",
                "start": "1/2",
                "duration": "7/2",
                "map": { "type": "clip", "trim": "1/50", "rate": "3/2", "end": "hold" },
                "source": "3/1",
                "looping": false,
            })
        );
        assert_eq!(serde_json::from_value::<ClipClock>(wire).unwrap(), clock);
    }
}
