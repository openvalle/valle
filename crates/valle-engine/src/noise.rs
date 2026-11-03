//! Seeded noise from integer hashing (SplitMix64), so every host makes the same values.

/// SplitMix64's output function.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// `count` values in [low, high), SplitMix64's sequence from `seed`.
pub fn random(seed: u64, count: usize, [low, high]: [f64; 2]) -> Vec<f64> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state.wrapping_add(GOLDEN);
            low + unit(mix(state)) * (high - low)
        })
        .collect()
}

/// The top 53 bits as a number in [0, 1).
fn unit(bits: u64) -> f64 {
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// A value in [0, 1) for each seed, stream and lattice point.
fn lattice(seed: u64, stream: u64, cell: i64) -> f64 {
    let z = mix(mix(seed.wrapping_add(GOLDEN)) ^ stream.wrapping_mul(GOLDEN));
    unit(mix(z ^ (cell as u64).wrapping_mul(GOLDEN)))
}

/// Smooth noise in [0, 1]: the lattice values joined by a uniform cubic B-spline, whose curve and
/// its slope and bend are continuous, so a value moving along it never stops at a lattice point.
pub fn noise1(seed: u64, stream: u64, x: f64) -> f64 {
    if !x.is_finite() {
        return 0.5;
    }
    let i = x.floor();
    let (t, i) = (x - i, i as i64);
    let p = |k: i64| lattice(seed, stream, i.wrapping_add(k));
    let (t2, t3) = (t * t, t * t * t);
    let w = [
        (1.0 - t) * (1.0 - t) * (1.0 - t),
        3.0 * t3 - 6.0 * t2 + 4.0,
        -3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0,
        t3,
    ];
    (w[0] * p(-1) + w[1] * p(0) + w[2] * p(1) + w[3] * p(2)) / 6.0
}

/// `wiggle()`, one axis of it: a value within ±amplitude that drifts `frequency` times a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wiggle {
    seed: u64,
    frequency: f64,
    amplitude: f64,
    phase: f64,
    axis: u64,
}

impl Wiggle {
    /// From `seed|frequency|amplitude|phase|axis`.
    pub fn parse(spec: &str) -> Result<Wiggle, String> {
        let bad = || format!("wiggle {spec}");
        let [seed, frequency, amplitude, phase, axis] = spec.split('|').collect::<Vec<_>>()[..]
        else {
            return Err(bad());
        };
        let number = |s: &str| {
            s.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(bad)
        };
        let frequency = number(frequency)?;
        if frequency <= 0.0 {
            return Err(bad());
        }
        Ok(Wiggle {
            seed: seed.parse().map_err(|_| bad())?,
            frequency,
            amplitude: number(amplitude)?,
            phase: number(phase)?,
            axis: axis.parse().map_err(|_| bad())?,
        })
    }

    pub fn at(&self, t: f64) -> f64 {
        if t.is_nan() {
            return t;
        }
        let n = noise1(self.seed, self.axis, t * self.frequency + self.phase);
        (2.0 * n - 1.0) * self.amplitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_smooth_and_within_its_range() {
        let (mut least, mut most, mut sum) = (1.0f64, 0.0f64, 0.0);
        let h = 1e-4;
        for i in -2000..2000 {
            let x = f64::from(i) * 0.01 + 0.005;
            let n = noise1(7, 0, x);
            (least, most, sum) = (least.min(n), most.max(n), sum + n);
            let slope = |x: f64| (noise1(7, 0, x + h) - noise1(7, 0, x - h)) / (2.0 * h);
            // No jump in the value or its slope across a lattice point.
            let k = f64::from(i / 100);
            assert!((noise1(7, 0, k - 1e-9) - noise1(7, 0, k + 1e-9)).abs() < 1e-8);
            assert!((slope(k - 1e-3) - slope(k + 1e-3)).abs() < 1e-2);
        }
        assert!(
            least >= 0.0 && most <= 1.0 && most - least > 0.5,
            "{least} {most}"
        );
        assert!((sum / 4000.0 - 0.5).abs() < 0.1);
    }

    #[test]
    fn random_values_are_splitmix64_s() {
        // The first outputs of SplitMix64 from seed 0, as its reference implementation gives them.
        let first = [
            0xE220_A839_7B1D_CDAF_u64,
            0x6E78_9E6A_A1B9_65F4,
            0x06C4_5D18_8009_454F,
        ];
        let expected = first.map(unit);
        assert_eq!(random(0, 3, [0.0, 1.0]), expected);
        let scaled = random(9, 1000, [-2.0, 3.0]);
        assert!(scaled.iter().all(|&v| (-2.0..3.0).contains(&v)));
        assert!((scaled.iter().sum::<f64>() / 1000.0 - 0.5).abs() < 0.2);
        assert_ne!(random(1, 1, [0.0, 1.0]), random(2, 1, [0.0, 1.0]));
    }

    #[test]
    fn seeds_and_streams_differ() {
        let at = |seed, stream| noise1(seed, stream, 3.3);
        assert_eq!(at(1, 0), at(1, 0));
        assert_ne!(at(1, 0), at(2, 0));
        assert_ne!(at(1, 0), at(1, 1));
        assert_ne!(at(0, 0), at(0, 1));
    }

    #[test]
    fn wiggles_stay_within_their_amplitude() {
        let w = Wiggle::parse("3|2|10|0.5|1").unwrap();
        assert!((0..1000).all(|i| w.at(f64::from(i) / 100.0).abs() <= 10.0));
        assert_ne!(w.at(1.0), Wiggle::parse("3|2|10|0.5|0").unwrap().at(1.0));
        for bad in [
            "-1|1|1|0|0",
            "1|0|1|0|0",
            "1|1|1|0",
            "1.5|1|1|0|0",
            "1|1|x|0|0",
        ] {
            assert!(Wiggle::parse(bad).is_err(), "{bad}");
        }
    }
}
