//! What a file holds, from its decoded sound and frames: loudness as EBU R128 and ITU-R BS.1770
//! measure it, where it is silent, and where its shots cut. The decoding is the caller's; this
//! takes the samples and differences it gives, so it runs without FFmpeg.

/// The rate sound is measured at: the K-weighting below is for it.
pub const RATE: u32 = 48_000;

/// The span silence is looked for in, in samples: 10 ms.
const WINDOW: usize = RATE as usize / 100;
/// The span loudness is gated in, in samples: 100 ms, four to a gating block.
const STEP: usize = RATE as usize / 10;

/// A file's sound measured once: what silence and loudness are found from, with any thresholds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound {
    /// The level of each 10 ms, in dBFS: its mean square over the channels, full scale 0.
    pub levels: Vec<f32>,
    /// The K-weighted mean square of each 100 ms, summed over the channels.
    pub steps: Vec<f64>,
    /// The highest true peak, as a linear sample value.
    pub peak: f64,
}

/// Measures sound fed a block at a time: interleaved samples at `RATE`, one or two channels.
pub struct Meter {
    channels: usize,
    filters: Vec<KWeight>,
    oversampler: Vec<TruePeak>,
    /// The sum of squares of the window and the step being filled, and how many frames are in.
    window: (f64, usize),
    step: (f64, usize),
    sound: Sound,
}

impl Meter {
    pub fn new(channels: u16) -> Meter {
        let channels = usize::from(channels.clamp(1, 2));
        Meter {
            channels,
            filters: (0..channels).map(|_| KWeight::default()).collect(),
            oversampler: (0..channels).map(|_| TruePeak::default()).collect(),
            window: (0.0, 0),
            step: (0.0, 0),
            sound: Sound::default(),
        }
    }

    pub fn feed(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let (mut square, mut weighted) = (0.0, 0.0);
            for (c, &s) in frame.iter().enumerate() {
                let s = if s.is_finite() { f64::from(s) } else { 0.0 };
                square += s * s;
                let k = self.filters[c].run(s);
                weighted += k * k;
                self.sound.peak = self.sound.peak.max(self.oversampler[c].run(s));
            }
            self.window.0 += square / self.channels as f64;
            self.window.1 += 1;
            if self.window.1 == WINDOW {
                self.sound
                    .levels
                    .push(decibels(self.window.0 / WINDOW as f64) as f32);
                self.window = (0.0, 0);
            }
            self.step.0 += weighted;
            self.step.1 += 1;
            if self.step.1 == STEP {
                self.sound.steps.push(self.step.0 / STEP as f64);
                self.step = (0.0, 0);
            }
        }
    }

    /// What was measured, a last part window or step counted as it is.
    pub fn finish(mut self) -> Sound {
        if self.window.1 > 0 {
            let level = decibels(self.window.0 / self.window.1 as f64);
            self.sound.levels.push(level as f32);
        }
        if self.step.1 > 0 {
            self.sound.steps.push(self.step.0 / self.step.1 as f64);
        }
        self.sound
    }
}

/// `10 log10` of a mean square, as dB; -200 for nothing.
fn decibels(square: f64) -> f64 {
    if square > 0.0 {
        10.0 * square.log10()
    } else {
        -200.0
    }
}

/// Loudness of a K-weighted mean square, in LUFS.
fn lufs(square: f64) -> f64 {
    -0.691 + decibels(square)
}

/// Integrated loudness in LUFS over 400 ms blocks a 100 ms step apart, gated at -70 LUFS and
/// then 10 LU under the loudness of what passed; none when nothing passes.
pub fn integrated(sound: &Sound) -> Option<f64> {
    let blocks: Vec<f64> = sound
        .steps
        .windows(4)
        .map(|w| w.iter().sum::<f64>() / 4.0)
        .collect();
    let loud: Vec<f64> = blocks.into_iter().filter(|&b| lufs(b) > -70.0).collect();
    if loud.is_empty() {
        return None;
    }
    let relative = lufs(loud.iter().sum::<f64>() / loud.len() as f64) - 10.0;
    let kept: Vec<f64> = loud.into_iter().filter(|&b| lufs(b) > relative).collect();
    match kept.is_empty() {
        true => None,
        false => Some(lufs(kept.iter().sum::<f64>() / kept.len() as f64)),
    }
}

