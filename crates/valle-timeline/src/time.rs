//! Time for editing and media: integer ticks, rational frame rates, and every conversion between
//! them and seconds. Conversions happen once, at the boundary, and round one of two ways: integer
//! and rational results round down (`div_euclid`), f64 seconds round to the nearest tick. The only
//! exception is a frame's start, which rounds up so that `frame_at(frame_start(f)) == f`.
//!
//! Ticks never reach scene code: JS numbers hold integers exactly only up to 2⁵³, about 9.85 hours
//! of ticks. Scenes, JSON and the SDK speak seconds.

use std::fmt;
use std::ops::{Add, Neg, Sub};

/// Ticks per second: 2¹²·3⁴·5⁶·7², the tick of Premiere Pro. A frame at the common rates (23.976
/// to 120 fps) and a sample at 8–192 kHz (the 44.1 kHz family included) are whole numbers of
/// ticks, and so is every decimal with up to 6 places.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

const TPS: i128 = TICKS_PER_SECOND as i128;

/// A point in time or a length of time. i64 covers about ±420 days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    /// Seconds written as decimal text (a JSON number), parsed exactly: up to 6 decimal places
    /// convert without rounding, more round to the nearest tick.
    pub fn parse_seconds(text: &str) -> Result<Tick, String> {
        let d = Decimal::parse(text).ok_or_else(|| {
            format!("invalid seconds {text:?}: expected a decimal number such as 2.5")
        })?;
        let ticks = d.times(TPS).ok_or_else(|| out_of_range(text))?;
        Tick::from_i128(ticks).ok_or_else(|| out_of_range(text))
    }

    /// The shortest decimal that parses back to this tick: at most 6 places for most times, about
    /// 12 for the frame boundaries of rates like 29.97.
    pub fn seconds_text(self) -> String {
        let ticks = i128::from(self.0).abs();
        // 12 places put the decimal within 0.13 ticks, so the loop always ends.
        for places in 0..=12 {
            let scale = 10i128.pow(places);
            let scaled = div_nearest(ticks * scale, TPS);
            if div_nearest(scaled * TPS, scale) == ticks {
                let sign = if self.0 < 0 && scaled != 0 { "-" } else { "" };
                let whole = scaled / scale;
                return match places {
                    0 => format!("{sign}{whole}"),
                    _ => {
                        let width = places as usize;
                        format!("{sign}{whole}.{:0width$}", scaled % scale)
                    }
                };
            }
        }
        unreachable!("12 decimal places resolve every tick")
    }

    /// Seconds given as an f64 by scene code (`<Video time>`, a `TimeScope` offset), to the nearest
    /// tick. Rounding down instead would put 0.3 on the tick before, from its binary error.
    pub fn from_seconds(seconds: f64) -> Result<Tick, String> {
        let ticks = (seconds * TICKS_PER_SECOND as f64).round();
        // i64::MAX as f64 rounds up to 2⁶³, which is itself out of range.
        if !ticks.is_finite() || ticks.abs() >= i64::MAX as f64 {
            return Err(out_of_range(&seconds.to_string()));
        }
        Ok(Tick(ticks as i64))
    }

    /// The time in seconds, for Motion evaluation.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    fn from_i128(ticks: i128) -> Option<Tick> {
        i64::try_from(ticks).ok().map(Tick)
    }
}

// Inputs are bounded long before i64 runs out, so overflow here is a bug, not an input error.
impl Add for Tick {
    type Output = Tick;
    fn add(self, other: Tick) -> Tick {
        Tick(self.0.checked_add(other.0).expect("tick overflow"))
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, other: Tick) -> Tick {
        Tick(self.0.checked_sub(other.0).expect("tick overflow"))
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.checked_neg().expect("tick overflow"))
    }
}

/// A frame rate in frames per second, as a reduced fraction such as 30/1 or 30000/1001.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
    num: i64,
    den: i64,
}

/// Bound on a rate's numerator and denominator, so tick arithmetic stays inside i128.
const MAX_TERM: i64 = 1_000_000_000;

