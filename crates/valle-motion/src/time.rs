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

/// The largest gap two clock values may have and still count as equal: 2⁻²⁴ seconds, about
/// 6×10⁻⁸. A frame number of 24 hours at 240 fps needs about 1.8×10⁻⁸.
pub const CLOCK_TOLERANCE_CAP: f64 = 1.0 / 16_777_216.0;

/// How far apart two values computed from the clock may be and still be equal: a few units of
/// floating-point roundoff at their size, at most [`CLOCK_TOLERANCE_CAP`]. The cap keeps large
/// values apart: 2⁵⁰ and 2⁵⁰ + 1 differ.
pub fn clock_tolerance(a: f64, b: f64) -> f64 {
    (4.0 * f64::EPSILON * a.abs().max(b.abs()).max(1.0)).min(CLOCK_TOLERANCE_CAP)
}

/// The integer `value` is within roundoff of, if there is one.
fn clock_integer(value: f64) -> Option<f64> {
    let nearest = value.round();
    // Adding zero turns the -0 of a value just below zero into 0.
    ((value - nearest).abs() <= clock_tolerance(value, nearest)).then_some(nearest + 0.0)
}

/// `Math.floor` of a value computed from the clock. A value within roundoff of an integer is that
/// integer, so a frame boundary reached through seconds × rate lands on its own step; a real
/// subframe sample still selects the step before it.
pub fn clock_floor(value: f64) -> f64 {
    clock_integer(value).unwrap_or_else(|| value.floor())
}

/// `Math.ceil` of a value computed from the clock, with the boundary rule of [`clock_floor`].
pub fn clock_ceil(value: f64) -> f64 {
    clock_integer(value).unwrap_or_else(|| value.ceil())
}

/// `Math.trunc` of a value computed from the clock, with the boundary rule of [`clock_floor`].
pub fn clock_trunc(value: f64) -> f64 {
    clock_integer(value).unwrap_or_else(|| value.trunc())
}

/// `Math.round` of a value computed from the clock: `floor(value + 0.5)`, so a half-step boundary
/// takes the same tolerance as a whole one.
pub fn clock_round(value: f64) -> f64 {
    clock_floor(value + 0.5)
}

/// Whether two values computed from the clock are equal: they differ by no more than roundoff.
pub fn clock_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= clock_tolerance(a, b)
}

/// The exact remainder of `a / b` with the sign of `a`, and whether `a` sits on a multiple of `b`
/// up to roundoff. The test is made in the units of `a`, never through a rounded quotient: the
/// gap to the nearest multiple is exact, and it is compared with the tolerance of `a` and that
/// multiple.
fn clock_remainder_parts(a: f64, b: f64) -> (f64, bool) {
    let remainder = a % b;
    let period = b.abs();
    // Exact: when the remainder is past half the period, it and the period are within a factor
    // of two of each other (Sterbenz).
    let gap = if remainder.abs() <= period * 0.5 {
        remainder
    } else {
        remainder - period.copysign(a)
    };
    (remainder, gap.abs() <= clock_tolerance(a, a - gap))
}

/// JavaScript `a % b` for a value computed from the clock: the sign of `a`, and zero when `a`
/// sits on a multiple of `b` up to roundoff. `b` must not be zero.
pub fn clock_remainder(a: f64, b: f64) -> f64 {
    match clock_remainder_parts(a, b) {
        (_, true) => 0.0f64.copysign(a),
        (remainder, false) => remainder,
    }
}

