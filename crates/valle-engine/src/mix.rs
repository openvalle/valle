//! A Timeline document's sound as a mix (`audio::Mix`), and the mixer that plays it: its audio
//! tracks, the own sound of each visual track's video and nested timeline clips, and the `mix`
//! section's buses and master. A nested timeline's clip sounds its document's mix.
//!
//! Gains are amplitude factors, as the Timeline's are; levels inside effects are decibels
//! (`…Db`), times seconds. A track's or bus's `gain`, `pan` and send gains read the timeline's
//! seconds, a clip's its own; a track's or bus's fader comes after its effects, the master's
//! before them (into its limiter). A crossfade fades the two clips over its span with equal
//! power, each reading its source past the cut.

use serde_json::{Map, Value};
use valle_timeline::{Animated, Clip, Content, Document, End as SourceEnd, SourceTime, Track};

use crate::audio::{
    Band, BandKind, Bus, Clip as Sounding, Compressor, Effect, End, Key, Limiter, Mix, Mixer,
    Param, Pcm, RATE, Reverb, Send, Strip, Target, TimeMap, Track as Channel,
};
use crate::ease::{Easing, Jump};
use crate::front;
use crate::template::Error;
use crate::time::Tick;

const FS: f64 = RATE as f64;

/// What a mix's sources are: sound files, or nested timelines, by path.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(String),
    Timeline(String),
}

/// What mixing reads from the host: a file's text, and a file's sound, interleaved stereo at
/// `RATE` (none when it has no audio).
pub struct Host<'a> {
    pub read: &'a dyn Fn(&str) -> Option<String>,
    pub sound: &'a Decode,
}

/// A file's sound, as `Host` decodes it.
pub type Decode = dyn Fn(&str) -> Result<Option<Vec<f32>>, String>;

/// The mixer of the document at `path` (`/`-separated), its sources decoded; `nesting` are the
/// documents it is nested in.
pub fn mixer(path: &str, doc: &Document, host: &Host<'_>) -> Result<Mixer, Error> {
    mixer_in(path, doc, host, &[])
}

/// Validate the mix's sources without decoding their PCM. Only audio clips require a sound
/// stream; a visual clip may be silent. Nested documents use the same rules as the mixer.
pub fn check(
    path: &str,
    doc: &Document,
    read: &dyn Fn(&str) -> Option<String>,
    audio: &dyn Fn(&str) -> Result<bool, String>,
) -> Result<(), Error> {
    fn visit(
        path: &str,
        doc: &Document,
        read: &dyn Fn(&str) -> Option<String>,
        audio: &dyn Fn(&str) -> Result<bool, String>,
        nesting: &mut Vec<String>,
    ) -> Result<(), Error> {
        if nesting.iter().any(|p| p == path) {
            return Err(format!("{path} is nested in itself"));
        }
        nesting.push(path.to_string());
        let (mix, sources) = mix(path, doc)?;
        for (i, source) in sources.iter().enumerate() {
            match source {
                Source::File(file) => {
                    let origin = source_origin(path, doc, &mix, i);
                    if !audio(file).map_err(|e| format!("{origin}: {e}"))?
                        && let Some(c) = mix
                            .tracks
                            .iter()
                            .flat_map(|t| &t.clips)
                            .find(|c| c.source == i && !c.optional)
                    {
                        return Err(format!(
                            "{origin}: {}: its file has no sound ({file})",
                            c.what
                        ));
                    }
                }
                Source::Timeline(file) => {
                    let origin = source_origin(path, doc, &mix, i);
                    let text = read(file).ok_or_else(|| format!("{origin}: {file}: not found"))?;
                    let inner = valle_timeline::parse(&text).map_err(|e| format!("{file}: {e}"))?;
                    visit(file, &inner, read, audio, nesting)?;
                }
            }
        }
        nesting.pop();
        Ok(())
    }
    visit(path, doc, read, audio, &mut Vec::new())
}