impl Rate {
    pub fn new(num: i64, den: i64) -> Result<Rate, String> {
        if num <= 0 || den <= 0 {
            return Err(format!("frame rate {num}/{den} must be positive"));
        }
        let g = gcd(num, den);
        let (num, den) = (num / g, den / g);
        if num > MAX_TERM || den > MAX_TERM {
            return Err(format!("frame rate {num}/{den} has terms above {MAX_TERM}"));
        }
        Ok(Rate { num, den })
    }

    /// `30`, `29.97` (exactly 2997/100) or `30000/1001`.
    pub fn parse(text: &str) -> Result<Rate, String> {
        let bad = || format!("invalid frame rate {text:?}: expected 30, 29.97 or 30000/1001");
        if let Some((num, den)) = text.split_once('/') {
            let term = |t: &str| {
                let digits = !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
                digits
                    .then(|| t.parse::<i64>().ok())
                    .flatten()
                    .ok_or_else(bad)
            };
            return Rate::new(term(num)?, term(den)?);
        }
        let d = Decimal::parse(text).ok_or_else(bad)?;
        let (num, den) = d.fraction().ok_or_else(bad)?;
        Rate::new(num, den)
    }

    /// A rate given as a JS or JSON number, read through its shortest decimal: 29.97 is 2997/100,
    /// not 30000/1001.
    pub fn from_f64(fps: f64) -> Result<Rate, String> {
        if !fps.is_finite() {
            return Err(format!("invalid frame rate {fps}"));
        }
        Rate::parse(&fps.to_string())
    }

    pub fn num(self) -> i64 {
        self.num
    }

    pub fn den(self) -> i64 {
        self.den
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Where frame `frame` starts: the first tick at or after `frame` frame lengths.
    pub fn frame_start(self, frame: i64) -> Result<Tick, String> {
        let n = i128::from(frame)
            .checked_mul(TPS * i128::from(self.den))
            .ok_or_else(|| format!("frame {frame} is out of range"))?;
        let ticks = -(-n).div_euclid(i128::from(self.num));
        Tick::from_i128(ticks).ok_or_else(|| format!("frame {frame} is out of range"))
    }

    /// The frame a tick falls in.
    pub fn frame_at(self, t: Tick) -> i64 {
        let frame = (i128::from(t.0) * i128::from(self.num)).div_euclid(TPS * i128::from(self.den));
        // |t| < 2⁶³ and the rate is at most 10⁹ fps, so the frame is well inside i64.
        frame as i64
    }

    /// Where the `k`th of `n` shutter samples falls from its frame's time: the middles of `n` even
    /// steps across an exposure of `angle`° of a frame, centered on it, rounded down.
    pub fn shutter(self, angle: u32, k: u32, n: u32) -> Tick {
        // (k + ½) / n − ½ of angle / 360 frames.
        let steps = 2 * i128::from(k) + 1 - i128::from(n);
        let ticks = TPS * i128::from(self.den) * i128::from(angle) * steps;
        let per = i128::from(self.num) * 720 * i128::from(n);
        Tick::from_i128(ticks.div_euclid(per)).expect("an exposure within a frame")
    }

    /// `t` on the start of the nearest frame when it is less than a thousandth of a frame from
    /// it, else `t`: seconds written to 6 places cut exactly where the frames of rates like 29.97
    /// are.
    pub fn snap(self, t: Tick) -> Tick {
        let (num, den) = (i128::from(self.num), TPS * i128::from(self.den));
        let scaled = i128::from(t.0) * num;
        let frame = (2 * scaled + den).div_euclid(2 * den);
        let off = (scaled - frame * den).abs();
        match i64::try_from(frame).ok().map(|f| self.frame_start(f)) {
            Some(Ok(start)) if off * 1000 < den => start,
            _ => t,
        }
    }

    /// How many frames, counting from frame 0, start before `duration`.
    pub fn frames_in(self, duration: Tick) -> i64 {
        if duration.0 <= 0 {
            0
        } else {
            self.frame_at(Tick(duration.0 - 1)) + 1
        }
    }
}

/// A playback rate, as a `TimeScope` takes it: a reduced fraction of any sign, read through its
/// shortest decimal, so 1.1 is exactly 11/10. 0 holds the time still; below 0 it runs backwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Speed {
    num: i64,
    den: i64,
}

impl Speed {
    /// The source's own speed.
    pub const ONE: Speed = Speed { num: 1, den: 1 };

