//! Data-to-output scales using deterministic math. Tick rounding follows d3-array's tickIncrement,
//! ticks, and nice semantics (ISC). Preserve the negative reciprocal-step convention to avoid
//! decimal-step accumulation error.

use serde::Serialize;
use valle_draw::math;

/// Exact f64 geometric thresholds for the 1-2-5 step sequence, matching JavaScript square-root
/// results.
const E10: f64 = 7.0710678118654755; // √50
const E5: f64 = 3.1622776601683795; // √10
const E2: f64 = std::f64::consts::SQRT_2; // Square root of 2 matching the JavaScript double value.

/// Suggested tick increment; negative values encode the reciprocal step size.
pub fn tick_increment(start: f64, stop: f64, count: usize) -> f64 {
    let step = (stop - start) / (count.max(1) as f64);
    if !step.is_finite() || step <= 0.0 {
        return f64::NAN;
    }
    let power = math::log10(step).floor();
    let error = step / math::pow(10.0, power);
    let divisor = if error >= E10 {
        10.0
    } else if error >= E5 {
        5.0
    } else if error >= E2 {
        2.0
    } else {
        1.0
    };
    if power >= 0.0 {
        divisor * math::pow(10.0, power)
    } else {
        -math::pow(10.0, -power) / divisor
    }
}

/// Ticks within the inclusive interval; empty or invalid domains return an empty list.
pub fn ticks(start: f64, stop: f64, count: usize) -> Vec<f64> {
    if start == stop {
        return if start.is_finite() {
            vec![start]
        } else {
            Vec::new()
        };
    }
    let (a, b, reverse) = if stop < start {
        (stop, start, true)
    } else {
        (start, stop, false)
    };
    let inc = tick_increment(a, b, count);
    if !inc.is_finite() || inc == 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if inc > 0.0 {
        let i0 = (a / inc).ceil();
        let i1 = (b / inc).floor();
        let n = (i1 - i0 + 1.0).max(0.0) as usize;
        out.reserve(n);
        for j in 0..n {
            out.push((i0 + j as f64) * inc);
        }
    } else {
        let inv = -inc;
        let i0 = (a * inv).ceil();
        let i1 = (b * inv).floor();
        let n = (i1 - i0 + 1.0).max(0.0) as usize;
        out.reserve(n);
        for j in 0..n {
            out.push((i0 + j as f64) / inv);
        }
    }
    if reverse {
        out.reverse();
    }
    out
}

/// Expand domain endpoints to convenient tick values. Preserve equal endpoints for the caller to
/// expand separately.
pub fn nice(mut start: f64, mut stop: f64, count: usize) -> (f64, f64) {
    let mut prestep = f64::NAN;
    loop {
        let step = tick_increment(start, stop, count);
        if !step.is_finite() || step == 0.0 || step == prestep {
            return (start, stop);
        }
        if step > 0.0 {
            start = (start / step).floor() * step;
            stop = (stop / step).ceil() * step;
        } else {
            // Use reciprocal-step multiplication and division to reduce decimal rounding error.
            start = (start * step).ceil() / step;
            stop = (stop * step).floor() / step;
        }
        prestep = step;
    }
}

// Numeric scales.

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearScale {
    pub domain: (f64, f64),
    /// Output interval; reverse its endpoints to invert the y axis.
    pub range: (f64, f64),
}

impl LinearScale {
    pub fn new(domain: (f64, f64), range: (f64, f64)) -> Self {
        LinearScale { domain, range }
    }

    pub fn map(&self, v: f64) -> f64 {
        let (d0, d1) = self.domain;
        let (r0, r1) = self.range;
        if d1 == d0 {
            return (r0 + r1) / 2.0; // Map a degenerate domain to the range midpoint.
        }
        r0 + (v - d0) / (d1 - d0) * (r1 - r0)
    }

    pub fn invert(&self, px: f64) -> f64 {
        let (d0, d1) = self.domain;
        let (r0, r1) = self.range;
        if r1 == r0 {
            return d0;
        }
        d0 + (px - r0) / (r1 - r0) * (d1 - d0)
    }

    pub fn nice(&mut self, count: usize) -> &mut Self {
        self.domain = nice(self.domain.0, self.domain.1, count);
        self
    }