/// Keep the user's alias when the mix has deduplicated files into numeric source slots.
fn source_origin(path: &str, doc: &Document, mix: &Mix, index: usize) -> String {
    let clip = mix
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.source == index)
        .and_then(|c| {
            doc.tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|d| d.at == c.what)
        });
    let alias = clip.and_then(|c| match &c.content {
        Content::Video { src } | Content::Audio { src } | Content::Timeline { src } => Some(src),
        _ => None,
    });
    match alias {
        Some(alias) => format!("{path}: resources.{alias}"),
        None => path.to_string(),
    }
}

fn mixer_in(path: &str, doc: &Document, host: &Host<'_>, nesting: &[&str]) -> Result<Mixer, Error> {
    let (mix, sources) = mix(path, doc)?;
    let mut pcm = Vec::new();
    for s in &sources {
        pcm.push(Pcm(match s {
            Source::File(file) => (host.sound)(file)?.unwrap_or_default().into(),
            Source::Timeline(file) => {
                if nesting.contains(&file.as_str()) || file == path {
                    return Err(format!("{file} is nested in itself"));
                }
                let text = (host.read)(file).ok_or_else(|| format!("{file}: not found"))?;
                let inner = valle_timeline::parse(&text).map_err(|e| format!("{file}: {e}"))?;
                let mut deeper = nesting.to_vec();
                deeper.push(path);
                let mut m = mixer_in(file, &inner, host, &deeper)?;
                let mut out = vec![0.0; 2 * m.length() as usize];
                m.render(&mut out);
                out.into()
            }
        }));
    }
    // An audio clip with no sound in its file.
    for t in &mix.tracks {
        for c in &t.clips {
            if !c.optional && pcm[c.source].0.is_empty() {
                return Err(format!("{}: its file has no sound", c.what));
            }
        }
    }
    Mixer::new(mix, pcm)
}

/// Output samples at `t`: 48 000 a second divides the ticks a second.
fn samples(t: Tick) -> i64 {
    t.0 / (crate::time::TICKS_PER_SECOND / i64::from(RATE))
}

