//! The mix running: nodes (tracks, buses, the master) in an order where every node comes after
//! what feeds it and what keys it, a block at a time, each sample from the mix and the state alone.

use std::sync::Arc;

use super::dsp::{Comp, Eq, HALF, Kernel, Lim, Verb};
use super::{Effect, End, Key, Mix, RATE, Strip, Target};

const FS: f64 = RATE as f64;
/// Samples per block inside the mixer (any size gives the same samples).
const BLOCK: usize = 1024;
/// Gain reduction is kept per this many samples (100 ms), its largest.
const WINDOW: i64 = RATE as i64 / 10;

/// A source decoded: interleaved stereo at `RATE`.
#[derive(Clone)]
pub struct Pcm(pub Arc<[f32]>);

impl Pcm {
    fn frames(&self) -> i64 {
        (self.0.len() / 2) as i64
    }
}

enum Fx {
    Eq(Eq),
    Comp(Comp, Option<usize>),
    Verb(Box<Verb>),
    Lim(Box<Lim>),
}

/// A compressor's gain reduction over the program: its largest per 100 ms, dB.
pub struct GainReduction {
    pub owner: String,
    pub insert: usize,
    pub key: Option<String>,
    pub windows: Vec<f32>,
}

pub struct Mixer {
    mix: Mix,
    pcm: Vec<Pcm>,
    kernel: Kernel,
    /// Nodes by index: tracks, then buses, then the master.
    order: Vec<usize>,
    fx: Vec<Vec<Fx>>,
    input: Vec<Vec<[f64; 2]>>,
    output: Vec<Vec<[f64; 2]>>,
    latency: i64,
    preroll: i64,
    /// The next graph sample, and the next output sample (`latency` behind it).
    graph: i64,
    position: i64,
    reductions: Vec<GainReduction>,
    /// (node, insert) → `reductions` index.
    reduction_of: Vec<(usize, usize, usize)>,
}

impl Mixer {
    /// The mix over its decoded sources (`pcm` by `mix.sources`).
    pub fn new(mix: Mix, pcm: Vec<Pcm>) -> Result<Mixer, String> {
        if pcm.len() != mix.sources.len() {
            return Err("audio: a decoded source per source expected".into());
        }
        let tracks = mix.tracks.len();
        let buses = mix.buses.len();
        let master = tracks + buses;
        let node = |t: Target| match t {
            Target::Master => master,
            Target::Bus(b) => tracks + b,
        };
        let key_node = |k: Key| match k {
            Key::Track(t) => t,
            Key::Bus(b) => tracks + b,
        };
        // Edges: what feeds a node, and what keys its compressors.
        let mut feeds: Vec<Vec<usize>> = vec![Vec::new(); master + 1];
        for i in 0..master {
            let s = strip(&mix, i);
            feeds[node(s.output)].push(i);
            for send in &s.sends {
                feeds[tracks + send.to].push(i);
            }
        }
        for (i, fed) in feeds.iter_mut().enumerate() {
            for e in &strip(&mix, i).effects {
                if let Effect::Compressor(c) = e
                    && let Some(k) = c.key
                {
                    fed.push(key_node(k));
                }
                if let Effect::Limiter(_) = e
                    && i != master
                {
                    return Err(format!(
                        "{}: a limiter goes on the master only (its look-ahead delays what it limits)",
                        name(&mix, i)
                    ));
                }
            }
        }
        // Kahn's order, lowest index first.
        let mut waiting: Vec<usize> = feeds.iter().map(Vec::len).collect();
        let mut order = Vec::with_capacity(master + 1);
        let mut ready: std::collections::BTreeSet<usize> =
            (0..=master).filter(|&i| waiting[i] == 0).collect();
        while let Some(i) = ready.pop_first() {
            order.push(i);
            for (j, f) in feeds.iter().enumerate() {
                for &from in f {
                    if from == i {
                        waiting[j] -= 1;
                        if waiting[j] == 0 {
                            ready.insert(j);
                        }
                    }
                }
            }
        }
        if order.len() <= master {
            let stuck: Vec<String> = (0..=master)
                .filter(|i| !order.contains(i))
                .map(|i| name(&mix, i))
                .collect();
            return Err(format!(
                "audio: a loop through {} (outputs, sends and sidechains must not come back)",
                stuck.join(", ")
            ));
        }
        // Clips that run past their source's end, with `end: error`.
        for t in &mix.tracks {
            for c in &t.clips {
                let frames = pcm[c.source].frames();
                let seconds = frames as f64 / FS;
                let reach = match &c.time {
                    super::TimeMap::Linear { trim, rate } => {
                        trim + (c.end - c.start) as f64 / FS * rate
                    }
                    super::TimeMap::Points(p) => p.iter().map(|q| q.1).fold(0.0, f64::max),
                };
                if c.end_policy == End::Error
                    && reach > seconds + 1.0 / FS
                    && !(c.optional && frames == 0)
                {
                    return Err(format!(
                        "{}: plays its source to {reach:.3} s, which is {seconds:.3} s long (end: \"hold\" or \"loop\" goes past it)",
                        c.what
                    ));
                }
            }
        }
        let mut fx = Vec::new();
        let mut reductions = Vec::new();
        let mut reduction_of = Vec::new();
        let mut latency = 0;
        let mut settle = vec![0.0f64; master + 1];
        for (i, own) in settle.iter_mut().enumerate() {
            let mut v = Vec::new();
            for (k, e) in strip(&mix, i).effects.iter().enumerate() {
                let f = match e {
                    Effect::Eq(b) => Fx::Eq(Eq::new(b)),
                    Effect::Compressor(c) => {
                        reduction_of.push((i, k, reductions.len()));
                        reductions.push(GainReduction {
                            owner: name(&mix, i),
                            insert: k,
                            key: c.key.map(|k| name(&mix, key_node(k))),
                            windows: Vec::new(),
                        });
                        Fx::Comp(Comp::new(c), c.key.map(key_node))
                    }
                    Effect::Reverb(r) => Fx::Verb(Box::new(Verb::new(r))),
                    Effect::Limiter(l) => {
                        let lim = Lim::new(l);
                        latency += lim.latency() as i64;
                        Fx::Lim(Box::new(lim))
                    }
                };
                *own += match &f {
                    Fx::Eq(e) => e.settle(),
                    Fx::Comp(c, _) => c.settle(),
                    Fx::Verb(v) => v.settle(),
                    Fx::Lim(l) => l.settle(),
                };
                v.push(f);
            }
            fx.push(v);
        }
        // A node settles once what feeds it has, then itself.
        let mut total = vec![0.0f64; master + 1];
        for &i in &order {
            total[i] = settle[i] + feeds[i].iter().map(|&f| total[f]).fold(0.0, f64::max);
        }
        let preroll = total[master].ceil() as i64 + latency;
        Ok(Mixer {
            input: vec![vec![[0.0; 2]; BLOCK]; master + 1],
            output: vec![vec![[0.0; 2]; BLOCK]; master + 1],
            mix,
            pcm,
            kernel: Kernel::default(),
            order,
            fx,
            latency,
            preroll,
            graph: 0,
            position: -latency,
            reductions,
            reduction_of,
        })
    }