/// `mod(a, b)` for a value computed from the clock: in `[0, |b|)`, and zero when `a` sits on a
/// multiple of `b` up to roundoff. `b` must not be zero.
pub fn clock_mod(a: f64, b: f64) -> f64 {
    let period = b.abs();
    match clock_remainder_parts(a, b) {
        (_, true) => 0.0,
        (remainder, false) if remainder < 0.0 => {
            let wrapped = remainder + period;
            // A remainder far smaller than the period can round up to it.
            if wrapped < period {
                wrapped
            } else {
                period.next_down()
            }
        }
        (remainder, false) => remainder,
    }
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

#[cfg(test)]
mod clock_tolerance_rules {
    use super::*;

    const RATES: [f64; 6] = [24.0, 25.0, 30.0, 60.0, 30000.0 / 1001.0, 24000.0 / 1001.0];

    #[test]
    fn frame_boundaries_survive_seconds_round_trip_without_moving_subframes() {
        for fps in RATES {
            for frame in -10_000..10_000 {
                let time = f64::from(frame) / fps;
                assert_eq!(clock_floor(time * fps), f64::from(frame), "{fps}, {frame}");
                assert_eq!(clock_ceil(time * fps), f64::from(frame), "{fps}, {frame}");
                assert_eq!(clock_trunc(time * fps), f64::from(frame), "{fps}, {frame}");
                assert_eq!(clock_floor(f64::from(frame) - 1e-8), f64::from(frame - 1));
                assert_eq!(clock_ceil(f64::from(frame) + 1e-8), f64::from(frame + 1));
            }
        }
    }

    #[test]
    fn steps_written_in_seconds_are_exact_on_frame_boundaries() {
        for fps in [24, 25, 30, 48, 50, 60, 120] {
            for frame in 0..5_000 {
                let seconds = f64::from(frame) / f64::from(fps);
                // A step every 0.2 s: frame / fps = n / 5 exactly when fps divides 5 × frame.
                let steps = seconds / 0.2;
                if (5 * frame) % fps == 0 {
                    let n = f64::from(5 * frame / fps);
                    assert_eq!(clock_floor(steps), n, "{fps}, {frame}");
                    assert_eq!(clock_round(steps), n, "{fps}, {frame}");
                    assert_eq!(clock_mod(seconds, 0.2), 0.0, "{fps}, {frame}");
                    assert_eq!(clock_remainder(seconds, 0.2), 0.0, "{fps}, {frame}");
                } else {
                    assert_eq!(clock_floor(steps), steps.floor(), "{fps}, {frame}");
                    assert_ne!(clock_mod(seconds, 0.2), 0.0, "{fps}, {frame}");
                }
                // A switch at i × 0.1 s: frame / fps = i / 10 exactly when fps divides 10 × frame.
                if (10 * frame) % fps == 0 {
                    let threshold = f64::from(10 * frame / fps) * 0.1;
                    assert!(clock_equal(seconds, threshold), "{fps}, {frame}");
                }
            }
        }
        assert_eq!(clock_mod(0.3, 0.1), 0.0);
        assert_eq!(clock_remainder(0.3, 0.1), 0.0);
        assert_eq!(clock_floor(0.3 / 0.1), 3.0);
        assert!(clock_equal(0.299_999_999_999_999_93, 0.3));
        assert_eq!(clock_round(2.5 - 1e-15), 3.0);
        assert_eq!(clock_round(2.4), 2.0);
        assert_eq!(clock_round(-2.5), -2.0);
    }

    #[test]
    fn large_values_keep_their_whole_number_differences() {
        let big = 2f64.powi(50);
        assert!(!clock_equal(big, big + 1.0));
        assert_eq!(clock_floor(big + 1.0), big + 1.0);
        assert_eq!(clock_tolerance(big, big), CLOCK_TOLERANCE_CAP);
        // The quotient is within roundoff of 1, but the gap to the multiple, 1, is not.
        assert_eq!(clock_mod(big - 1.0, big), big - 1.0);
        assert_eq!(clock_remainder(big - 1.0, big), big - 1.0);
        assert_eq!(clock_mod(-(big - 1.0), big), 1.0);
    }

    #[test]
    fn remainders_stay_in_range_with_the_sign_of_the_dividend() {
        // A fixed linear congruential sequence keeps the check deterministic.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            let exponent = ((state >> 3) % 80) as i32 - 40;
            (unit * 2.0 - 1.0) * 2f64.powi(exponent)
        };
        for _ in 0..400_000 {
            let a = next();
            let b = next();
            if b == 0.0 {
                continue;
            }
            let wrapped = clock_mod(a, b);
            assert!(
                (0.0..b.abs()).contains(&wrapped),
                "mod({a}, {b}) = {wrapped}"
            );
            let remainder = clock_remainder(a, b);
            assert!(remainder.abs() < b.abs(), "{a} % {b} = {remainder}");
            assert!(
                remainder == 0.0 || remainder.signum() == a.signum(),
                "{a} % {b} = {remainder}"
            );
        }
    }

    #[test]
    fn a_remainder_that_rounds_up_to_the_period_stays_below_it() {
        let period = 2f64.powi(40);
        let a = -2f64.powi(-20);
        assert!(clock_mod(a, period) < period);
        assert_eq!(clock_mod(a, period), period.next_down());
    }
}