/// The mix of the document at `path`, and its sources.
pub fn mix(path: &str, doc: &Document) -> Result<(Mix, Vec<Source>), Error> {
    crate::entry::Kind::Timeline.check(path)?;
    let folder = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let section = doc.mix.clone().unwrap_or(Value::Object(Map::new()));
    fields(&section, &["buses", "master"], "mix")?;
    let sounds = |c: &Clip| {
        c.enabled && matches!(c.content, Content::Video { .. } | Content::Timeline { .. })
    };
    let silent =
        |t: &Track| matches!(&t.sound.gain, Some(Animated::Constant(v)) if v.as_f64() == Some(0.0));
    let tracks: Vec<&Track> = (doc.tracks.iter())
        .filter(|t| t.enabled)
        .filter(|t| match t.kind {
            valle_timeline::Kind::Audio => true,
            valle_timeline::Kind::Visual => !silent(t) && t.clips.iter().any(sounds),
            _ => false,
        })
        .collect();
    let mut names = Names {
        tracks: tracks.iter().map(|t| t.id.clone()).collect(),
        buses: Vec::new(),
    };
    for (i, b) in list(&section["buses"]).iter().enumerate() {
        names
            .buses
            .push(text(&b["id"], &format!("mix.buses[{i}].id"))?.to_string());
    }
    let all: Vec<&String> = names.tracks.iter().chain(&names.buses).collect();
    if let Some((_, d)) =
        (all.iter().enumerate()).find(|(i, a)| a.as_str() == "master" || all[..*i].contains(a))
    {
        return Err(format!(
            "mix: {d:?} names two of the tracks and buses, or the master; ids are unique over \
             them, and \"master\" is the master's"
        ));
    }
    let mut sources: Vec<Source> = Vec::new();
    let mut source = |s: Source| match sources.iter().position(|x| *x == s) {
        Some(i) => i,
        None => {
            sources.push(s);
            sources.len() - 1
        }
    };
    let file = |alias: &str, at: &str| -> Result<String, Error> {
        let r = (doc.resources.get(alias))
            .ok_or_else(|| format!("{at}: no resource {alias:?} in resources"))?;
        let f = r
            .files
            .first()
            .ok_or_else(|| format!("{at}: {alias} has no file"))?;
        Ok(match f.starts_with('/') || front::web(f) {
            true => f.clone(),
            false => front::join(folder, f),
        })
    };
    let mut channels = Vec::new();
    for t in &tracks {
        let mut clips = Vec::new();
        for c in t.clips.iter().filter(|c| c.enabled) {
            let (s, optional) = match &c.content {
                Content::Audio { src } => {
                    (Source::File(file(src, &format!("{}.src", c.at))?), false)
                }
                Content::Video { src } => {
                    (Source::File(file(src, &format!("{}.src", c.at))?), true)
                }
                Content::Timeline { src } => {
                    let path = file(src, &format!("{}.src", c.at))?;
                    crate::entry::Kind::Timeline
                        .check(&path)
                        .map_err(|e| format!("{}.src: {e}", c.at))?;
                    (Source::Timeline(path), true)
                }
                _ => continue,
            };
            clips.push(clip(c, source(s), optional)?);
        }
        // Crossfades: each clip reads past the cut into the other's span, and the two fade over
        // it.
        for x in &t.transitions {
            let (lo, hi) = x.span(t);
            let fade = samples(hi) - samples(lo);
            let (from, to) = (&t.clips[x.from], &t.clips[x.to]);
            if let Some(a) = clips.iter_mut().find(|c| c.what == from.at) {
                a.end = samples(hi);
                a.fades[1] = fade;
            }
            if let Some(b) = clips.iter_mut().find(|c| c.what == to.at) {
                let early = (to.start - lo).seconds();
                b.start = samples(lo);
                b.fades[0] = fade;
                b.time = match &b.time {
                    TimeMap::Linear { trim, rate } => TimeMap::Linear {
                        trim: trim - early * rate,
                        rate: *rate,
                    },
                    TimeMap::Points(p) => {
                        TimeMap::Points(p.iter().map(|&(x, y)| (x + early, y)).collect())
                    }
                };
            }
        }
        let strip = strip(&t.sound_value(), &names, &t.at, false)?;
        channels.push(Channel {
            id: t.id.clone(),
            clips,
            strip,
        });
    }
    let mut buses = Vec::new();
    for (i, b) in list(&section["buses"]).iter().enumerate() {
        let what = format!("mix.buses[{i}]");
        fields(
            b,
            &["id", "effects", "gain", "pan", "sends", "output"],
            &what,
        )?;
        buses.push(Bus {
            id: names.buses[i].clone(),
            strip: strip(b, &names, &what, false)?,
        });
    }
    let master = &section["master"];
    if !master.is_null() {
        fields(master, &["effects", "gain", "pan"], "mix.master")?;
    }
    let master = strip(master, &names, "mix.master", true)?;
    Ok((
        Mix {
            sources: sources.iter().map(|s| format!("{s:?}")).collect(),
            tracks: channels,
            buses,
            master,
            length: samples(doc.duration),
        },
        sources,
    ))
}

/// A track's sound as the `mix` section writes a strip.
trait SoundValue {
    fn sound_value(&self) -> Value;
}

impl SoundValue for Track {
    fn sound_value(&self) -> Value {
        let s = &self.sound;
        let mut o = Map::new();
        if let Some(g) = &s.gain {
            o.insert("gain".into(), animated(g));
        }
        if let Some(p) = &s.pan {
            o.insert("pan".into(), animated(p));
        }
        o.insert("effects".into(), Value::Array(s.effects.clone()));
        o.insert("sends".into(), Value::Array(s.sends.clone()));
        if let Some(out) = &s.output {
            o.insert("output".into(), Value::String(out.clone()));
        }
        Value::Object(o)
    }
}

