//! Exact time helpers shared by Motion phase, signal, and Glass evaluation.
//!
//! `FrameRate`, `RationalTime`, and `SampleTime` remain the Timeline public time kernel. This
//! module only defines Motion's conversions between those primitives; it does not duplicate their
//! wire representation.

use valle_timeline::internal::SampleTime;
use valle_timeline::{ExactRational, FrameRate, RationalTime, TimeError};

/// Exact clip-local sample at the left boundary of `frame`.
pub fn sample_time_at_frame(frame: i64, frame_rate: FrameRate) -> Result<SampleTime, TimeError> {
    let frame_count = ExactRational::new(frame, 1)?;
    let seconds = frame_count.checked_div(frame_rate.into_exact())?;
    Ok(SampleTime::new(RationalTime::from_exact(seconds)))
}

/// Covering frame for an exact clip-local sample, using mathematical floor.
pub fn frame_at_sample_floor(sample: SampleTime, frame_rate: FrameRate) -> Result<i64, TimeError> {
    let frames = sample
        .composition()
        .into_exact()
        .checked_mul(frame_rate.into_exact())?;
    Ok(frames
        .numerator()
        .div_euclid(i64::from(frames.denominator())))
}

/// Lossy projection used only by continuous Motion math after exact frame/sample identity exists.
pub fn frame_rate_as_f64(frame_rate: FrameRate) -> f64 {
    frame_rate.numerator() as f64 / f64::from(frame_rate.denominator())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_sample_roundtrip_is_exact_for_fractional_rates() {
        let rate = FrameRate::new(30_000, 1_001).unwrap();
        let sample = sample_time_at_frame(1_000, rate).unwrap();
        assert_eq!(sample.composition(), RationalTime::new(1_001, 30).unwrap());
        assert_eq!(frame_at_sample_floor(sample, rate), Ok(1_000));
    }

    #[test]
    fn covering_frame_uses_exact_floor() {
        let rate = FrameRate::new(30, 1).unwrap();
        let sample = SampleTime::new(RationalTime::new(1, 20).unwrap());
        assert_eq!(frame_at_sample_floor(sample, rate), Ok(1));
    }
}