    /// Samples run before a sample that is not the first: what effects remember settles.
    pub fn preroll(&self) -> i64 {
        self.preroll
    }

    /// Samples the master's limiter looks ahead (everything is delayed to match).
    pub fn latency(&self) -> i64 {
        self.latency
    }

    pub fn length(&self) -> i64 {
        self.mix.length
    }

    pub fn reductions(&self) -> &[GainReduction] {
        &self.reductions
    }

    /// The same mix, its sources shared, ready to render from the start.
    pub fn fresh(&self) -> Mixer {
        Mixer::new(self.mix.clone(), self.pcm.clone()).expect("the same mix")
    }

    /// Ready to render from `sample`: from the start as the export does, else after pre-roll.
    pub fn seek(&mut self, sample: i64) {
        let fresh = self.fresh();
        let windows: Vec<Vec<f32>> = self
            .reductions
            .iter_mut()
            .map(|r| std::mem::take(&mut r.windows))
            .collect();
        *self = fresh;
        for (r, w) in self.reductions.iter_mut().zip(windows) {
            r.windows = w;
        }
        let from = (sample - self.preroll).max(0);
        self.graph = from;
        self.position = from - self.latency;
        self.skip_to(sample);
    }

    /// Runs the graph, its output dropped, until the next output sample is `sample`.
    fn skip_to(&mut self, sample: i64) {
        while self.position < sample {
            let n = ((sample - self.position) as usize).min(BLOCK);
            self.block(n);
            self.graph += n as i64;
            self.position += n as i64;
        }
    }

    /// The next `out.len() / 2` samples, interleaved stereo (from 0 when nothing was asked for).
    pub fn render(&mut self, out: &mut [f32]) {
        self.skip_to(0);
        for chunk in out.chunks_mut(2 * BLOCK) {
            let n = chunk.len() / 2;
            self.block(n);
            let master = self.mix.tracks.len() + self.mix.buses.len();
            for (o, s) in chunk
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(&self.output[master][..n])
            {
                o[0] = s[0] as f32;
                o[1] = s[1] as f32;
            }
            self.graph += n as i64;
            self.position += n as i64;
        }
    }

