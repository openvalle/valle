//! The effects' signal processing, f64, one stereo sample at a time, and how long each remembers
//! (`settle`: samples for its state to fall 100 dB, the mixer's pre-roll).

use std::collections::VecDeque;

use super::{Band, BandKind, Compressor, Limiter, RATE, Reverb, amplitude};

const FS: f64 = RATE as f64;

/// A biquad's coefficients (normalized: a0 = 1), as the RBJ Audio EQ Cookbook gives them.
#[derive(Clone, Copy, Debug)]
pub struct Coefficients {
    pub b: [f64; 3],
    pub a: [f64; 2],
}

impl Coefficients {
    pub fn band(band: &Band) -> Coefficients {
        let w0 = std::f64::consts::TAU * band.frequency / FS;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * band.q);
        let a = 10f64.powf(band.gain_db / 40.0);
        let root = 2.0 * a.sqrt() * alpha;
        let (b, a0, a1, a2) = match band.kind {
            BandKind::Highpass => (
                [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandKind::Lowpass => (
                [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandKind::Peaking => (
                [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            BandKind::LowShelf => (
                [
                    a * ((a + 1.0) - (a - 1.0) * cos + root),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - root),
                ],
                (a + 1.0) + (a - 1.0) * cos + root,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - root,
            ),
            BandKind::HighShelf => (
                [
                    a * ((a + 1.0) + (a - 1.0) * cos + root),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - root),
                ],
                (a + 1.0) - (a - 1.0) * cos + root,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - root,
            ),
        };
        Coefficients {
            b: b.map(|x| x / a0),
            a: [a1 / a0, a2 / a0],
        }
    }

    /// The gain at `frequency` Hz, as the transfer function says.
    #[cfg(test)]
    pub fn response(&self, frequency: f64) -> f64 {
        let w = std::f64::consts::TAU * frequency / FS;
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            self.b[0] + self.b[1] * z1.0 + self.b[2] * z2.0,
            self.b[1] * z1.1 + self.b[2] * z2.1,
        );
        let den = (
            1.0 + self.a[0] * z1.0 + self.a[1] * z2.0,
            self.a[0] * z1.1 + self.a[1] * z2.1,
        );
        (num.0.hypot(num.1)) / (den.0.hypot(den.1))
    }

    /// Samples for its impulse response to fall 100 dB: from the larger pole's radius.
    fn settle(&self) -> f64 {
        let (p, q) = (self.a[0], self.a[1]);
        let disc = p * p - 4.0 * q;
        let r = if disc < 0.0 {
            q.sqrt()
        } else {
            ((-p + disc.sqrt()) / 2.0)
                .abs()
                .max(((-p - disc.sqrt()) / 2.0).abs())
        };
        if r <= 0.0 {
            return 2.0;
        }
        // High-Q responses ring up before they fall: half again.
        1.5 * 100.0 / (-20.0 * r.log10()).max(1e-9) + 2.0
    }
}

/// Transposed direct form II, a state per channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    z: [[f64; 2]; 2],
}

impl Biquad {
    pub fn run(&mut self, c: &Coefficients, x: [f64; 2]) -> [f64; 2] {
        let mut y = [0.0; 2];
        for ch in 0..2 {
            let z = &mut self.z[ch];
            y[ch] = c.b[0] * x[ch] + z[0];
            z[0] = c.b[1] * x[ch] - c.a[0] * y[ch] + z[1];
            z[1] = c.b[2] * x[ch] - c.a[1] * y[ch];
        }
        y
    }
}

pub struct Eq {
    bands: Vec<(Coefficients, Biquad)>,
}

impl Eq {
    pub fn new(bands: &[Band]) -> Eq {
        Eq {
            bands: bands
                .iter()
                .map(|b| (Coefficients::band(b), Biquad::default()))
                .collect(),
        }
    }

    pub fn run(&mut self, mut x: [f64; 2]) -> [f64; 2] {
        for (c, f) in &mut self.bands {
            x = f.run(c, x);
        }
        x
    }

    pub fn settle(&self) -> f64 {
        self.bands.iter().map(|(c, _)| c.settle()).sum()
    }
}

/// A compressor's gain computer and smoothed gain reduction (Giannoulis, Massberg and Reiss 2012:
/// log domain, smooth branching peak detector).
pub struct Comp {
    p: Compressor,
    attack: f64,
    release: f64,
    /// The gain reduction, dB (≥ 0).
    pub reduction: f64,
}

impl Comp {
    pub fn new(p: &Compressor) -> Comp {
        let coefficient = |seconds: f64| (-1.0 / (seconds.max(1e-6) * FS)).exp();
        Comp {
            p: *p,
            attack: coefficient(p.attack),
            release: coefficient(p.release),
            reduction: 0.0,
        }
    }

