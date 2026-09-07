//! Deterministic frame/sample identity quantization for render admission.

use crate::internal::time::{FrameRate, RationalTime, TimeError};
pub(crate) use crate::quantize::quantize_canonical_scalar;

/// A visual interval obtained by quantizing both exact boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInterval {
    pub start_frame: i64,
    pub end_frame: i64,
    pub duration_frames: i64,
}

/// An audio interval obtained by quantizing both exact boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleInterval {
    pub start_sample: i64,
    pub end_sample: i64,
    pub duration_samples: i64,
}

/// Quantizes exact seconds to a frame boundary without passing through `f64`.
pub fn quantize_frame_boundary(
    time: RationalTime,
    frame_rate: FrameRate,
) -> Result<i64, TimeError> {
    let numerator = (time.numerator() as i128)
        .checked_mul(frame_rate.numerator() as i128)
        .ok_or(TimeError::Overflow)?;
    let denominator = (time.denominator() as u128)
        .checked_mul(frame_rate.denominator() as u128)
        .ok_or(TimeError::Overflow)?;
    round_ratio_half_away_from_zero(numerator, denominator)
}

/// Quantizes exact seconds to a sample boundary without passing through `f64`.
pub fn quantize_sample_boundary(time: RationalTime, sample_rate: u32) -> Result<i64, TimeError> {
    if sample_rate == 0 {
        return Err(TimeError::InvalidSampleRate);
    }
    let numerator = (time.numerator() as i128)
        .checked_mul(sample_rate as i128)
        .ok_or(TimeError::Overflow)?;
    round_ratio_half_away_from_zero(numerator, time.denominator() as u128)
}

/// Quantizes `[start, start + duration)` by its exact boundaries.
///
/// `duration` is never rounded independently. This is the same rule Sequence accumulation must use:
/// advance the exact cursor first, then quantize each boundary.
pub fn quantize_frame_interval(
    start: RationalTime,
    duration: RationalTime,
    frame_rate: FrameRate,
) -> Result<FrameInterval, TimeError> {
    let end = start.checked_add(duration)?;
    let start_frame = quantize_frame_boundary(start, frame_rate)?;
    let end_frame = quantize_frame_boundary(end, frame_rate)?;
    let duration_frames = end_frame
        .checked_sub(start_frame)
        .ok_or(TimeError::Overflow)?;
    Ok(FrameInterval {
        start_frame,
        end_frame,
        duration_frames,
    })
}

/// Quantizes `[start, start + duration)` to sample identities by its exact boundaries.
pub fn quantize_sample_interval(
    start: RationalTime,
    duration: RationalTime,
    sample_rate: u32,
) -> Result<SampleInterval, TimeError> {
    if sample_rate == 0 {
        return Err(TimeError::InvalidSampleRate);
    }
    let end = start.checked_add(duration)?;
    let start_sample = quantize_sample_boundary(start, sample_rate)?;
    let end_sample = quantize_sample_boundary(end, sample_rate)?;
    let duration_samples = end_sample
        .checked_sub(start_sample)
        .ok_or(TimeError::Overflow)?;
    Ok(SampleInterval {
        start_sample,
        end_sample,
        duration_samples,
    })
}

fn round_ratio_half_away_from_zero(numerator: i128, denominator: u128) -> Result<i64, TimeError> {
    if denominator == 0 {
        return Err(TimeError::ZeroDenominator);
    }

    let magnitude = numerator.unsigned_abs();
    let quotient = magnitude / denominator;
    let remainder = magnitude % denominator;
    // `remainder >= denominator - remainder` is equivalent to `2 * remainder >= denominator`,
    // without risking an unsigned multiplication overflow.
    let rounded_magnitude = if remainder >= denominator - remainder {
        quotient.checked_add(1).ok_or(TimeError::Overflow)?
    } else {
        quotient
    };

    if rounded_magnitude == 0 {
        return Ok(0);
    }
    if numerator < 0 {
        let minimum_magnitude = (i64::MAX as u128) + 1;
        if rounded_magnitude == minimum_magnitude {
            return Ok(i64::MIN);
        }
        let rounded = i64::try_from(rounded_magnitude).map_err(|_| TimeError::Overflow)?;
        rounded.checked_neg().ok_or(TimeError::Overflow)
    } else {
        i64::try_from(rounded_magnitude).map_err(|_| TimeError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_ties_go_away_from_zero() {
        let fps = FrameRate::new(30, 1).unwrap();
        assert_eq!(
            quantize_frame_boundary(RationalTime::new(1, 60).unwrap(), fps),
            Ok(1)
        );
        assert_eq!(
            quantize_frame_boundary(RationalTime::new(-1, 60).unwrap(), fps),
            Ok(-1)
        );
    }

    #[test]
    fn exact_boundaries_not_independent_duration() {
        let interval = quantize_frame_interval(
            RationalTime::new(49, 3_000).unwrap(),
            RationalTime::new(1, 1_500).unwrap(),
            FrameRate::new(30, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(interval.start_frame, 0);
        assert_eq!(interval.end_frame, 1);
        assert_eq!(interval.duration_frames, 1);
    }
}