    pub fn from_f64(rate: f64) -> Result<Speed, String> {
        let bad = || format!("rate {rate} is not a finite number between -10¹⁸ and 10¹⁸");
        if !rate.is_finite() {
            return Err(bad());
        }
        // Rust prints an f64 as its shortest decimal, without an exponent.
        let (num, den) = Decimal::parse(&rate.to_string())
            .and_then(|d| d.fraction())
            .ok_or_else(bad)?;
        // Terms up to 10¹⁸ keep `t × num` inside i128.
        if num.abs() > 1_000_000_000_000_000_000 || den > 1_000_000_000_000_000_000 {
            return Err(bad());
        }
        Ok(Speed { num, den })
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// `t × rate`, rounded down.
    pub fn apply(self, t: Tick) -> Result<Tick, String> {
        let ticks = (i128::from(t.0) * i128::from(self.num)).div_euclid(i128::from(self.den));
        Tick::from_i128(ticks).ok_or_else(|| out_of_range(&(ticks / TPS).to_string()))
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// A decimal number as written in JSON: `digits × 10^exponent`, with its sign.
struct Decimal {
    negative: bool,
    digits: i128,
    exponent: i32,
}

/// More significant digits than JSON writers ever print (17 for an f64), small enough that
/// `digits × TICKS_PER_SECOND` stays inside i128.
const MAX_DIGITS: usize = 24;

impl Decimal {
    /// JSON number syntax: `-?digits(.digits)?([eE][+-]?digits)?`.
    fn parse(text: &str) -> Option<Decimal> {
        let (negative, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => {
                let e = e.strip_prefix('+').unwrap_or(e);
                let digits = e.strip_prefix('-').unwrap_or(e);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                // Beyond ±400 the value is far outside the tick range or rounds to zero.
                (
                    m,
                    e.parse::<i32>()
                        .unwrap_or(if e.starts_with('-') { -400 } else { 400 }),
                )
            }
            None => (rest, 0),
        };
        let (whole, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let is_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !is_digits(whole) || (mantissa.contains('.') && !is_digits(frac)) {
            return None;
        }
        let all = format!("{whole}{frac}");
        let significant = all.trim_start_matches('0');
        if significant.len() > MAX_DIGITS {
            return None;
        }
        Some(Decimal {
            negative,
            digits: significant.parse().unwrap_or(0),
            exponent: exponent.clamp(-400, 400) - frac.len() as i32,
        })
    }

    /// `self × factor`, rounded to the nearest integer; `None` past i128.
    fn times(&self, factor: i128) -> Option<i128> {
        let n = self.digits * factor;
        let value = if self.exponent >= 0 {
            10i128
                .checked_pow(self.exponent as u32)
                .and_then(|p| n.checked_mul(p))?
        } else {
            match 10i128.checked_pow(self.exponent.unsigned_abs()) {
                Some(p) => div_nearest(n, p),
                // 10^39 and beyond: n < 10^36, so the result rounds to zero.
                None => 0,
            }
        };
        Some(if self.negative { -value } else { value })
    }

    /// The value as a reduced fraction, its sign on the numerator, if it fits i64.
    fn fraction(&self) -> Option<(i64, i64)> {
        if self.digits == 0 {
            return Some((0, 1));
        }
        let (num, den) = if self.exponent >= 0 {
            (
                self.digits
                    .checked_mul(10i128.checked_pow(self.exponent as u32)?)?,
                1,
            )
        } else {
            (
                self.digits,
                10i128.checked_pow(self.exponent.unsigned_abs())?,
            )
        };
        let g = gcd_i128(num, den);
        let num = if self.negative { -num / g } else { num / g };
        Some((i64::try_from(num).ok()?, i64::try_from(den / g).ok()?))
    }
}

/// `n / d` for `n ≥ 0`, `d > 0`, rounded to the nearest integer, halves up.
fn div_nearest(n: i128, d: i128) -> i128 {
    let (q, r) = (n / d, n % d);
    if 2 * r >= d { q + 1 } else { q }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn gcd_i128(a: i128, b: i128) -> i128 {
    if b == 0 { a } else { gcd_i128(b, a % b) }
}

fn out_of_range(text: &str) -> String {
    format!("{text} seconds is outside the supported range of about ±420 days")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(text: &str) -> Rate {
        Rate::parse(text).unwrap()
    }

    fn seconds(text: &str) -> Tick {
        Tick::parse_seconds(text).unwrap()
    }

    #[test]
    fn common_frames_and_samples_are_whole_ticks() {
        for r in [
            "24000/1001",
            "24",
            "25",
            "30000/1001",
            "30",
            "50",
            "60000/1001",
            "60",
            "120",
        ] {
            let r = rate(r);
            assert_eq!(TPS * i128::from(r.den()) % i128::from(r.num()), 0, "{r}");
        }
        for hz in [
            8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
        ] {
            assert_eq!(TICKS_PER_SECOND % hz, 0, "{hz} Hz");
        }
    }

    #[test]
    fn decimal_seconds_parse_exactly() {
        assert_eq!(seconds("0"), Tick(0));
        assert_eq!(seconds("1"), Tick(TICKS_PER_SECOND));
        assert_eq!(seconds("-2.5"), Tick(-TICKS_PER_SECOND * 5 / 2));
        assert_eq!(seconds("0.000001"), Tick(TICKS_PER_SECOND / 1_000_000));
        assert_eq!(seconds("0.3"), Tick(TICKS_PER_SECOND / 10 * 3));
        assert_eq!(seconds("1.5e2"), Tick(TICKS_PER_SECOND * 150));
        assert_eq!(seconds("25E-1"), seconds("2.5"));
        assert_eq!(seconds("0.0000000000001"), Tick(0));
        // 7 places and beyond round to the nearest tick (half a tick is 1.968…e-12 s).
        assert_eq!(seconds("0.0000000000019683"), Tick(0));
        assert_eq!(seconds("0.0000000000019685"), Tick(1));
        assert_eq!(seconds("1e-400"), Tick(0));
    }

    #[test]
    fn bad_or_huge_seconds_fail() {
        for bad in [
            "", "-", ".5", "1.", "+1", "1e", "1e+", "abc", "NaN", "0x10", "1 ",
        ] {
            assert!(Tick::parse_seconds(bad).is_err(), "{bad:?}");
        }
        assert!(Tick::parse_seconds("1e10").is_err());
        assert!(Tick::parse_seconds("1e400").is_err());
        assert!(Tick::parse_seconds("3e7").is_ok());
    }

    #[test]
    fn seconds_print_shortest() {
        assert_eq!(seconds("12").seconds_text(), "12");
        assert_eq!(seconds("-1.25").seconds_text(), "-1.25");
        assert_eq!(seconds("0.000001").seconds_text(), "0.000001");
        assert_eq!(Tick(1).seconds_text(), "0.000000000004");
        assert_eq!(Tick(-1).seconds_text(), "-0.000000000004");
        let ntsc = rate("30000/1001");
        assert_eq!(
            ntsc.frame_start(1).unwrap().seconds_text(),
            "0.033366666667"
        );
        for f in -500..500 {
            for r in [ntsc, rate("2997/100"), rate("44100")] {
                let t = r.frame_start(f).unwrap();
                assert_eq!(seconds(&t.seconds_text()), t, "{r} frame {f}");
            }
        }
        for t in [i64::MAX, i64::MIN + 1, 123_456_789_012_345] {
            assert_eq!(seconds(&Tick(t).seconds_text()), Tick(t));
        }
    }

    #[test]
    fn f64_seconds_round_to_nearest() {
        assert_eq!(Tick::from_seconds(0.3), Ok(seconds("0.3")));
        assert_eq!(
            Tick::from_seconds(1.0 / 30.0),
            Ok(rate("30").frame_start(1).unwrap())
        );
        assert_eq!(Tick::from_seconds(-0.1), Ok(seconds("-0.1")));
        assert!(Tick::from_seconds(f64::NAN).is_err());
        assert!(Tick::from_seconds(1e12).is_err());
        assert_eq!(seconds("2.5").seconds(), 2.5);
    }

    #[test]
    fn rates_parse_and_print() {
        assert_eq!(rate("30").to_string(), "30");
        assert_eq!(rate("29.97").to_string(), "2997/100");
        assert_eq!(rate("30000/1001").to_string(), "30000/1001");
        assert_eq!(rate("60/2").to_string(), "30");
        assert_eq!(Rate::from_f64(29.97), Ok(rate("2997/100")));
        assert_eq!(Rate::from_f64(30.0), Ok(rate("30")));
        for bad in [
            "0", "-30", "30/0", "0/1", "1/2/3", "abc", "", "30/", "/1", "+30/1", "1e10",
        ] {
            assert!(Rate::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Rate::from_f64(f64::INFINITY).is_err());
    }

    #[test]
    fn shutter_samples_center_on_the_frame() {
        let r = rate("30");
        let frame = r.frame_start(1).unwrap().0;
        let at = |angle, k, n| r.shutter(angle, k, n).0;
        assert_eq!(at(180, 0, 1), 0);
        assert_eq!([at(360, 0, 2), at(360, 1, 2)], [-frame / 4, frame / 4]);
        assert_eq!(
            [at(180, 0, 4), at(180, 3, 4)],
            [-frame * 3 / 16, frame * 3 / 16]
        );
        // A third of an 11 fps frame is no whole number of ticks: down, not toward zero.
        let odd = rate("11");
        assert_eq!(odd.shutter(360, 0, 3).0, -(odd.shutter(360, 2, 3).0) - 1);
    }

    #[test]
    fn speeds_scale_exactly_and_round_down() {
        let speed = |r| Speed::from_f64(r).unwrap();
        assert_eq!(speed(1.1), Speed { num: 11, den: 10 });
        assert_eq!(speed(-0.5), Speed { num: -1, den: 2 });
        assert_eq!(speed(0.0), Speed { num: 0, den: 1 });
        assert_eq!(speed(1.1).apply(seconds("3")), Ok(seconds("3.3")));
        assert_eq!(speed(1.0 / 3.0).apply(Tick(2)), Ok(Tick(0)));
        assert_eq!(speed(-0.5).apply(Tick(3)), Ok(Tick(-2)));
        assert_eq!(speed(0.0).apply(seconds("7")), Ok(Tick(0)));
        assert!(Speed::from_f64(f64::NAN).is_err());
        assert!(Speed::from_f64(1e19).is_err());
        assert!(Speed::from_f64(1e-19).is_err());
        assert!(speed(1e9).apply(seconds("1000000")).is_err());
    }

    #[test]
    fn frames_round_trip() {
        for r in ["30", "30000/1001", "2997/100", "24000/1001", "120", "1/3"] {
            let r = rate(r);
            for f in -1000..1000 {
                let start = r.frame_start(f).unwrap();
                assert_eq!(r.frame_at(start), f, "{r} frame {f}");
                assert_eq!(r.frame_at(start - Tick(1)), f - 1, "{r} before frame {f}");
            }
        }
        assert!(rate("30").frame_start(i64::MAX).is_err());
    }

    #[test]
    fn frame_counts() {
        assert_eq!(rate("30").frames_in(seconds("4")), 120);
        assert_eq!(rate("30").frames_in(seconds("4.01")), 121);
        assert_eq!(rate("30000/1001").frames_in(seconds("1")), 30);
        assert_eq!(rate("30").frames_in(Tick(1)), 1);
        assert_eq!(rate("30").frames_in(Tick(0)), 0);
        assert_eq!(rate("30").frames_in(seconds("-1")), 0);
    }

    #[test]
    fn cuts_snap_to_frames_within_a_thousandth_of_one() {
        let ntsc = rate("30000/1001");
        // Frame 1 at 29.97 starts 0.0333666… s in; 6 places land on it.
        assert_eq!(ntsc.snap(seconds("0.033367")), ntsc.frame_start(1).unwrap());
        assert_eq!(
            ntsc.snap(seconds("10.010000")),
            ntsc.frame_start(300).unwrap()
        );
        // A hundredth of a frame off stays where it is.
        let off = seconds("0.0337");
        assert_eq!(ntsc.snap(off), off);
        assert_eq!(rate("30").snap(seconds("2")), seconds("2"));
        assert_eq!(
            rate("30").snap(seconds("-0.0333333")),
            rate("30").frame_start(-1).unwrap()
        );
    }
}