    /// The level out at a level in, dB, before smoothing.
    pub fn curve(p: &Compressor, x: f64) -> f64 {
        let (t, r, w) = (p.threshold_db, p.ratio, p.knee_db);
        let over = x - t;
        if 2.0 * over < -w {
            x
        } else if 2.0 * over <= w && w > 0.0 {
            x + (1.0 / r - 1.0) * (over + w / 2.0).powi(2) / (2.0 * w)
        } else {
            t + over / r
        }
    }

    /// `x` compressed by what `key` (`x` itself without a sidechain) says.
    pub fn run(&mut self, x: [f64; 2], key: [f64; 2]) -> [f64; 2] {
        let level = key[0].abs().max(key[1].abs()).max(1e-10);
        let db = 20.0 * level.log10();
        let mut wanted = db - Self::curve(&self.p, db);
        if let Some(range) = self.p.range_db {
            wanted = wanted.min(range);
        }
        let a = if wanted > self.reduction {
            self.attack
        } else {
            self.release
        };
        self.reduction = a * self.reduction + (1.0 - a) * wanted;
        let g = amplitude(self.p.makeup_db - self.reduction);
        [x[0] * g, x[1] * g]
    }

    /// Two runs settle together as fast as the slower of attack and release contracts them:
    /// 100 dB of reduction apart to 1e-4 dB.
    pub fn settle(&self) -> f64 {
        let slow = self.p.attack.max(self.p.release).max(1e-6);
        (1e6f64).ln() * slow * FS + 1.0
    }
}

/// Freeverb's tunings at 44.1 kHz, scaled to the rate.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

struct Comb {
    buffer: Vec<f64>,
    at: usize,
    store: f64,
}

struct Allpass {
    buffer: Vec<f64>,
    at: usize,
}

pub struct Verb {
    p: Reverb,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    feedback: f64,
    damp: f64,
}

fn scaled(n: usize) -> usize {
    (n as f64 * FS / 44_100.0).round() as usize
}

impl Verb {
    pub fn new(p: &Reverb) -> Verb {
        let side = |spread: usize| {
            (
                COMBS
                    .iter()
                    .map(|&n| Comb {
                        buffer: vec![0.0; scaled(n + spread)],
                        at: 0,
                        store: 0.0,
                    })
                    .collect::<Vec<_>>(),
                ALLPASSES
                    .iter()
                    .map(|&n| Allpass {
                        buffer: vec![0.0; scaled(n + spread)],
                        at: 0,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(SPREAD);
        Verb {
            p: *p,
            combs: [cl, cr],
            allpasses: [al, ar],
            feedback: p.room * 0.28 + 0.7,
            damp: p.damping * 0.4,
        }
    }

    pub fn run(&mut self, x: [f64; 2]) -> [f64; 2] {
        // Freeverb's fixed input gain and wet/dry scaling.
        let input = (x[0] + x[1]) * 0.015;
        let mut out = [0.0; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for c in &mut self.combs[ch] {
                let y = c.buffer[c.at];
                c.store = y * (1.0 - self.damp) + c.store * self.damp;
                c.buffer[c.at] = input + c.store * self.feedback;
                c.at = (c.at + 1) % c.buffer.len();
                acc += y;
            }
            for a in &mut self.allpasses[ch] {
                let b = a.buffer[a.at];
                a.buffer[a.at] = acc + b * 0.5;
                a.at = (a.at + 1) % a.buffer.len();
                acc = b - acc;
            }
            *o = acc;
        }
        let wet = self.p.wet * 3.0;
        let (w1, w2) = (
            wet * (self.p.width / 2.0 + 0.5),
            wet * ((1.0 - self.p.width) / 2.0),
        );
        let dry = self.p.dry * 2.0;
        [
            out[0] * w1 + out[1] * w2 + x[0] * dry,
            out[1] * w1 + out[0] * w2 + x[1] * dry,
        ]
    }

    pub fn settle(&self) -> f64 {
        let loops = 100.0 / (-20.0 * self.feedback.log10()).max(1e-9);
        let comb = scaled(COMBS[7] + SPREAD) as f64;
        let allpass: f64 = ALLPASSES
            .iter()
            .map(|&n| 100.0 / 6.02 * scaled(n + SPREAD) as f64)
            .sum();
        loops * comb + allpass
    }
}

/// Half the taps of the true-peak interpolator, the input samples on each side of a gap.
pub const TAPS: usize = 8;
/// Positions between two samples it reads (4× oversampling, BS.1770-4 Annex 2).
const PHASES: usize = 4;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-17 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// A Kaiser-windowed sinc, `half` zero crossings each side.
fn windowed_sinc(u: f64, half: f64, beta: f64) -> f64 {
    if u.abs() >= half {
        return 0.0;
    }
    let s = if u == 0.0 {
        1.0
    } else {
        let p = std::f64::consts::PI * u;
        p.sin() / p
    };
    let r = u / half;
    s * bessel_i0(beta * (1.0 - r * r).sqrt()) / bessel_i0(beta)
}

/// True peak: the largest of a sample and the three points after it up to the next, from the
/// samples around them; it answers `TAPS` samples late.
pub struct TruePeak {
    taps: [[f64; 2 * TAPS]; PHASES - 1],
    history: [[f64; 2 * TAPS]; 2],
    at: usize,
}

impl Default for TruePeak {
    fn default() -> Self {
        let mut taps = [[0.0; 2 * TAPS]; PHASES - 1];
        for (k, row) in taps.iter_mut().enumerate() {
            let f = (k + 1) as f64 / PHASES as f64;
            // The point f past sample TAPS-1 of the window.
            for (i, t) in row.iter_mut().enumerate() {
                *t = windowed_sinc(f - (i as f64 - (TAPS as f64 - 1.0)), TAPS as f64, 7.0);
            }
        }
        TruePeak {
            taps,
            history: [[0.0; 2 * TAPS]; 2],
            at: 0,
        }
    }
}

impl TruePeak {
    /// Takes a sample; the peak at the sample `TAPS` before it (and on to the next).
    pub fn push(&mut self, x: [f64; 2]) -> f64 {
        let n = 2 * TAPS;
        for (h, v) in self.history.iter_mut().zip(x) {
            h[self.at] = v;
        }
        self.at = (self.at + 1) % n;
        let mut peak = 0.0f64;
        for ch in 0..2 {
            let h = &self.history[ch];
            // Oldest first: window index i is history[(at + i) % n].
            let sample = h[(self.at + TAPS - 1) % n];
            peak = peak.max(sample.abs());
            for row in &self.taps {
                let mut y = 0.0;
                for (i, t) in row.iter().enumerate() {
                    y += t * h[(self.at + i) % n];
                }
                peak = peak.max(y.abs());
            }
        }
        peak
    }
}

/// The look-ahead limiter: the gain each sample needs (ceiling over its true peak), its minimum over
/// the look-ahead, averaged over the look-ahead (so it ramps down before the peak and is at most
/// what the peak needs on it), then a release; the signal delayed to meet it.
pub struct Lim {
    ceiling: f64,
    release: f64,
    span: usize,
    peak: TruePeak,
    /// Needed gains by input index, rising (a sliding minimum).
    min: VecDeque<(i64, f64)>,
    box_values: VecDeque<f64>,
    box_sum: f64,
    env: f64,
    delay: VecDeque<[f64; 2]>,
    n: i64,
}

impl Lim {
    pub fn new(p: &Limiter) -> Lim {
        let span = ((p.lookahead * FS).round() as usize).max(1);
        Lim {
            ceiling: amplitude(p.ceiling_db),
            release: (-1.0 / (p.release.max(1e-6) * FS)).exp(),
            span,
            peak: TruePeak::default(),
            min: VecDeque::new(),
            box_values: std::iter::repeat_n(1.0, span + 1).collect(),
            box_sum: (span + 1) as f64,
            env: 1.0,
            delay: std::iter::repeat_n([0.0; 2], span + TAPS).collect(),
            n: 0,
        }
    }

    /// Samples it delays what it limits.
    pub fn latency(&self) -> usize {
        self.span + TAPS
    }

    pub fn run(&mut self, x: [f64; 2]) -> [f64; 2] {
        let j = self.n - TAPS as i64;
        self.n += 1;
        let tp = self.peak.push(x);
        let need = if tp > self.ceiling {
            self.ceiling / tp
        } else {
            1.0
        };
        while self.min.back().is_some_and(|b| b.1 >= need) {
            self.min.pop_back();
        }
        self.min.push_back((j, need));
        while self.min.front().is_some_and(|f| f.0 < j - self.span as i64) {
            self.min.pop_front();
        }
        let q = self.min.front().map_or(1.0, |f| f.1);
        self.box_sum += q - self.box_values.pop_front().unwrap_or(1.0);
        self.box_values.push_back(q);
        let g = self.box_sum / (self.span + 1) as f64;
        self.env = g.min(self.env + (1.0 - self.env) * (1.0 - self.release));
        self.delay.push_back(x);
        let y = self.delay.pop_front().unwrap_or([0.0; 2]);
        [y[0] * self.env, y[1] * self.env]
    }

    pub fn settle(&self) -> f64 {
        (2 * self.span + TAPS) as f64 + (1e6f64).ln() / (1.0 - self.release).max(1e-12)
    }
}

/// The resampling kernel: a Kaiser-windowed sinc (β 9) tabulated at `STEPS` points per zero
/// crossing, `HALF` crossings each side.
pub const HALF: usize = 16;
const STEPS: usize = 256;

pub struct Kernel {
    table: Vec<f64>,
}

impl Default for Kernel {
    fn default() -> Self {
        Kernel {
            table: (0..=HALF * STEPS + 1)
                .map(|i| windowed_sinc(i as f64 / STEPS as f64, HALF as f64, 9.0))
                .collect(),
        }
    }
}

impl Kernel {
    /// The kernel at `u` zero crossings from its centre.
    pub fn at(&self, u: f64) -> f64 {
        let x = u.abs() * STEPS as f64;
        let i = x as usize;
        if i >= HALF * STEPS {
            return 0.0;
        }
        let f = x - i as f64;
        self.table[i] * (1.0 - f) + self.table[i + 1] * f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_eq_kind_has_the_expected_gain_at_its_reference_frequencies() {
        let gain = 10f64.powf(6.0 / 20.0);
        for (kind, expected) in [
            (
                BandKind::Lowpass,
                [1.0, std::f64::consts::FRAC_1_SQRT_2, 0.0],
            ),
            (
                BandKind::Highpass,
                [0.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
            ),
            (BandKind::Peaking, [1.0, gain, 1.0]),
            (BandKind::LowShelf, [gain, gain.sqrt(), 1.0]),
            (BandKind::HighShelf, [1.0, gain.sqrt(), gain]),
        ] {
            let c = Coefficients::band(&Band {
                kind,
                frequency: 1200.0,
                q: std::f64::consts::FRAC_1_SQRT_2,
                gain_db: 6.0,
            });
            for (frequency, expected) in [0.0, 1200.0, FS / 2.0].into_iter().zip(expected) {
                assert!(
                    (c.response(frequency) - expected).abs() < 1e-10,
                    "{kind:?} at {frequency}"
                );
            }
        }
    }

    #[test]
    fn eq_impulses_do_not_leak_between_channels_and_decay_within_preroll() {
        for kind in [
            BandKind::Lowpass,
            BandKind::Highpass,
            BandKind::Peaking,
            BandKind::LowShelf,
            BandKind::HighShelf,
        ] {
            let c = Coefficients::band(&Band {
                kind,
                frequency: 80.0,
                q: 4.0,
                gain_db: -12.0,
            });
            let mut filter = Biquad::default();
            let first = filter.run(&c, [1.0, 0.0]);
            assert_eq!(first, [c.b[0], 0.0]);
            let mut last = [0.0; 2];
            for _ in 0..c.settle().ceil() as usize {
                last = filter.run(&c, [0.0; 2]);
                assert!(last[0].is_finite());
                assert_eq!(last[1], 0.0);
            }
            assert!(last[0].abs() < 1e-5, "{kind:?}: {last:?}");
            assert_eq!(Biquad::default().run(&c, [1.0, 0.0]), first);
        }
    }

    #[test]
    fn soft_knee_is_continuous_and_sidechain_reduction_respects_its_range() {
        let p = Compressor {
            threshold_db: -24.0,
            ratio: 4.0,
            knee_db: 8.0,
            attack: 0.001,
            release: 0.01,
            makeup_db: 0.0,
            range_db: Some(6.0),
            key: None,
        };
        assert_eq!(Comp::curve(&p, -28.0), -28.0);
        assert_eq!(Comp::curve(&p, -24.0), -24.75);
        assert_eq!(Comp::curve(&p, -20.0), -23.0);
        for edge in [-28.0, -20.0] {
            assert!((Comp::curve(&p, edge - 1e-6) - Comp::curve(&p, edge + 1e-6)).abs() < 3e-6);
        }
        let mut c = Comp::new(&p);
        let mut y = [0.0; 2];
        for _ in 0..4800 {
            y = c.run([0.5, -0.25], [1.0, 0.0]);
        }
        let gain = 10f64.powf(-6.0 / 20.0);
        assert!((y[0] - 0.5 * gain).abs() < 1e-9);
        assert_eq!(y[1], -y[0] / 2.0);
        for _ in 0..24000 {
            y = c.run([0.5, -0.25], [0.0; 2]);
        }
        assert!((y[0] - 0.5).abs() < 1e-9);
    }
}