    fn block(&mut self, n: usize) {
        let tracks = self.mix.tracks.len();
        let master = tracks + self.mix.buses.len();
        let g0 = self.graph;
        for buf in &mut self.input {
            buf[..n].fill([0.0; 2]);
        }
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let mut buf = std::mem::take(&mut self.input[i]);
            if i < tracks {
                self.clips(i, g0, &mut buf[..n]);
            }
            // The master's fader comes first (what its limiter gets is what leaves), a channel's
            // after its inserts.
            if i == master {
                self.fader(i, g0, 0, &mut buf[..n]);
            }
            // Inserts; past the master's limiter its content is `latency` behind.
            let mut behind = 0;
            for k in 0..self.fx[i].len() {
                let (fx, output) = (&mut self.fx[i][k], &self.output);
                match fx {
                    Fx::Eq(e) => buf[..n].iter_mut().for_each(|x| *x = e.run(*x)),
                    Fx::Verb(v) => buf[..n].iter_mut().for_each(|x| *x = v.run(*x)),
                    Fx::Lim(l) => {
                        buf[..n].iter_mut().for_each(|x| *x = l.run(*x));
                        behind += l.latency() as i64;
                    }
                    Fx::Comp(c, key) => {
                        let r = self
                            .reduction_of
                            .iter()
                            .find(|q| q.0 == i && q.1 == k)
                            .map(|q| q.2)
                            .expect("a compressor's record");
                        let windows = &mut self.reductions[r].windows;
                        for (j, x) in buf[..n].iter_mut().enumerate() {
                            let heard = key.map_or(*x, |key| output[key][j]);
                            *x = c.run(*x, heard);
                            let s = g0 + j as i64 - behind;
                            if s >= 0 {
                                let w = (s / WINDOW) as usize;
                                if windows.len() <= w {
                                    windows.resize(w + 1, 0.0);
                                }
                                windows[w] = windows[w].max(c.reduction as f32);
                            }
                        }
                    }
                }
            }
            if i != master {
                self.fader(i, g0, behind, &mut buf[..n]);
            }
            let s = strip(&self.mix, i);
            if i != master {
                let to = match s.output {
                    Target::Master => master,
                    Target::Bus(b) => tracks + b,
                };
                for (j, x) in buf[..n].iter().enumerate() {
                    let d = &mut self.input[to][j];
                    d[0] += x[0];
                    d[1] += x[1];
                }
                for send in &s.sends {
                    for (j, x) in buf[..n].iter().enumerate() {
                        let g = send.gain.at((g0 + j as i64) as f64 / FS);
                        let d = &mut self.input[tracks + send.to][j];
                        d[0] += x[0] * g;
                        d[1] += x[1] * g;
                    }
                }
            }
            self.output[i][..n].copy_from_slice(&buf[..n]);
            self.input[i] = buf;
        }
    }

    /// Node `i`'s gain and pan over graph samples from `g0`, its content `behind` them.
    fn fader(&self, i: usize, g0: i64, behind: i64, buf: &mut [[f64; 2]]) {
        let s = strip(&self.mix, i);
        for (j, x) in buf.iter_mut().enumerate() {
            let t = (g0 + j as i64 - behind) as f64 / FS;
            let g = s.gain.at(t);
            let (l, r) = balance(s.pan.at(t));
            *x = [x[0] * g * l, x[1] * g * r];
        }
    }

    /// Track `t`'s clips over graph samples [g0, g0 + out.len()), added into `out`.
    fn clips(&self, t: usize, g0: i64, out: &mut [[f64; 2]]) {
        let g1 = g0 + out.len() as i64;
        for c in &self.mix.tracks[t].clips {
            let (lo, hi) = (c.start.max(g0), c.end.min(g1));
            let pcm = &self.pcm[c.source];
            for s in lo..hi {
                let x = (s - c.start) as f64 / FS;
                let (src, speed) = c.time.at(x);
                let v = self.read(pcm, src * FS, speed.abs(), c.end_policy);
                let g = c.gain.at(x) * fade(c, s);
                let (l, r) = balance(c.pan.at(x));
                let o = &mut out[(s - g0) as usize];
                o[0] += v[0] * g * l;
                o[1] += v[1] * g * r;
            }
        }
    }

    /// The source at frame position `pos` passing at `speed`: the sample itself on the grid at
    /// speed 1, else band-limited (the kernel widened by the speed going faster); silent where
    /// it holds still.
    fn read(&self, pcm: &Pcm, pos: f64, speed: f64, end: End) -> [f64; 2] {
        let frames = pcm.frames();
        let at = |i: i64| -> [f64; 2] {
            let i = match end {
                _ if (0..frames).contains(&i) => i,
                End::Loop if frames > 0 => i.rem_euclid(frames),
                End::Hold if i >= frames && frames > 0 => frames - 1,
                _ => return [0.0; 2],
            } as usize;
            [f64::from(pcm.0[2 * i]), f64::from(pcm.0[2 * i + 1])]
        };
        if speed < 1e-9 {
            return [0.0; 2];
        }
        let nearest = pos.round();
        if (speed - 1.0).abs() < 1e-12 && (pos - nearest).abs() < 1e-6 {
            return at(nearest as i64);
        }
        let fc = (1.0 / speed).min(1.0);
        let reach = HALF as f64 / fc;
        let (first, last) = (
            (pos - reach).floor() as i64 + 1,
            (pos + reach).floor() as i64,
        );
        let (mut y, mut w) = ([0.0; 2], 0.0);
        for i in first..=last {
            let k = self.kernel.at(fc * (pos - i as f64));
            let v = at(i);
            y[0] += v[0] * k;
            y[1] += v[1] * k;
            w += k;
        }
        if w.abs() < 1e-12 {
            return [0.0; 2];
        }
        [y[0] / w, y[1] / w]
    }
}