    pub fn ticks(&self, count: usize) -> Vec<f64> {
        ticks(self.domain.0, self.domain.1, count)
    }
}

/// Expand a constant domain toward zero; an all-zero domain becomes [0,1].
pub fn extent_or_default(min: f64, max: f64) -> (f64, f64) {
    if !min.is_finite() || !max.is_finite() {
        return (0.0, 1.0);
    }
    if min != max {
        return (min, max);
    }
    if min == 0.0 {
        (0.0, 1.0)
    } else if min > 0.0 {
        (0.0, min)
    } else {
        (min, 0.0)
    }
}

/// Finite extent of a value sequence; return None for empty or entirely non-finite input.
pub fn extent(values: &[f64]) -> Option<(f64, f64)> {
    let mut iter = values.iter().copied().filter(|v| v.is_finite());
    let first = iter.next()?;
    Some(iter.fold((first, first), |(min, max), v| (min.min(v), max.max(v))))
}

// Categorical scales.

/// Band scale following d3-scale semantics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BandScale {
    pub range: (f64, f64),
    pub count: usize,
    /// Inner band gap as a fraction of step, in [0,1).
    pub padding_inner: f64,
    /// Outer padding as a fraction of step.
    pub padding_outer: f64,
    /// Distribution of leftover space; 0.5 centers the bands.
    pub align: f64,
}

impl BandScale {
    /// Default to contiguous category bands. Geometry consumers apply any gaps within each
    /// category.
    pub fn new(range: (f64, f64), count: usize) -> Self {
        BandScale {
            range,
            count,
            padding_inner: 0.0,
            padding_outer: 0.0,
            align: 0.5,
        }
    }

    pub fn step(&self) -> f64 {
        let span = self.range.1 - self.range.0;
        let n = self.count.max(1) as f64;
        let denom = (n - self.padding_inner + self.padding_outer * 2.0).max(1.0);
        span / denom
    }

    pub fn bandwidth(&self) -> f64 {
        (self.step() * (1.0 - self.padding_inner)).max(0.0)
    }

    fn origin(&self) -> f64 {
        let span = self.range.1 - self.range.0;
        let n = self.count.max(1) as f64;
        self.range.0 + (span - self.step() * (n - self.padding_inner)) * self.align
    }

    /// Half-open interval occupied by band i.
    pub fn band(&self, i: usize) -> (f64, f64) {
        let s = self.origin() + self.step() * i as f64;
        (s, s + self.bandwidth())
    }

    pub fn center(&self, i: usize) -> f64 {
        let (a, b) = self.band(i);
        (a + b) / 2.0
    }
}

/// Point scale with category positions on the range boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointScale {
    pub range: (f64, f64),
    pub count: usize,
}

impl PointScale {
    pub fn new(range: (f64, f64), count: usize) -> Self {
        PointScale { range, count }
    }

    pub fn at(&self, i: usize) -> f64 {
        if self.count <= 1 {
            return (self.range.0 + self.range.1) / 2.0;
        }
        let t = i as f64 / (self.count - 1) as f64;
        self.range.0 + t * (self.range.1 - self.range.0)
    }

