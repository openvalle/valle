//! Sound: a mix as a graph, as a mixing console has it. Clips sound on tracks; each track's strip
//! runs its inserts (`effects`, in order), then its fader (`gain`, `pan`), into a bus or the master,
//! and sends what leaves the fader to buses (`sends`); a compressor can listen to another track or
//! bus (its `sidechain` key). Buses are strips too, and the master is the last; its fader comes
//! before its inserts (a master fader into the final limiter), so what its limiter lets out is
//! what leaves.
//!
//! Everything is 48 kHz stereo, computed in f64 one sample after another, so blocks of any size
//! give the same samples (the samples are a function of the mix alone). Automated values (`gain`,
//! `pan`, send gains) are read per sample. Starting anywhere but the start runs the graph from
//! `Mixer::preroll` before it, so what effects remember (filters, envelopes, reverb tails) has
//! settled to under -100 dBFS of the export's.

mod dsp;
pub mod loudness;
mod render;

use crate::ease::Easing;

pub use render::{GainReduction, Mixer, Pcm};

/// Samples per second, every source resampled to it.
pub const RATE: u32 = 48_000;

/// A value over time: a constant, or keyframes (seconds on its clock, value, the easing leaving
/// it), constant before the first and after the last.
#[derive(Clone, Debug)]
pub enum Param {
    Const(f64),
    Keys(Vec<(f64, f64, Easing)>),
}

impl Param {
    pub fn at(&self, t: f64) -> f64 {
        match self {
            Param::Const(v) => *v,
            Param::Keys(k) => {
                let i = k.partition_point(|p| p.0 <= t);
                if i == 0 {
                    return k[0].1;
                }
                if i == k.len() {
                    return k[i - 1].1;
                }
                let (a, b) = (&k[i - 1], &k[i]);
                let u = (t - a.0) / (b.0 - a.0);
                a.1 + (b.1 - a.1) * a.2.at(u)
            }
        }
    }
}

/// Where a clip's source is at clip seconds `x`: `trim + x · rate`, or a time map's
/// [clip seconds, source seconds] points, straight between them and held past the ends.
#[derive(Clone, Debug)]
pub enum TimeMap {
    Linear { trim: f64, rate: f64 },
    Points(Vec<(f64, f64)>),
}

impl TimeMap {
    /// Source seconds at clip seconds `x`, and how fast the source passes there (its slope).
    pub fn at(&self, x: f64) -> (f64, f64) {
        match self {
            TimeMap::Linear { trim, rate } => (trim + x * rate, *rate),
            TimeMap::Points(p) => {
                let i = p.partition_point(|q| q.0 <= x);
                if i == 0 {
                    return (p[0].1, 0.0);
                }
                if i == p.len() {
                    return (p[i - 1].1, 0.0);
                }
                let (a, b) = (p[i - 1], p[i]);
                let slope = (b.1 - a.1) / (b.0 - a.0);
                (a.1 + (x - a.0) * slope, slope)
            }
        }
    }
}

/// What a clip does past its source's end: rejected, the last sample held, or the source again
/// from its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Error,
    Hold,
    Loop,
}

#[derive(Clone, Debug)]
pub struct Clip {
    /// Index into `Mix::sources`.
    pub source: usize,
    /// Output samples it sounds over, [start, end).
    pub start: i64,
    pub end: i64,
    pub time: TimeMap,
    pub end_policy: End,
    /// On clip seconds.
    pub gain: Param,
    pub pan: Param,
    /// A video's own sound: silent where the file has none.
    pub optional: bool,
    /// Samples it fades in over from its start, and out over to its end, as crossfades have
    /// them: equal power, sin and cos of a quarter turn.
    pub fades: [i64; 2],
    /// Where the clip is in the document, for messages.
    pub what: String,
}

/// Where a strip's output goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Master,
    Bus(usize),
}

#[derive(Clone, Debug)]
pub struct Send {
    pub to: usize,
    pub gain: Param,
}

/// A track's, a bus's or the master's processing: inserts, then fader and pan (timeline seconds),
/// then out and to its sends.
#[derive(Clone, Debug)]
pub struct Strip {
    pub effects: Vec<Effect>,
    pub gain: Param,
    pub pan: Param,
    pub sends: Vec<Send>,
    pub output: Target,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: String,
    pub clips: Vec<Clip>,
    pub strip: Strip,
}

#[derive(Clone, Debug)]
pub struct Bus {
    pub id: String,
    pub strip: Strip,
}

/// A whole mix: sources by alias, tracks, buses and the master, `length` samples long.
#[derive(Clone, Debug)]
pub struct Mix {
    pub sources: Vec<String>,
    pub tracks: Vec<Track>,
    pub buses: Vec<Bus>,
    pub master: Strip,
    pub length: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKind {
    Highpass,
    Lowpass,
    Peaking,
    LowShelf,
    HighShelf,
}

/// An equalizer band, as the RBJ cookbook defines them: `frequency` Hz, `q`, `gain_db` (peaking and
/// shelves).
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub kind: BandKind,
    pub frequency: f64,
    pub q: f64,
    pub gain_db: f64,
}

/// What a compressor listens to: its own input, or a track's or bus's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Track(usize),
    Bus(usize),
}

/// A feed-forward compressor on the louder channel's peak, in decibels: above `threshold_db` (a
/// `knee_db` wide soft knee) the level rises 1/`ratio` as fast; `attack` and `release` (seconds)
/// smooth the gain reduction, `range_db` caps it (a ducker's depth), `makeup_db` is added.
#[derive(Clone, Copy, Debug)]
pub struct Compressor {
    pub threshold_db: f64,
    pub ratio: f64,
    pub knee_db: f64,
    pub attack: f64,
    pub release: f64,
    pub makeup_db: f64,
    pub range_db: Option<f64>,
    pub key: Option<Key>,
}

/// Freeverb (Jezar's, public domain): `room` size and `damping` 0–1, `wet` and `dry` levels,
/// stereo `width`.
#[derive(Clone, Copy, Debug)]
pub struct Reverb {
    pub room: f64,
    pub damping: f64,
    pub wet: f64,
    pub dry: f64,
    pub width: f64,
}

/// A look-ahead limiter on true peaks (4× oversampled, BS.1770): nothing leaves above
/// `ceiling_db`; the gain comes back over `release` seconds. On the master only: its look-ahead
/// delays what it limits, and the mixer delays everything by as much (latency compensated).
#[derive(Clone, Copy, Debug)]
pub struct Limiter {
    pub ceiling_db: f64,
    pub release: f64,
    pub lookahead: f64,
}

#[derive(Clone, Debug)]
pub enum Effect {
    Eq(Vec<Band>),
    Compressor(Compressor),
    Reverb(Reverb),
    Limiter(Limiter),
}

/// Decibels as an amplitude factor.
pub fn amplitude(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// An amplitude in decibels (-inf for 0).
pub fn decibels(a: f64) -> f64 {
    20.0 * a.log10()
}