/// A value as the document writes it: a constant, or `{ keyframes, interpolation }`.
fn animated(a: &Animated) -> Value {
    match a {
        Animated::Constant(v) => v.clone(),
        Animated::Keys { keys, step } => {
            let keys: Vec<Value> = (keys.iter())
                .map(|k| {
                    let mut v = vec![Value::from(k.at.seconds()), k.value.clone()];
                    v.extend(k.easing.clone().map(Value::String));
                    Value::Array(v)
                })
                .collect();
            let mut o = Map::new();
            o.insert("keyframes".into(), Value::Array(keys));
            if *step {
                o.insert("interpolation".into(), "step".into());
            }
            Value::Object(o)
        }
    }
}

/// A clip of a track on its source: its span, its time, its gain and pan on its own seconds.
fn clip(c: &Clip, source: usize, optional: bool) -> Result<Sounding, Error> {
    let time = match &c.source.time {
        SourceTime::Linear { trim, rate } => TimeMap::Linear {
            trim: trim.seconds(),
            rate: rate.as_f64(),
        },
        SourceTime::Map(points) => TimeMap::Points(
            (points.iter())
                .map(|(x, y)| (x.seconds(), y.seconds()))
                .collect(),
        ),
    };
    let end_policy = match c.source.end {
        SourceEnd::Error => End::Error,
        SourceEnd::Hold => End::Hold,
        SourceEnd::Loop => End::Loop,
    };
    let value = |a: &Option<Animated>| a.as_ref().map(animated).unwrap_or(Value::Null);
    Ok(Sounding {
        source,
        start: samples(c.start),
        end: samples(c.end()),
        time,
        end_policy,
        gain: gain(&value(&c.sound.gain), &format!("{}.gain", c.at))?,
        pan: param(&value(&c.sound.pan), 0.0, &format!("{}.pan", c.at))?,
        optional,
        fades: [0, 0],
        what: c.at.clone(),
    })
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn text<'a>(v: &'a Value, what: &str) -> Result<&'a str, Error> {
    v.as_str().ok_or_else(|| format!("{what}: text"))
}

fn num(v: &Value, what: &str) -> Result<f64, Error> {
    (v.as_f64().filter(|x| x.is_finite())).ok_or_else(|| format!("{what}: a number"))
}

fn number_or(v: &Value, default: f64, what: &str) -> Result<f64, Error> {
    match v {
        Value::Null => Ok(default),
        v => num(v, what),
    }
}

