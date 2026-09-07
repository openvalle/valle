//! DSP beat detection from mono f32 samples. Compute a detrended log-spectral-flux onset envelope,
//! estimate tempo by autocorrelation with a log-Gaussian prior, then track beats using dynamic
//! programming. Return both stable beat positions and denser onset events.

use std::f32::consts::PI;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// STFT window length in samples.
const WIN: usize = 1024;
/// STFT hop length in samples.
const HOP: usize = 256;
/// Log-spectral compression coefficient in log(1 + gamma * magnitude).
const COMPRESS: f32 = 100.0;
/// Local detrending window in seconds.
const DETREND_S: f64 = 0.5;
/// Onset threshold as a quantile of positive envelope values, robust to sparse and dense attacks.
const ONSET_GATE_PCT: f64 = 0.50;
/// Dynamic-programming timing regularization strength.
const TIGHTNESS: f32 = 100.0;
/// Tempo prior center in BPM and width in octaves.
const TEMPO_PRIOR_BPM: f64 = 120.0;
const TEMPO_PRIOR_OCTAVES: f64 = 0.9;

/// Onset time and detrended strength, comparable within one file.
#[derive(Debug, Clone, Copy)]
pub struct Onset {
    pub t: f64,
    pub strength: f32,
}

/// Detection result; empty beats and zero BPM indicate no stable beat grid.
#[derive(Debug, Clone, Default)]
pub struct Beat {
    /// Representative tempo derived from the median beat interval.
    pub bpm: f64,
    /// Normalized envelope autocorrelation at the beat period, in 0..1. This measures periodic
    /// support rather than onset alignment; consumers should choose their confidence threshold.
    pub support: f64,
    /// Beat positions in ascending seconds.
    pub beats: Vec<f64>,
    /// Onsets in ascending time order.
    pub onsets: Vec<Onset>,
}

/// Detect beats from mono samples within the requested BPM search range.
pub fn detect(samples: &[f32], rate: u32, min_bpm: f64, max_bpm: f64) -> Beat {
    if samples.len() < WIN * 4 || rate == 0 {
        return Beat::default();
    }
    let env = onset_envelope(samples);
    let fps = rate as f64 / HOP as f64;
    let d = detrend(&env, (DETREND_S * fps).round() as usize);
    let onsets = pick_onsets(&d, fps);
    let Some((period, support)) = tempo_period(&d, fps, min_bpm, max_bpm) else {
        return Beat {
            bpm: 0.0,
            support: 0.0,
            beats: Vec::new(),
            onsets,
        };
    };
    let beats_f = track_beats(&d, period);
    let to_t = |i: usize| (i * HOP + WIN / 2) as f64 / rate as f64;
    let beats: Vec<f64> = beats_f.iter().map(|&i| to_t(i)).collect();
    let bpm = if beats.len() >= 2 {
        let mut gaps: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(|a, b| a.total_cmp(b));
        60.0 / gaps[gaps.len() / 2]
    } else {
        0.0
    };
    Beat {
        bpm,
        support,
        beats,
        onsets,
    }
}

/// Log-compressed spectral-flux envelope with one frame per hop.
fn onset_envelope(samples: &[f32]) -> Vec<f32> {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(WIN);
    let hann: Vec<f32> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WIN as f32).cos())
        .collect();
    let frames = (samples.len() - WIN) / HOP + 1;
    let mut prev = vec![0f32; WIN / 2];
    let mut mag = vec![0f32; WIN / 2];
    let mut buf = vec![Complex::default(); WIN];
    let mut env = Vec::with_capacity(frames);
    for f in 0..frames {
        let s = &samples[f * HOP..f * HOP + WIN];
        for i in 0..WIN {
            buf[i] = Complex::new(s[i] * hann[i], 0.0);
        }
        fft.process(&mut buf);
        let mut flux = 0f32;
        for i in 0..WIN / 2 {
            mag[i] = (1.0 + COMPRESS * buf[i].norm()).ln();
            flux += (mag[i] - prev[i]).max(0.0);
        }
        std::mem::swap(&mut prev, &mut mag);
        env.push(if f == 0 { 0.0 } else { flux });
    }
    env
}

/// Subtract a moving local mean and clamp negative values to zero.
fn detrend(env: &[f32], half: usize) -> Vec<f32> {
    let n = env.len();
    let mut out = vec![0f32; n];
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        let mean = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        out[i] = (env[i] - mean).max(0.0);
    }
    out
}

/// Select local peaks within three frames that exceed the positive-envelope quantile threshold.
fn pick_onsets(d: &[f32], fps: f64) -> Vec<Onset> {
    let mut pos: Vec<f32> = d.iter().copied().filter(|v| *v > 0.0).collect();
    if pos.is_empty() {
        return Vec::new();
    }
    pos.sort_by(|a, b| a.total_cmp(b));
    let gate = pos[((pos.len() - 1) as f64 * ONSET_GATE_PCT) as usize];
    let mut out = Vec::new();
    let w = 3usize;
    for i in w..d.len().saturating_sub(w) {
        if d[i] >= gate && (i - w..i + w + 1).all(|j| d[j] <= d[i]) && d[i] > 0.0 {
            out.push(Onset {
                t: (i * HOP + WIN / 2) as f64 / (fps * HOP as f64),
                strength: d[i],
            });
        }
    }
    out
}