/// The loudness of each whole second, and of the part left at the end, in LUFS; none for a
/// second under -70.
pub fn each_second(sound: &Sound) -> Vec<Option<f64>> {
    sound
        .steps
        .chunks(10)
        .map(|s| Some(lufs(s.iter().sum::<f64>() / s.len() as f64)).filter(|&l| l > -70.0))
        .collect()
}

/// The true peak in dBTP; none for silence.
pub fn true_peak(sound: &Sound) -> Option<f64> {
    (sound.peak > 0.0).then(|| 20.0 * sound.peak.log10())
}

/// A silent part of the sound, in seconds into it, with its level in dBFS: the mean square over
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct Silence {
    pub start: f64,
    pub end: f64,
    pub level: f64,
}

/// The parts of at least `min` seconds whose every 10 ms is under `below` dBFS.
pub fn silences(sound: &Sound, below: f64, min: f64) -> Vec<Silence> {
    let seconds = WINDOW as f64 / f64::from(RATE);
    let mut out = Vec::new();
    let mut run: Option<(usize, f64)> = None;
    let levels = sound.levels.iter().map(|&l| Some(f64::from(l)));
    for (i, level) in levels.chain([None]).enumerate() {
        match (level.filter(|&l| l < below), run) {
            (Some(l), Some((start, sum))) => run = Some((start, sum + 10f64.powf(l / 10.0))),
            (Some(l), None) => run = Some((i, 10f64.powf(l / 10.0))),
            (None, Some((start, sum))) => {
                if (i - start) as f64 * seconds >= min - 1e-9 {
                    out.push(Silence {
                        start: start as f64 * seconds,
                        end: i as f64 * seconds,
                        level: decibels(sum / (i - start) as f64),
                    });
                }
                run = None;
            }
            (None, None) => {}
        }
    }
    out
}

/// A cut between shots: when the first frame of the new shot starts, in seconds, and how much
/// it differs from the frame before, from 0 to 100.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub time: f64,
    pub difference: f64,
}

/// Frames that differ from the one before by at least 6 in 100, and by three times as much as
/// the two frames each side of them do on average, at least half a second after the last cut:
/// a fade or a pan changes each frame a little, a cut changes one frame a lot.
pub fn cuts(differences: &[(f64, f64)]) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for (i, &(time, d)) in differences.iter().enumerate() {
        let near: Vec<f64> = (i.saturating_sub(2)..(i + 3).min(differences.len()))
            .filter(|&j| j != i)
            .map(|j| differences[j].1)
            .collect();
        let around = near.iter().sum::<f64>() / near.len().max(1) as f64;
        if d < 6.0 || d < 3.0 * around {
            continue;
        }
        match out.last_mut() {
            Some(last) if time - last.time < 0.5 => {
                if d > last.difference {
                    *last = Cut {
                        time,
                        difference: d,
                    };
                }
            }
            _ => out.push(Cut {
                time,
                difference: d,
            }),
        }
    }
    out
}

/// The two-stage K-weighting filter of ITU-R BS.1770 at 48 kHz: a high shelf, then a high pass.
#[derive(Default)]
struct KWeight {
    stages: [[f64; 4]; 2],
}

impl KWeight {
    const COEFFICIENTS: [([f64; 3], [f64; 2]); 2] = [
        (
            [1.53512485958697, -2.69169618940638, 1.19839281085285],
            [-1.69065929318241, 0.73248077421585],
        ),
        ([1.0, -2.0, 1.0], [-1.99004745483398, 0.99007225036621]),
    ];

