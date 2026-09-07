//! Explicitly seeded pseudorandom values and continuous noise. SplitMix64 uses one u64 state, works
//! with a zero seed, and yields reproducible sequences across hosts through integer operations and
//! exact float conversion.

/// SplitMix64 state; sequence values depend on mutation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededRandom {
    state: u64,
}

impl SeededRandom {
    pub fn new(seed: u64) -> Self {
        SeededRandom { state: seed }
    }

    /// Return the next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in [0,1) using the high 53 bits divided by 2^53.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform value in [low, high); return low when the bounds are not strictly increasing.
    pub fn next_range(&mut self, low: f64, high: f64) -> f64 {
        if !matches!(low.partial_cmp(&high), Some(core::cmp::Ordering::Less)) {
            return low;
        }
        low + self.next_f64() * (high - low)
    }
}

/// Continuous one-dimensional value noise interpolating hashed lattice values with polynomial
/// smoothstep.
pub fn value_noise_1d(seed: u64, x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let i = x.floor();
    let t = x - i;
    // Use wrapping lattice arithmetic because large floating-point coordinates saturate when
    // converted to i64. Hashing wrapped coordinates is deterministic and avoids debug overflow
    // across the JS callback boundary.
    let a = hashed_unit(seed, i as i64);
    let b = hashed_unit(seed, (i as i64).wrapping_add(1));
    let smooth = t * t * (3.0 - 2.0 * t);
    a + (b - a) * smooth
}

/// Two-dimensional value noise with smoothstep interpolation across four corners.
pub fn value_noise_2d(seed: u64, x: f64, y: f64) -> f64 {
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let (ix, iy) = (x.floor(), y.floor());
    let (tx, ty) = (x - ix, y - iy);
    let (ix, iy) = (ix as i64, iy as i64);
    // Use wrapping lattice arithmetic for large coordinates.
    let corner = |dx: i64, dy: i64| {
        let cell = ix
            .wrapping_add(dx)
            .wrapping_add(iy.wrapping_add(dy).wrapping_mul(0x1_0000_0001));
        hashed_unit(seed, cell)
    };
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * sx;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// Stateless hash of seed and lattice coordinate into [0,1).
fn hashed_unit(seed: u64, cell: i64) -> f64 {
    let mut rng = SeededRandom::new(seed ^ (cell as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    rng.next_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Large coordinates must not panic during lattice arithmetic, including debug builds and host
    /// callbacks.
    #[test]
    fn lattice_coordinates_near_the_integer_limit_do_not_overflow() {
        for x in [9.3e18_f64, -9.3e18, 1e300, -1e300] {
            let value = value_noise_1d(1, x);
            assert!((0.0..=1.0).contains(&value), "1d at {x} → {value}");
        }
        for (x, y) in [(1e18_f64, 4e18_f64), (-9.3e18, 9.3e18), (1e300, -1e300)] {
            let value = value_noise_2d(1, x, y);
            assert!((0.0..=1.0).contains(&value), "2d at ({x}, {y}) → {value}");
        }
    }

    #[test]
    fn the_same_seed_always_gives_the_same_sequence() {
        // Equal seeds must reproduce identical sequences.
        let take = |seed| {
            let mut rng = SeededRandom::new(seed);
            (0..8).map(|_| rng.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(take(42), take(42));
        assert_ne!(take(42), take(43));
    }

    #[test]
    fn a_zero_seed_is_as_good_as_any_other() {
        // A zero seed must not collapse into a constant-zero sequence.
        let mut rng = SeededRandom::new(0);
        let values: Vec<f64> = (0..16).map(|_| rng.next_f64()).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        assert!(
            values.windows(2).any(|w| (w[0] - w[1]).abs() > 0.05),
            "a zero seed must not degenerate: {values:?}"
        );
    }

    #[test]
    fn uniform_values_stay_in_the_half_open_unit_interval() {
        let mut rng = SeededRandom::new(7);
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for _ in 0..20_000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v), "{v} escaped [0, 1)");
            min = min.min(v);
            max = max.max(v);
        }
        // Samples should cover the interval broadly enough to detect bit-extraction errors.
        assert!(min < 0.01 && max > 0.99, "poor coverage: {min}..{max}");
    }

    #[test]
    fn an_empty_range_returns_its_endpoint_instead_of_swapping_the_bounds() {
        // Reversed bounds are not silently swapped.
        let mut rng = SeededRandom::new(1);
        assert_eq!(rng.next_range(5.0, 5.0), 5.0);
        assert_eq!(rng.next_range(9.0, 2.0), 9.0);
        assert_eq!(rng.next_range(f64::NAN, 1.0).to_bits(), f64::NAN.to_bits());
    }

    #[test]
    fn value_noise_is_continuous_unlike_uniform_random() {
        // Nearby noise inputs should produce nearby outputs.
        let mut biggest_jump = 0.0f64;
        let mut previous = value_noise_1d(3, 0.0);
        for step in 1..2000 {
            let current = value_noise_1d(3, f64::from(step) * 0.01);
            biggest_jump = biggest_jump.max((current - previous).abs());
            previous = current;
        }
        assert!(
            biggest_jump < 0.05,
            "value noise must be continuous, biggest step was {biggest_jump}"
        );
    }

    #[test]
    fn value_noise_hits_its_lattice_points_exactly() {
        // At integer coordinates, interpolation must return the exact hashed lattice value.
        for cell in -3..3 {
            let at_lattice = value_noise_1d(11, f64::from(cell));
            let just_after = value_noise_1d(11, f64::from(cell) + 1e-9);
            assert!((at_lattice - just_after).abs() < 1e-6);
        }
    }

    #[test]
    fn two_dimensional_noise_varies_along_both_axes() {
        let a = value_noise_2d(5, 0.5, 0.5);
        let b = value_noise_2d(5, 3.5, 0.5);
        let c = value_noise_2d(5, 0.5, 3.5);
        assert!((a - b).abs() > 1e-6, "x must matter");
        assert!((a - c).abs() > 1e-6, "y must matter");
        assert!((0.0..=1.0).contains(&a));
    }

    #[test]
    fn non_finite_inputs_return_zero_rather_than_nan_geometry() {
        // Handle non-finite coordinates before producing invalid geometry.
        assert_eq!(value_noise_1d(1, f64::NAN), 0.0);
        assert_eq!(value_noise_2d(1, 1.0, f64::INFINITY), 0.0);
    }
}