    pub fn step(&self) -> f64 {
        if self.count <= 1 {
            return 0.0;
        }
        (self.range.1 - self.range.0) / (self.count - 1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare tick outputs with d3-array reference values.
    #[test]
    fn ticks_match_d3() {
        assert_eq!(
            ticks(0.0, 100.0, 5),
            vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );
        assert_eq!(ticks(1.0, 10.0, 5), vec![2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(ticks(-1.0, 1.0, 5), vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        // Generate fractional ticks by division instead of repeated decimal multiplication.
        assert_eq!(ticks(0.0, 1.0, 5), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        // Keep the final tick within the domain's upper bound.
        let t = ticks(0.0, 0.95, 10);
        assert_eq!(t.len(), 10);
        assert_eq!(t[0], 0.0);
        assert!((t[9] - 0.9).abs() < 1e-12);
        // Reverse ticks for a reversed domain.
        assert_eq!(
            ticks(100.0, 0.0, 5),
            vec![100.0, 80.0, 60.0, 40.0, 20.0, 0.0]
        );
    }

    #[test]
    fn negative_step_dodges_float_noise() {
        // Reciprocal steps avoid decimal accumulation error.
        let t = ticks(0.0, 1.0, 10);
        assert_eq!(t.len(), 11);
        assert_eq!(t[3], 0.3, "0.3 must remain exact, not 0.30000000000000004");
        assert_eq!(t[7], 0.7);
    }

    #[test]
    fn nice_matches_d3() {
        assert_eq!(nice(0.1, 0.9, 5), (0.0, 1.0));
        assert_eq!(nice(1.0, 9.0, 5), (0.0, 10.0));
        assert_eq!(
            nice(0.0, 100.0, 5),
            (0.0, 100.0),
            "already rounded bounds must remain unchanged"
        );
        assert_eq!(nice(3.0, 97.0, 5), (0.0, 100.0));
        assert_eq!(nice(-0.3, 0.7, 5), (-0.4, 0.8));
        // Preserve degenerate domains; extent_or_default handles expansion.
        assert_eq!(nice(5.0, 5.0, 5), (5.0, 5.0));
    }

    #[test]
    fn degenerate_extents_expand_toward_zero() {
        // Expand equal values toward zero rather than creating a misleading narrow domain.
        assert_eq!(extent_or_default(5.0, 5.0), (0.0, 5.0));
        assert_eq!(extent_or_default(-5.0, -5.0), (-5.0, 0.0));
        assert_eq!(extent_or_default(0.0, 0.0), (0.0, 1.0));
        assert_eq!(extent_or_default(f64::NAN, 1.0), (0.0, 1.0));
        assert_eq!(extent_or_default(1.0, 9.0), (1.0, 9.0));
    }

    #[test]
    fn extent_ignores_non_finite_values_instead_of_poisoning_the_domain() {
        assert_eq!(extent(&[3.0, 1.0, 4.0, 1.5]), Some((1.0, 4.0)));
        assert_eq!(extent(&[f64::NAN, 2.0, f64::INFINITY]), Some((2.0, 2.0)));
        assert_eq!(extent(&[]), None);
        assert_eq!(extent(&[f64::NAN]), None);
    }

    #[test]
    fn linear_maps_and_inverts() {
        let s = LinearScale::new((0.0, 100.0), (0.0, 200.0));
        assert_eq!(s.map(50.0), 100.0);
        assert_eq!(s.invert(100.0), 50.0);
        // Reversing output endpoints naturally inverts the y axis.
        let y = LinearScale::new((0.0, 10.0), (300.0, 100.0));
        assert_eq!(y.map(0.0), 300.0);
        assert_eq!(y.map(10.0), 100.0);
        assert_eq!(y.map(5.0), 200.0);
    }

    #[test]
    fn linear_never_emits_nan_on_a_degenerate_domain() {
        let s = LinearScale::new((7.0, 7.0), (0.0, 100.0));
        assert_eq!(s.map(7.0), 50.0);
        assert!(s.map(999.0).is_finite());
    }

    #[test]
    fn band_tiles_the_range() {
        let b = BandScale::new((0.0, 300.0), 3);
        assert_eq!(b.step(), 100.0);
        assert_eq!(
            b.bandwidth(),
            100.0,
            "bands tile tightly by default; geometry controls spacing"
        );
        assert_eq!(b.band(0), (0.0, 100.0));
        assert_eq!(b.band(2), (200.0, 300.0));
        assert_eq!(b.center(1), 150.0);
    }

    #[test]
    fn band_padding_follows_d3() {
        let b = BandScale {
            padding_inner: 0.2,
            padding_outer: 0.1,
            ..BandScale::new((0.0, 100.0), 4)
        };
        // step = 100 / (4 - 0.2 + 0.2) = 25
        assert!((b.step() - 25.0).abs() < 1e-9);
        assert!((b.bandwidth() - 20.0).abs() < 1e-9);
        // Apply equal outer padding at both ends.
        assert!((b.band(0).0 - 2.5).abs() < 1e-9);
    }

    #[test]
    fn point_puts_ends_on_the_edges() {
        let p = PointScale::new((0.0, 300.0), 4);
        assert_eq!(p.at(0), 0.0);
        assert_eq!(p.at(3), 300.0);
        assert_eq!(p.step(), 100.0);
        // Center a single point.
        let one = PointScale::new((0.0, 300.0), 1);
        assert_eq!(one.at(0), 150.0);
        assert_eq!(one.step(), 0.0);
    }
}