/// Estimate the beat period and normalized autocorrelation. A log-Gaussian prior reduces
/// half/double-tempo ambiguity.
fn tempo_period(d: &[f32], fps: f64, min_bpm: f64, max_bpm: f64) -> Option<(f64, f64)> {
    let lag_min = (fps * 60.0 / max_bpm.max(1.0)).floor().max(1.0) as usize;
    let lag_max = (fps * 60.0 / min_bpm.max(1.0)).ceil() as usize;
    if lag_max + 1 >= d.len() || lag_min >= lag_max {
        return None;
    }
    let mut best = (0f64, 0usize, 0f64);
    for lag in lag_min..=lag_max {
        let (mut ac, mut e0, mut e1) = (0f64, 0f64, 0f64);
        for i in 0..d.len() - lag {
            ac += (d[i] * d[i + lag]) as f64;
            e0 += (d[i] * d[i]) as f64;
            e1 += (d[i + lag] * d[i + lag]) as f64;
        }
        let norm = (e0 * e1).sqrt();
        if norm <= 0.0 {
            continue;
        }
        let bpm = fps * 60.0 / lag as f64;
        let w = (-0.5 * ((bpm / TEMPO_PRIOR_BPM).log2() / TEMPO_PRIOR_OCTAVES).powi(2)).exp();
        let score = ac * w;
        if score > best.0 {
            best = (score, lag, ac / norm);
        }
    }
    (best.0 > 0.0).then_some((best.1 as f64, best.2))
}

/// Track beats with a squared log-interval penalty around the estimated period; return ascending
/// frame indices.
fn track_beats(d: &[f32], period: f64) -> Vec<usize> {
    let n = d.len();
    let std = {
        let mean = d.iter().sum::<f32>() / n as f32;
        (d.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n as f32).sqrt()
    };
    if std <= 0.0 {
        return Vec::new();
    }
    let dn: Vec<f32> = d.iter().map(|v| v / std).collect();
    let lo_off = (period / 2.0).round() as usize;
    let hi_off = (period * 2.0).round() as usize;
    let mut score = vec![0f32; n];
    let mut back = vec![usize::MAX; n];
    for i in 0..n {
        let mut best = f32::MIN;
        let mut bp = usize::MAX;
        let lo = i.saturating_sub(hi_off);
        let hi = i.saturating_sub(lo_off);
        if lo < hi {
            for (p, &previous_score) in score.iter().enumerate().take(hi).skip(lo) {
                let delta = (i - p) as f32;
                let v = previous_score - TIGHTNESS * (delta / period as f32).ln().powi(2);
                if v > best {
                    best = v;
                    bp = p;
                }
            }
        }
        score[i] = dn[i]
            + if bp != usize::MAX && best > 0.0 {
                best
            } else {
                0.0
            };
        back[i] = if best > 0.0 { bp } else { usize::MAX };
    }
    // Backtrack from the highest-scoring endpoint within the final two periods.
    let tail = n.saturating_sub(hi_off);
    let Some(mut cur) = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b])) else {
        return Vec::new();
    };
    let mut beats = vec![cur];
    while back[cur] != usize::MAX {
        cur = back[cur];
        beats.push(cur);
    }
    beats.reverse();
    beats
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic click track using short, exponentially decaying 1 kHz bursts.
    fn click_track(bpm: f64, secs: f64, rate: u32) -> Vec<f32> {
        let n = (secs * rate as f64) as usize;
        let mut s = vec![0f32; n];
        let step = 60.0 / bpm;
        let mut t = 0.5; // Offset the first beat to test phase recovery.
        while t < secs {
            let start = (t * rate as f64) as usize;
            for i in 0..(rate as usize / 200).min(n - start) {
                let ph = 2.0 * PI * 1000.0 * i as f32 / rate as f32;
                s[start + i] += ph.sin() * (-(i as f32) / (rate as f32 * 0.002)).exp() * 0.8;
            }
            t += step;
        }
        s
    }

    fn assert_grid(bpm: f64) {
        let rate = 22_050;
        let doc = detect(&click_track(bpm, 20.0, rate), rate, 60.0, 200.0);
        assert!((doc.bpm - bpm).abs() < 2.0, "bpm {} ≠ {}", doc.bpm, bpm);
        assert!(doc.support > 0.8, "support {} is too low", doc.support);
        // Interior beats should align with a true click within 35 ms.
        let step = 60.0 / bpm;
        let mut hit = 0usize;
        let body: Vec<&f64> = doc.beats.iter().filter(|&&b| b > 1.0 && b < 19.0).collect();
        assert!(
            body.len() as f64 > 15.0 / step * 0.8,
            "too few beats: {}",
            body.len()
        );
        for &b in &body {
            let nearest = 0.5 + ((b - 0.5) / step).round() * step;
            if (b - nearest).abs() < 0.035 {
                hit += 1;
            }
        }
        assert!(
            hit as f64 >= body.len() as f64 * 0.9,
            "alignment {}/{} is below 90%",
            hit,
            body.len()
        );
    }

    #[test]
    fn click_120_bpm_tracked() {
        assert_grid(120.0);
    }

    #[test]
    fn click_87_bpm_tracked() {
        assert_grid(87.0);
    }

    #[test]
    fn silence_yields_nothing() {
        let doc = detect(&vec![0f32; 22_050 * 10], 22_050, 60.0, 200.0);
        assert_eq!(doc.bpm, 0.0);
        assert!(doc.beats.is_empty());
        assert!(doc.onsets.is_empty());
    }

    #[test]
    fn onsets_land_on_clicks() {
        let rate = 22_050;
        let doc = detect(&click_track(100.0, 12.0, rate), rate, 60.0, 200.0);
        assert!(!doc.onsets.is_empty());
        let step = 0.6;
        for o in &doc.onsets {
            let nearest = 0.5 + ((o.t - 0.5) / step).round() * step;
            assert!(
                (o.t - nearest).abs() < 0.035,
                "onset {} is too far from the click",
                o.t
            );
        }
    }
}