/// Fails on any field of `v` not in `known`.
fn fields(v: &Value, known: &[&str], what: &str) -> Result<(), Error> {
    let Some(o) = v.as_object() else {
        return Err(format!("{what}: an object"));
    };
    match o.keys().find(|k| !known.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "{what}: no field {k:?}; it takes {}",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

/// A constant or `{ keyframes: [[t, v, easing?], …], interpolation }`, on seconds.
fn param(v: &Value, default: f64, what: &str) -> Result<Param, Error> {
    match v {
        Value::Null => Ok(Param::Const(default)),
        Value::Object(o) => {
            fields(v, &["keyframes", "interpolation"], what)?;
            let step = match o.get("interpolation").and_then(Value::as_str) {
                None | Some("linear") => false,
                Some("step") => true,
                Some(other) => {
                    return Err(format!(
                        "{what}.interpolation: \"linear\" or \"step\", not {other:?}"
                    ));
                }
            };
            let mut keys = Vec::new();
            for (i, k) in list(&o["keyframes"]).iter().enumerate() {
                let w = format!("{what}.keyframes[{i}]");
                let k = list(k);
                let easing = match (step, k.get(2)) {
                    (true, _) => Easing::Steps {
                        count: 1,
                        jump: Jump::End,
                    },
                    (false, None) => Easing::Linear,
                    (false, Some(e)) => {
                        Easing::parse(text(e, &w)?).map_err(|e| format!("{w}: {e}"))?
                    }
                };
                keys.push((num(&k[0], &w)?, num(&k[1], &w)?, easing));
            }
            if keys.is_empty() {
                return Err(format!("{what}.keyframes: one or more"));
            }
            Ok(Param::Keys(keys))
        }
        v => Ok(Param::Const(num(v, what)?)),
    }
}

/// An amplitude factor, 0 or more.
fn gain(v: &Value, what: &str) -> Result<Param, Error> {
    let p = param(v, 1.0, what)?;
    let ok = match &p {
        Param::Const(g) => *g >= 0.0,
        Param::Keys(k) => k.iter().all(|k| k.1 >= 0.0),
    };
    match ok {
        true => Ok(p),
        false => Err(format!(
            "{what}: an amplitude factor, 0 or more: 0 silent, 1 as it is"
        )),
    }
}

struct Names {
    tracks: Vec<String>,
    buses: Vec<String>,
}

impl Names {
    fn bus(&self, id: &Value, what: &str) -> Result<usize, Error> {
        let id = text(id, what)?;
        (self.buses.iter().position(|b| b == id))
            .ok_or_else(|| format!("{what}: no bus {id:?} in mix.buses"))
    }
}

fn effect(v: &Value, names: &Names, what: &str) -> Result<Effect, Error> {
    let kind = text(&v["type"], &format!("{what}.type"))?;
    let f = |name: &str| format!("{what}.{name}");
    Ok(match kind {
        "eq" => {
            fields(v, &["type", "bands"], what)?;
            let mut bands = Vec::new();
            for (i, b) in list(&v["bands"]).iter().enumerate() {
                let w = format!("{what}.bands[{i}]");
                fields(b, &["type", "frequency", "q", "gainDb"], &w)?;
                let kind = match text(&b["type"], &format!("{w}.type"))? {
                    "highpass" => BandKind::Highpass,
                    "lowpass" => BandKind::Lowpass,
                    "peaking" => BandKind::Peaking,
                    "lowShelf" => BandKind::LowShelf,
                    "highShelf" => BandKind::HighShelf,
                    o => {
                        return Err(format!(
                            "{w}.type: highpass, lowpass, peaking, lowShelf or highShelf, not {o:?}"
                        ));
                    }
                };
                let frequency = num(&b["frequency"], &format!("{w}.frequency"))?;
                let q = number_or(&b["q"], 0.707, &format!("{w}.q"))?;
                if !(frequency > 0.0 && frequency < FS / 2.0 && q > 0.0) {
                    return Err(format!(
                        "{w}: a frequency above 0 and below 24000 Hz, and a q above 0"
                    ));
                }
                bands.push(Band {
                    kind,
                    frequency,
                    q,
                    gain_db: number_or(&b["gainDb"], 0.0, &format!("{w}.gainDb"))?,
                });
            }
            Effect::Eq(bands)
        }
        "compressor" => {
            let known = [
                "type",
                "thresholdDb",
                "ratio",
                "kneeDb",
                "attack",
                "release",
                "makeupDb",
                "rangeDb",
                "sidechain",
            ];
            fields(v, &known, what)?;
            let key = match &v["sidechain"] {
                Value::Null => None,
                id => {
                    let w = f("sidechain");
                    let id = text(id, &w)?;
                    if let Some(t) = names.tracks.iter().position(|t| t == id) {
                        Some(Key::Track(t))
                    } else if let Some(b) = names.buses.iter().position(|b| b == id) {
                        Some(Key::Bus(b))
                    } else {
                        return Err(format!("{w}: no track or bus {id:?} that sounds"));
                    }
                }
            };
            let c = Compressor {
                threshold_db: num(&v["thresholdDb"], &f("thresholdDb"))?,
                ratio: num(&v["ratio"], &f("ratio"))?,
                knee_db: number_or(&v["kneeDb"], 0.0, &f("kneeDb"))?,
                attack: number_or(&v["attack"], 0.01, &f("attack"))?,
                release: number_or(&v["release"], 0.1, &f("release"))?,
                makeup_db: number_or(&v["makeupDb"], 0.0, &f("makeupDb"))?,
                range_db: match &v["rangeDb"] {
                    Value::Null => None,
                    r => Some(num(r, &f("rangeDb"))?),
                },
                key,
            };
            if c.ratio < 1.0 || c.knee_db < 0.0 || c.attack < 0.0 || c.release <= 0.0 {
                return Err(format!(
                    "{what}: a ratio of 1 or more, kneeDb and attack 0 or more, release above 0"
                ));
            }
            Effect::Compressor(c)
        }
        "reverb" => {
            fields(v, &["type", "room", "damping", "wet", "dry", "width"], what)?;
            let r = Reverb {
                room: number_or(&v["room"], 0.5, &f("room"))?,
                damping: number_or(&v["damping"], 0.5, &f("damping"))?,
                wet: number_or(&v["wet"], 1.0 / 3.0, &f("wet"))?,
                dry: number_or(&v["dry"], 0.0, &f("dry"))?,
                width: number_or(&v["width"], 1.0, &f("width"))?,
            };
            let unit = [r.room, r.damping, r.width];
            if !unit.iter().all(|x| (0.0..=1.0).contains(x)) || r.wet < 0.0 || r.dry < 0.0 {
                return Err(format!(
                    "{what}: room, damping and width from 0 to 1, wet and dry 0 or more"
                ));
            }
            Effect::Reverb(r)
        }
        "limiter" => {
            fields(v, &["type", "ceilingDb", "release", "lookahead"], what)?;
            let l = Limiter {
                ceiling_db: number_or(&v["ceilingDb"], -1.0, &f("ceilingDb"))?,
                release: number_or(&v["release"], 0.05, &f("release"))?,
                lookahead: number_or(&v["lookahead"], 0.005, &f("lookahead"))?,
            };
            if l.ceiling_db > 0.0 || l.release <= 0.0 || !(0.0005..=0.05).contains(&l.lookahead) {
                return Err(format!(
                    "{what}: ceilingDb 0 or less, release above 0, lookahead from 0.0005 to 0.05 s"
                ));
            }
            Effect::Limiter(l)
        }
        o => {
            return Err(format!(
                "{what}.type: eq, compressor, reverb or limiter, not {o:?}"
            ));
        }
    })
}

fn strip(v: &Value, names: &Names, what: &str, master: bool) -> Result<Strip, Error> {
    let effects = (list(&v["effects"]).iter().enumerate())
        .map(|(i, e)| effect(e, names, &format!("{what}.effects[{i}]")))
        .collect::<Result<_, _>>()?;
    let mut sends = Vec::new();
    for (i, s) in list(&v["sends"]).iter().enumerate() {
        let w = format!("{what}.sends[{i}]");
        if master {
            return Err(format!("{w}: the master sends nowhere"));
        }
        fields(s, &["to", "gain"], &w)?;
        sends.push(Send {
            to: names.bus(&s["to"], &format!("{w}.to"))?,
            gain: gain(&s["gain"], &format!("{w}.gain"))?,
        });
    }
    let output = match &v["output"] {
        Value::Null => Target::Master,
        _ if master => return Err(format!("{what}.output: the master is the output")),
        id if id.as_str() == Some("master") => Target::Master,
        id => Target::Bus(names.bus(id, &format!("{what}.output"))?),
    };
    Ok(Strip {
        effects,
        gain: gain(&v["gain"], &format!("{what}.gain"))?,
        pan: param(&v["pan"], 0.0, &format!("{what}.pan"))?,
        sends,
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checking_nested_sound_rejects_cycles_without_decoding() {
        let text = r#"{
            "canvas":{"width":64,"height":36,"fps":30},
            "resources":{"inner":"edit.timeline.json"},
            "tracks":{"visual":[{"items":[{"id":"nested","kind":"timeline","src":"inner","duration":1}]}]}
        }"#;
        let doc = valle_timeline::parse(text).unwrap();
        let error = check("edit.timeline.json", &doc, &|_| Some(text.into()), &|_| {
            panic!("a timeline is not an audio file")
        })
        .unwrap_err();
        assert!(error.contains("nested in itself"), "{error}");
    }

    /// A file of a steady 0.5 on both channels, 4 s long; "quiet.mp4" has no sound.
    fn sound(path: &str) -> Result<Option<Vec<f32>>, String> {
        Ok(match path {
            "quiet.mp4" => None,
            _ => Some(vec![0.5; 2 * 4 * RATE as usize]),
        })
    }

    fn render(json: &str) -> Result<Vec<f32>, Error> {
        let doc = valle_timeline::parse(json)?;
        let read = |_: &str| None;
        let host = Host {
            read: &read,
            sound: &sound,
        };
        let mut m = mixer("edit.timeline.json", &doc, &host)?;
        let mut out = vec![0.0; 2 * m.length() as usize];
        m.render(&mut out);
        Ok(out)
    }

    fn at(out: &[f32], seconds: f64) -> f32 {
        out[2 * (seconds * FS) as usize]
    }

    #[test]
    fn a_crossfade_keeps_the_power() {
        let out = render(
            r#"{ "canvas": { "width": 64, "height": 36, "fps": 30 },
                 "resources": { "a": "a.wav", "b": "b.wav" },
                 "tracks": { "audio": [{ "items": [
                   { "id": "a", "src": "a", "duration": 1, "gain": 2 },
                   { "transition": "crossfade", "inOffset": 0.5, "outOffset": 0.5 },
                   { "id": "b", "src": "b", "duration": 1, "trimStart": 1 } ] }] } }"#,
        )
        .unwrap();
        // The first clip twice as loud alone; the two at the crossfade's middle each at
        // sin(π/4) of their own.
        assert!((at(&out, 0.25) - 1.0).abs() < 1e-6);
        let middle = (1.0 + 0.5) * std::f32::consts::FRAC_1_SQRT_2;
        assert!((at(&out, 1.0) - middle).abs() < 1e-3, "{}", at(&out, 1.0));
        assert!((at(&out, 1.75) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_video_sounds_unless_its_track_is_silent() {
        let doc = |gain: &str| {
            format!(
                r#"{{ "canvas": {{ "width": 64, "height": 36, "fps": 30 }},
                     "resources": {{ "v": "v.mp4", "q": "quiet.mp4" }},
                     "tracks": {{ "visual": [
                       {{ "gain": {gain}, "clips": [{{ "kind": "video", "src": "v", "start": 0, "duration": 1 }}] }},
                       {{ "clips": [{{ "kind": "video", "src": "q", "start": 0, "duration": 1 }}] }} ] }} }}"#
            )
        };
        assert!((at(&render(&doc("1")).unwrap(), 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(at(&render(&doc("0")).unwrap(), 0.5), 0.0);
    }

    #[test]
    fn mistakes_name_the_field() {
        let e = render(
            r#"{ "canvas": { "width": 64, "height": 36, "fps": 30 },
                 "resources": { "a": "quiet.mp4" },
                 "tracks": { "audio": [{ "effects": [{ "type": "echo" }],
                   "clips": [{ "src": "a", "start": 0, "duration": 1 }] }] } }"#,
        );
        assert!(
            e.unwrap_err()
                .contains("tracks.audio[0].effects[0].type: eq, compressor")
        );
    }
}
