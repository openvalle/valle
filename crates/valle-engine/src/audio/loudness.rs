//! Loudness as ITU-R BS.1770-4 and EBU R128 measure it: K-weighted mean square in 400 ms blocks
//! (75 % overlap), gated at -70 LUFS and then 10 LU under the gated mean (integrated); momentary
//! (400 ms) and short-term (3 s) maxima; loudness range (EBU Tech 3342: short-term values gated at
//! -70 LUFS and 20 LU under, 10th to 95th percentile); sample and true peak (4× oversampled).

use super::dsp::{Biquad, Coefficients, TruePeak};
use super::{RATE, decibels};

/// BS.1770-4's K-weighting at 48 kHz: a high shelf for the head, then the RLB high-pass.
const K: [Coefficients; 2] = [
    Coefficients {
        b: [1.53512485958697, -2.69169618940638, 1.19839281085285],
        a: [-1.69065929318241, 0.73248077421585],
    },
    Coefficients {
        b: [1.0, -2.0, 1.0],
        a: [-1.99004745483398, 0.99007225036621],
    },
];

/// 100 ms, the blocks' hop.
const SUB: usize = RATE as usize / 10;

pub struct Meter {
    filters: [Biquad; 2],
    sum: f64,
    n: usize,
    /// Per 100 ms: the channels' mean squares, summed (left and right weigh 1).
    subs: Vec<f64>,
    peak: TruePeak,
    true_peak: f64,
    sample_peak: f64,
}

impl Default for Meter {
    fn default() -> Self {
        Meter {
            filters: [Biquad::default(); 2],
            sum: 0.0,
            n: 0,
            subs: Vec::new(),
            peak: TruePeak::default(),
            true_peak: 0.0,
            sample_peak: 0.0,
        }
    }
}

/// What a meter measured, LUFS, LU and dBFS (dBTP); -inf where nothing passed the gates.
#[derive(Clone, Copy, Debug)]
pub struct Loudness {
    pub integrated: f64,
    pub momentary_max: f64,
    pub short_term_max: f64,
    pub range: f64,
    pub true_peak: f64,
    pub sample_peak: f64,
}

fn lufs(z: f64) -> f64 {
    -0.691 + 10.0 * z.log10()
}

fn mean(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| s / n as f64)
}

impl Meter {
    /// Interleaved stereo samples.
    pub fn push(&mut self, samples: &[f32]) {
        for s in samples.as_chunks::<2>().0 {
            let x = [f64::from(s[0]), f64::from(s[1])];
            self.sample_peak = self.sample_peak.max(x[0].abs()).max(x[1].abs());
            self.true_peak = self.true_peak.max(self.peak.push(x));
            let k = self.filters[0].run(&K[0], x);
            let y = self.filters[1].run(&K[1], k);
            self.sum += y[0] * y[0] + y[1] * y[1];
            self.n += 1;
            if self.n == SUB {
                self.subs.push(self.sum / SUB as f64);
                self.sum = 0.0;
                self.n = 0;
            }
        }
    }

    /// Mean squares of the windows `subs` long (100 ms each), every 100 ms.
    fn windows(&self, subs: usize) -> Vec<f64> {
        if self.subs.len() < subs {
            return Vec::new();
        }
        (0..=self.subs.len() - subs)
            .map(|j| self.subs[j..j + subs].iter().sum::<f64>() / subs as f64)
            .collect()
    }

    /// Short-term loudness (3 s windows) every 100 ms, the first ending at 3 s.
    pub fn short_term(&self) -> Vec<f64> {
        self.windows(30).into_iter().map(lufs).collect()
    }

    pub fn result(&self) -> Loudness {
        let blocks = self.windows(4);
        let gated = |z: &[f64], relative: f64| -> Option<f64> {
            let absolute: Vec<f64> = z.iter().copied().filter(|&z| lufs(z) > -70.0).collect();
            let gate = lufs(mean(absolute.iter().copied())?) - relative;
            mean(absolute.into_iter().filter(|&z| lufs(z) > gate))
        };
        let integrated = gated(&blocks, 10.0).map_or(f64::NEG_INFINITY, lufs);
        let short = self.windows(30);
        let range = {
            let absolute: Vec<f64> = short.iter().copied().filter(|&z| lufs(z) > -70.0).collect();
            match mean(absolute.iter().copied()) {
                None => 0.0,
                Some(m) => {
                    let gate = lufs(m) - 20.0;
                    let mut l: Vec<f64> = absolute
                        .into_iter()
                        .map(lufs)
                        .filter(|&l| l > gate)
                        .collect();
                    l.sort_by(f64::total_cmp);
                    let at = |p: f64| {
                        let x = p * (l.len() - 1) as f64;
                        let (i, f) = (x.floor() as usize, x.fract());
                        l[i] + (l[(i + 1).min(l.len() - 1)] - l[i]) * f
                    };
                    if l.is_empty() {
                        0.0
                    } else {
                        at(0.95) - at(0.10)
                    }
                }
            }
        };
        let max = |z: &[f64]| {
            z.iter()
                .copied()
                .map(lufs)
                .fold(f64::NEG_INFINITY, f64::max)
        };
        Loudness {
            integrated,
            momentary_max: max(&blocks),
            short_term_max: max(&short),
            range,
            true_peak: decibels(self.true_peak),
            sample_peak: decibels(self.sample_peak),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stereo 997 Hz sine, `db` dBFS peak on both channels, `seconds` long.
    fn sine(db: f64, seconds: f64) -> Vec<f32> {
        let a = super::super::amplitude(db);
        let n = (seconds * RATE as f64) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (a * (std::f64::consts::TAU * 997.0 * i as f64 / RATE as f64).sin()) as f32;
                [v, v]
            })
            .collect()
    }

    fn measure(parts: &[(f64, f64)]) -> Loudness {
        let mut m = Meter::default();
        for &(db, s) in parts {
            m.push(&sine(db, s));
        }
        m.result()
    }

    /// EBU Tech 3341 cases 1-5 (stereo sines at the levels it gives, -23 LUFS integrated but
    /// case 2's -33), within its ±0.1 LU.
    #[test]
    fn ebu_3341() {
        let cases: [(&[(f64, f64)], f64); 5] = [
            (&[(-23.0, 20.0)], -23.0),
            (&[(-33.0, 20.0)], -33.0),
            (&[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)], -23.0),
            (
                &[
                    (-72.0, 10.0),
                    (-36.0, 10.0),
                    (-23.0, 60.0),
                    (-36.0, 10.0),
                    (-72.0, 10.0),
                ],
                -23.0,
            ),
            (&[(-26.0, 20.0), (-20.0, 20.1), (-26.0, 20.0)], -23.0),
        ];
        for (i, (parts, want)) in cases.iter().enumerate() {
            let got = measure(parts).integrated;
            assert!(
                (got - want).abs() <= 0.1,
                "case {}: {got} LUFS, {want} wanted",
                i + 1
            );
        }
    }

    /// A 997 Hz sine at -20 dBFS: momentary and short-term equal the integrated, peaks -20.
    #[test]
    fn steady_sine() {
        let l = measure(&[(-20.0, 10.0)]);
        assert!((l.momentary_max - l.integrated).abs() < 0.05);
        assert!((l.short_term_max - l.integrated).abs() < 0.05);
        assert!(l.range < 0.1);
        assert!((l.sample_peak + 20.0).abs() < 0.01, "{}", l.sample_peak);
        assert!((l.true_peak + 20.0).abs() < 0.05, "{}", l.true_peak);
    }
}