    fn run(&mut self, mut x: f64) -> f64 {
        for (state, (b, a)) in self.stages.iter_mut().zip(Self::COEFFICIENTS) {
            let [x1, x2, y1, y2] = *state;
            let y = b[0] * x + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
            *state = [x, x1, y, y1];
            x = y;
        }
        x
    }
}

/// Four times oversampling by the interpolation filter of ITU-R BS.1770 Annex 2, for the peak
/// between samples.
#[derive(Default)]
struct TruePeak {
    /// The last 12 samples, newest first.
    history: [f64; 12],
}

impl TruePeak {
    const PHASES: [[f64; 12]; 4] = [
        [
            0.0017089843750,
            0.0109863281250,
            -0.0196533203125,
            0.0332031250000,
            -0.0594482421875,
            0.1373291015625,
            0.9721679687500,
            -0.1022949218750,
            0.0476074218750,
            -0.0266113281250,
            0.0148925781250,
            -0.0083007812500,
        ],
        [
            -0.0291748046875,
            0.0292968750000,
            -0.0517578125000,
            0.0891113281250,
            -0.1665039062500,
            0.4650878906250,
            0.7797851562500,
            -0.2003173828125,
            0.1015625000000,
            -0.0582275390625,
            0.0330810546875,
            -0.0189208984375,
        ],
        [
            -0.0189208984375,
            0.0330810546875,
            -0.0582275390625,
            0.1015625000000,
            -0.2003173828125,
            0.7797851562500,
            0.4650878906250,
            -0.1665039062500,
            0.0891113281250,
            -0.0517578125000,
            0.0292968750000,
            -0.0291748046875,
        ],
        [
            -0.0083007812500,
            0.0148925781250,
            -0.0266113281250,
            0.0476074218750,
            -0.1022949218750,
            0.9721679687500,
            0.1373291015625,
            -0.0594482421875,
            0.0332031250000,
            -0.0196533203125,
            0.0109863281250,
            0.0017089843750,
        ],
    ];