/// How far clip `c` has faded in or out at output sample `s`, as its `fades` say.
fn fade(c: &super::Clip, s: i64) -> f64 {
    let quarter = std::f64::consts::FRAC_PI_2;
    let [into, out] = c.fades;
    let mut g = 1.0;
    if into > 0 && s < c.start + into {
        g *= (quarter * (s - c.start) as f64 / into as f64).sin();
    }
    if out > 0 && s >= c.end - out {
        g *= (quarter * (c.end - s) as f64 / out as f64).sin();
    }
    g
}

/// Pan as the Timeline has it: the opposite channel attenuated linearly.
fn balance(pan: f64) -> (f64, f64) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

fn strip(mix: &Mix, i: usize) -> &Strip {
    let tracks = mix.tracks.len();
    if i < tracks {
        &mix.tracks[i].strip
    } else if i < tracks + mix.buses.len() {
        &mix.buses[i - tracks].strip
    } else {
        &mix.master
    }
}

fn name(mix: &Mix, i: usize) -> String {
    let tracks = mix.tracks.len();
    if i < tracks {
        mix.tracks[i].id.clone()
    } else if i < tracks + mix.buses.len() {
        mix.buses[i - tracks].id.clone()
    } else {
        "master".into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn noise(seconds: f64, seed: u64) -> Pcm {
        let mut s = seed;
        let n = (seconds * FS) as usize;
        let mut v = Vec::with_capacity(2 * n);
        for i in 0..n {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let env = if (i / 4800) % 3 == 0 { 0.8 } else { 0.05 };
            v.push(((s % 2000) as f64 / 1000.0 - 1.0) as f32 * env);
            v.push(((s / 7 % 2000) as f64 / 1000.0 - 1.0) as f32 * env);
        }
        Pcm(v.into())
    }

    fn strip(effects: Vec<Effect>, output: Target) -> Strip {
        Strip {
            effects,
            gain: Param::Const(1.0),
            pan: Param::Const(0.0),
            sends: Vec::new(),
            output,
        }
    }

    fn clip(source: usize, seconds: f64, time: TimeMap) -> Clip {
        Clip {
            source,
            start: 0,
            end: (seconds * FS) as i64,
            time,
            end_policy: End::Hold,
            gain: Param::Const(1.0),
            pan: Param::Const(0.0),
            optional: false,
            fades: [0, 0],
            what: "clip".into(),
        }
    }

    /// Two tracks (one keying the other's compressor, one sent to a reverb bus) and a limited
    /// master, 4 s.
    fn mix() -> Mix {
        let comp = Compressor {
            threshold_db: -30.0,
            ratio: 8.0,
            knee_db: 6.0,
            attack: 0.01,
            release: 0.3,
            makeup_db: 0.0,
            range_db: Some(10.0),
            key: Some(Key::Track(0)),
        };
        let mut voice = strip(
            vec![Effect::Eq(vec![Band {
                kind: BandKind::Highpass,
                frequency: 80.0,
                q: 0.707,
                gain_db: 0.0,
            }])],
            Target::Master,
        );
        voice.sends.push(Send {
            to: 0,
            gain: Param::Keys(vec![(0.0, 0.0, Easing::Linear), (2.0, 0.5, Easing::Linear)]),
        });
        Mix {
            sources: vec!["a".into(), "b".into()],
            tracks: vec![
                Track {
                    id: "voice".into(),
                    clips: vec![clip(
                        0,
                        4.0,
                        TimeMap::Linear {
                            trim: 0.0,
                            rate: 1.0,
                        },
                    )],
                    strip: voice,
                },
                Track {
                    id: "music".into(),
                    clips: vec![clip(
                        1,
                        4.0,
                        TimeMap::Points(vec![(0.0, 0.0), (2.0, 3.0), (4.0, 1.0)]),
                    )],
                    strip: strip(vec![Effect::Compressor(comp)], Target::Master),
                },
            ],
            buses: vec![Bus {
                id: "room".into(),
                strip: strip(
                    vec![Effect::Reverb(Reverb {
                        room: 0.5,
                        damping: 0.5,
                        wet: 1.0 / 3.0,
                        dry: 0.0,
                        width: 1.0,
                    })],
                    Target::Master,
                ),
            }],
            master: strip(
                vec![Effect::Limiter(Limiter {
                    ceiling_db: -1.0,
                    release: 0.05,
                    lookahead: 0.005,
                })],
                Target::Master,
            ),
            length: (4.0 * FS) as i64,
        }
    }

    fn render(block: usize, from: i64) -> Vec<f32> {
        let mut m = Mixer::new(mix(), vec![noise(5.0, 1), noise(5.0, 2)]).unwrap();
        m.seek(from);
        let mut out = vec![0.0f32; 2 * (m.length() - from) as usize];
        for c in out.chunks_mut(2 * block) {
            m.render(c);
        }
        out
    }

    /// Any block size, the same samples.
    #[test]
    fn blocks_do_not_matter() {
        let a = render(4096, 0);
        for b in [1, 7, 480, 1024] {
            assert!(a == render(b, 0), "block {b}");
        }
    }

    /// Starting at 3 s after pre-roll: within -100 dBFS of the whole render.
    #[test]
    fn preroll_settles() {
        let whole = render(1024, 0);
        let from = (3.0 * FS) as i64;
        let tail = render(1024, from);
        let worst = whole[2 * from as usize..]
            .iter()
            .zip(&tail)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "{worst}");
    }

    /// The limiter: no true peak over its ceiling (a tenth of a dB for inter-sample error).
    #[test]
    fn limiter_holds_the_ceiling() {
        let out = render(1024, 0);
        let mut meter = loudness::Meter::default();
        meter.push(&out);
        let l = meter.result();
        assert!(l.true_peak <= -0.9, "{}", l.true_peak);
    }

    #[test]
    fn eq_matches_its_response() {
        let band = Band {
            kind: BandKind::Peaking,
            frequency: 1000.0,
            q: 1.0,
            gain_db: 6.0,
        };
        let c = dsp::Coefficients::band(&band);
        assert!((decibels(c.response(1000.0)) - 6.0).abs() < 1e-9);
        let mut eq = Eq::new(&[band]);
        for f in [100.0, 700.0, 1000.0, 3000.0] {
            // The second second's amplitude, by quadrature over its whole periods.
            let (mut a, mut b) = (0.0, 0.0);
            for i in 0..96_000 {
                let w = std::f64::consts::TAU * f * i as f64 / FS;
                let y = eq.run([w.sin(), w.sin()])[0];
                if i >= 48_000 {
                    a += y * w.sin();
                    b += y * w.cos();
                }
            }
            let amp = 2.0 * a.hypot(b) / 48_000.0;
            assert!(
                (decibels(amp) - decibels(c.response(f))).abs() < 0.01,
                "{f} Hz: {} vs {}",
                decibels(amp),
                decibels(c.response(f))
            );
        }
    }

    #[test]
    fn compressor_curve() {
        let c = Compressor {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            attack: 0.001,
            release: 0.1,
            makeup_db: 0.0,
            range_db: None,
            key: None,
        };
        assert_eq!(Comp::curve(&c, -30.0), -30.0);
        assert!((Comp::curve(&c, -8.0) - -17.0).abs() < 1e-12);
        // A steady -8 dBFS square settles to 9 dB of reduction.
        let mut comp = Comp::new(&c);
        let a = amplitude(-8.0);
        let mut y = [0.0; 2];
        for i in 0..48_000 {
            let x = if i % 2 == 0 { a } else { -a };
            y = comp.run([x, x], [x, x]);
        }
        assert!(
            (decibels(y[0].abs()) - -17.0).abs() < 0.01,
            "{}",
            decibels(y[0].abs())
        );
    }
}