    /// The highest of the four values between the sample before and `x`.
    fn run(&mut self, x: f64) -> f64 {
        self.history.rotate_right(1);
        self.history[0] = x;
        Self::PHASES
            .iter()
            .map(|p| {
                p.iter()
                    .zip(&self.history)
                    .map(|(c, s)| c * s)
                    .sum::<f64>()
                    .abs()
            })
            .fold(x.abs(), f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amplitude: f64, seconds: f64, phase: f64) -> Vec<f32> {
        (0..(seconds * f64::from(RATE)) as usize)
            .map(|i| {
                let t = i as f64 / f64::from(RATE);
                (amplitude * (2.0 * std::f64::consts::PI * hz * t + phase).sin()) as f32
            })
            .collect()
    }

    fn measure(channels: u16, samples: &[f32]) -> Sound {
        let mut meter = Meter::new(channels);
        // In uneven blocks, as a decoder gives them.
        for block in samples.chunks(channels as usize * 1001) {
            meter.feed(block);
        }
        meter.finish()
    }

    /// EBU Tech 3341's first case: a 1 kHz sine at -23 dBFS on both channels is -23 LUFS.
    #[test]
    fn a_stereo_sine_at_minus_23_is_minus_23_lufs() {
        let amplitude = 10f64.powf(-23.0 / 20.0);
        let mono = sine(1000.0, amplitude, 20.0, 0.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        let sound = measure(2, &stereo);
        let loudness = integrated(&sound).unwrap();
        assert!((loudness + 23.0).abs() < 0.1, "{loudness}");
        let seconds = each_second(&sound);
        assert_eq!(seconds.len(), 20);
        assert!(seconds.iter().all(|s| (s.unwrap() + 23.0).abs() < 0.1));
    }

    /// Tech 3341's gating case: 10 s at -36, 60 s at -23 and 10 s at -36 again measure -23, as
    /// the quiet parts fall under the relative gate.
    #[test]
    fn quiet_parts_fall_under_the_relative_gate() {
        let level = |db: f64| 10f64.powf(db / 20.0);
        let mut mono = sine(1000.0, level(-36.0), 10.0, 0.0);
        mono.extend(sine(1000.0, level(-23.0), 60.0, 0.0));
        mono.extend(sine(1000.0, level(-36.0), 10.0, 0.0));
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        let loudness = integrated(&measure(2, &stereo)).unwrap();
        assert!((loudness + 23.0).abs() < 0.1, "{loudness}");
    }

    #[test]
    fn silence_has_no_loudness_or_peak() {
        let sound = measure(1, &[0.0; 48_000]);
        assert_eq!(integrated(&sound), None);
        assert_eq!(true_peak(&sound), None);
        assert_eq!(each_second(&sound), [None]);
    }

    /// A sine at a quarter of the rate, sampled 45° off its crest, peaks at 0.707 in the samples
    /// and at 1 between them.
    #[test]
    fn the_true_peak_is_found_between_samples() {
        let samples = sine(12_000.0, 1.0, 1.0, std::f64::consts::FRAC_PI_4);
        let sampled = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!((sampled - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        let peak = true_peak(&measure(1, &samples)).unwrap();
        assert!(peak > -0.5 && peak < 0.5, "{peak}");
        // Each phase passes a constant through within 3%, as the ITU's table does.
        for phase in TruePeak::PHASES {
            assert!((phase.iter().sum::<f64>() - 1.0).abs() < 0.03);
        }
    }

    #[test]
    fn silences_are_runs_under_the_level_long_enough() {
        let level = |db: f64| 10f64.powf(db / 20.0);
        // 1 s of tone, 0.8 s at -50, 0.5 s of tone, 0.3 s at -50, 0.5 s of tone, then digital
        // silence to the end.
        let mut samples = sine(440.0, 0.5, 1.0, 0.0);
        samples.extend(sine(440.0, level(-47.0), 0.8, 0.0));
        samples.extend(sine(440.0, 0.5, 0.5, 0.0));
        samples.extend(sine(440.0, level(-47.0), 0.3, 0.0));
        samples.extend(sine(440.0, 0.5, 0.5, 0.0));
        samples.extend(vec![0.0; 24_000]);
        let found = silences(&measure(1, &samples), -40.0, 0.5);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!((found[0].start - 1.0).abs() < 0.011 && (found[0].end - 1.8).abs() < 0.011);
        assert!((found[0].level + 50.0).abs() < 0.2, "{}", found[0].level);
        assert!((found[1].start - 3.1).abs() < 0.011 && (found[1].end - 3.6).abs() < 1e-9);
        assert_eq!(found[1].level, -200.0);
        assert_eq!(silences(&measure(1, &samples), -40.0, 0.25).len(), 3);
    }

    #[test]
    fn cuts_are_one_frame_changes_not_gradual_ones() {
        let at = |i: usize| i as f64 / 25.0;
        let mut differences: Vec<(f64, f64)> = (1..250).map(|i| (at(i), 1.5)).collect();
        // A cut at 2 s; a fade over 1 s from 4 s, each frame changing by 8; a cut at 7 s and a
        // flash back a frame after it, which counts once; and a small change at 9 s.
        differences[49].1 = 30.0;
        for d in &mut differences[99..124] {
            d.1 = 8.0;
        }
        differences[174].1 = 25.0;
        differences[175].1 = 28.0;
        differences[224].1 = 5.0;
        let found = cuts(&differences);
        assert_eq!(
            found,
            [
                Cut {
                    time: 2.0,
                    difference: 30.0
                },
                Cut {
                    time: at(176),
                    difference: 28.0
                }
            ]
        );
    }
}
