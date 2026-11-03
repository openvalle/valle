//! The Timeline document, `*.timeline.json`: the edit as AI, Studio and other editors read and
//! write it. Parsing checks every field, so a field it does not know is an error that says what to
//! write instead, never ignored, and resolves the tracks to clips placed on the timeline in ticks.
//!
//! - A track is a sequence (`items`: clips one after another, `{ "gap": s }`, and transitions
//!   between two clips), or clips placed by `start`: seconds, or `{ "clip": id, "offset": s }`,
//!   anchored to a clip anywhere in the document, which it moves with.
//! - Times are seconds, read exactly as decimals; cuts within a thousandth of a frame of one are
//!   on it.
//!
//! The engine lowers a document to a Motion scene; this crate only holds it.

pub mod query;
pub mod time;
pub mod timecode;

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

use time::{Rate, Speed, Tick};

pub type Error = String;

pub struct Document {
    pub name: Option<String>,
    pub canvas: Canvas,
    pub start_timecode: Option<String>,
    pub markers: Vec<Value>,
    pub metadata: Option<Value>,
    pub resources: BTreeMap<String, Resource>,
    /// Visual tracks first, each kind in the order written: audio, caption, adjustment.
    pub tracks: Vec<Track>,
    /// The mix's buses and master, as written.
    pub mix: Option<Value>,
    /// When the last clip of any track ends.
    pub duration: Tick,
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: Rate,
    pub background: String,
}

/// A resource alias: its files, as written (a font family may have several), and the source's
/// own timecode, rate, length and name, for display and exchange.
pub struct Resource {
    pub files: Vec<String>,
    pub timecode: Option<String>,
    pub fps: Option<f64>,
    pub duration: Option<f64>,
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Visual,
    Audio,
    Caption,
    Adjustment,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Visual => "visual",
            Kind::Audio => "audio",
            Kind::Caption => "caption",
            Kind::Adjustment => "adjustment",
        }
    }
}

pub struct Track {
    pub id: String,
    pub kind: Kind,
    /// Where it is written, as `tracks.visual[0]`.
    pub at: String,
    /// Its clips by start.
    pub clips: Vec<Clip>,
    /// Transitions between two of its clips, by their indices.
    pub transitions: Vec<Transition>,
    pub sound: Sound,
    /// A caption track's style, the standard template's props.
    pub style: Option<Map<String, Value>>,
    /// A caption track's own component, a `captionPresenter()`.
    pub presenter: Option<Component>,
}

/// A track's or a clip's sound: its fader and pan, and a track's effects, sends and bus.
#[derive(Default)]
pub struct Sound {
    pub gain: Option<Animated>,
    pub pan: Option<Animated>,
    pub effects: Vec<Value>,
    pub sends: Vec<Value>,
    pub output: Option<String>,
}

pub struct Clip {
    /// Its id as written, or `track.n` from its track's id and its place there, from 1.
    pub id: String,
    /// Whether the id is written.
    pub named: bool,
    /// Where it is written, as `tracks.visual[0].items[2]`.
    pub at: String,
    pub name: Option<String>,
    pub markers: Vec<Value>,
    pub metadata: Option<Value>,
    pub enabled: bool,
    pub start: Tick,
    pub duration: Tick,
    /// The clip it is anchored to and the offset from that clip's start.
    pub anchor: Option<(String, Tick)>,
    pub content: Content,
    pub source: Source,
    pub look: Look,
    /// A video's or a nested timeline's own sound, or an audio clip's.
    pub sound: Sound,
}

impl Clip {
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
}

pub enum Content {
    Solid {
        color: String,
    },
    Image {
        src: String,
    },
    Video {
        src: String,
    },
    Motion(Component),
    Timeline {
        src: String,
    },
    Audio {
        src: String,
    },
    /// A caption: `{ text }` or `{ text, runs }`, its runs' times in ticks from its start.
    Caption {
        text: String,
        runs: Vec<Run>,
    },
    Grade {
        temperature: f64,
    },
}

pub struct Run {
    pub text: String,
    pub start: Tick,
    pub end: Tick,
}

/// A Motion component and what it is given: props (constants or keyframes), data, and its
/// resource slots bound to the document's aliases.
pub struct Component {
    pub component: String,
    pub props: Vec<(String, Animated)>,
    pub data: Option<Map<String, Value>>,
    pub resources: BTreeMap<String, String>,
}

/// Where a clip's source time comes from, and what happens past the source's end.
pub struct Source {
    pub time: SourceTime,
    pub end: End,
}

#[derive(Clone)]
pub enum SourceTime {
    /// `trimStart + clip time × rate`.
    Linear { trim: Tick, rate: Speed },
    /// [clip time, source time] points, straight between them, held past the ends.
    Map(Vec<(Tick, Tick)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Error,
    Hold,
    Loop,
}

/// Where a visual clip is on the canvas and how it is drawn there.
#[derive(Default)]
pub struct Look {
    pub position: Option<Animated>,
    pub size: Option<Animated>,
    pub scale: Option<Animated>,
    pub rotation: Option<Animated>,
    pub anchor: Option<[f64; 2]>,
    pub opacity: Option<Animated>,
    pub blend: Option<String>,
    pub fit: Option<Fit>,
    pub effects: Vec<Component>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Contain,
    Cover,
    Fill,
    None,
}

impl Fit {
    pub fn css(self) -> &'static str {
        match self {
            Fit::Contain => "contain",
            Fit::Cover => "cover",
            Fit::Fill => "fill",
            Fit::None => "none",
        }
    }
}

/// A value that is a constant or keyframes over clip time.
#[derive(Clone, Debug)]
pub enum Animated {
    Constant(Value),
    Keys { keys: Vec<Key>, step: bool },
}

#[derive(Clone, Debug)]
pub struct Key {
    pub at: Tick,
    pub value: Value,
    /// The easing of the stretch that leaves it.
    pub easing: Option<String>,
}

pub struct Transition {
    pub at: String,
    /// The clips it goes from and to, by their indices in the track.
    pub from: usize,
    pub to: usize,
    pub kind: String,
    /// How far the incoming clip starts before the cut, and the outgoing one runs past it.
    pub in_offset: Tick,
    pub out_offset: Tick,
    pub params: Map<String, Value>,
    /// A `motion` transition's component.
    pub component: Option<Component>,
    pub name: Option<String>,
    pub metadata: Option<Value>,
}

impl Transition {
    /// When it plays: from the cut less `in_offset` to the cut plus `out_offset`.
    pub fn span(&self, track: &Track) -> (Tick, Tick) {
        let cut = track.clips[self.to].start;
        (cut - self.in_offset, cut + self.out_offset)
    }
}

/// A transition's param: its name, default, low and high.
pub type Param = (&'static str, f64, f64, f64);

/// The built-in visual transitions and their params.
pub const TRANSITIONS: [(&str, &[Param]); 14] = [
    ("fade", &[]),
    ("wipeLeft", &[("softness", 0.0, 0.0, 1.0)]),
    ("wipeRight", &[("softness", 0.0, 0.0, 1.0)]),
    (
        "circleOpen",
        &[
            ("centerX", 0.5, 0.0, 1.0),
            ("centerY", 0.5, 0.0, 1.0),
            ("softness", 0.5, 0.0, 100.0),
        ],
    ),
    (
        "simpleZoom",
        &[
            ("quickness", 0.8, 0.2, 1.0),
            ("centerX", 0.5, 0.0, 1.0),
            ("centerY", 0.5, 0.0, 1.0),
        ],
    ),
    (
        "crossWarp",
        &[("centerX", 0.5, 0.0, 1.0), ("centerY", 0.5, 0.0, 1.0)],
    ),
    ("linearBlur", &[("intensity", 0.1, 0.0, 2.0)]),
    (
        "directionalWarp",
        &[
            ("directionX", -1.0, -1.0, 1.0),
            ("directionY", 1.0, -1.0, 1.0),
            ("softness", 0.5, 0.001, 1.0),
        ],
    ),
    (
        "dreamyZoom",
        &[
            ("strength", 0.5, 0.0, 4.0),
            ("centerX", 0.5, 0.0, 1.0),
            ("centerY", 0.5, 0.0, 1.0),
        ],
    ),
    (
        "ripple",
        &[
            ("frequency", 100.0, 0.0, 256.0),
            ("speed", 50.0, 0.0, 256.0),
            ("amplitude", 1.0 / 30.0, 0.0, 1.0),
        ],
    ),
    (
        "flyEye",
        &[
            ("size", 0.04, 0.0, 1.0),
            ("frequency", 50.0, 0.0, 256.0),
            ("colorSeparation", 0.3, 0.0, 1.0),
        ],
    ),
    ("multiplyBlend", &[("midpoint", 0.5, 0.001, 0.999)]),
    (
        "perlin",
        &[
            ("scale", 5.0, 0.001, 128.0),
            ("smoothness", 0.1, 0.001, 1.0),
        ],
    ),
    ("motion", &[]),
];

/// The blend modes a visual clip takes, Motion's `mix-blend-mode`.
pub const BLENDS: [&str; 17] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
    "plus-lighter",
];

/// The document in `text`.
pub fn parse(text: &str) -> Result<Document, Error> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    Document::read(&root)
}

impl Document {
    pub fn read(root: &Value) -> Result<Document, Error> {
        let o = object(root, "the document")?;
        only(
            o,
            "the document",
            &[
                "name",
                "canvas",
                "startTimecode",
                "markers",
                "metadata",
                "resources",
                "tracks",
                "mix",
            ],
        )?;
        let canvas = Canvas::read(o.get("canvas").ok_or("canvas: the document has none")?)?;
        let resources = match o.get("resources") {
            None => BTreeMap::new(),
            Some(r) => (object(r, "resources")?.iter())
                .map(|(k, v)| Ok((k.clone(), Resource::read(k, v)?)))
                .collect::<Result<_, Error>>()?,
        };
        let mut doc = Document {
            name: optional_text(o, "name", "name")?,
            start_timecode: optional_text(o, "startTimecode", "startTimecode")?,
            markers: markers(o.get("markers"), "markers")?,
            metadata: o.get("metadata").cloned(),
            resources,
            tracks: Vec::new(),
            mix: o.get("mix").cloned(),
            duration: Tick::ZERO,
            canvas,
        };
        if let Some(mix) = &doc.mix {
            object(mix, "mix")?;
        }
        let fps = doc.canvas.fps.as_f64();
        if let Some(tc) = &doc.start_timecode {
            timecode::parse(tc, fps).map_err(|e| format!("startTimecode: {e}"))?;
        }
        for (alias, r) in &doc.resources {
            if let Some(tc) = &r.timecode {
                timecode::parse(tc, r.fps.unwrap_or(fps))
                    .map_err(|e| format!("resources.{alias}.timecode: {e}"))?;
            }
        }
        let tracks = object(
            o.get("tracks").ok_or("tracks: the document has none")?,
            "tracks",
        )?;
        only(
            tracks,
            "tracks",
            &["visual", "audio", "caption", "adjustment"],
        )?;
        for kind in [Kind::Visual, Kind::Audio, Kind::Caption, Kind::Adjustment] {
            let Some(list) = tracks.get(kind.name()) else {
                continue;
            };
            let what = format!("tracks.{}", kind.name());
            for (n, t) in array(list, &what)?.iter().enumerate() {
                let track = doc.track(kind, n, t)?;
                doc.tracks.push(track);
            }
        }
        doc.anchor()?;
        for track in &mut doc.tracks {
            track.clips.sort_by_key(|c| c.start);
        }
        doc.check_tracks()?;
        doc.duration = (doc.tracks.iter())
            .flat_map(|t| t.clips.iter().map(Clip::end))
            .max()
            .unwrap_or_default();
        if doc.duration <= Tick::ZERO {
            return Err("tracks: the timeline has no clips, so it has no length".into());
        }
        Ok(doc)
    }

    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .find(|c| c.id == id)
    }

    fn track(&self, kind: Kind, n: usize, t: &Value) -> Result<Track, Error> {
        let at = format!("tracks.{}[{n}]", kind.name());
        let o = object(t, &at)?;
        let allowed: &[&str] = match kind {
            Kind::Visual => &[
                "id", "items", "clips", "gain", "pan", "effects", "sends", "output",
            ],
            Kind::Audio => &[
                "id", "items", "clips", "gain", "pan", "effects", "sends", "output",
            ],
            Kind::Caption => &["id", "clips", "style", "presenter"],
            Kind::Adjustment => &["id", "clips"],
        };
        only(o, &at, allowed)?;
        let id = match o.get("id") {
            None => format!("{}{n}", kind.name()),
            Some(v) => alias(v, &format!("{at}.id"))?,
        };
        let mut sound = Sound {
            gain: o
                .get("gain")
                .map(|v| animated(v, &format!("{at}.gain")))
                .transpose()?,
            pan: o
                .get("pan")
                .map(|v| animated(v, &format!("{at}.pan")))
                .transpose()?,
            effects: optional_array(o, "effects", &at)?,
            sends: optional_array(o, "sends", &at)?,
            output: optional_text(o, "output", &format!("{at}.output"))?,
        };
        if kind == Kind::Visual
            && (!sound.effects.is_empty() || !sound.sends.is_empty() || sound.output.is_some())
        {
            return Err(format!(
                "{at}: a visual track's sound takes gain and pan; effects, sends and output on \
                 it are not supported yet: put the sound on an audio track anchored to the clips"
            ));
        }
        if kind != Kind::Audio {
            sound.effects.clear();
        }
        let style = match o.get("style") {
            None => None,
            Some(v) => Some(object(v, &format!("{at}.style"))?.clone()),
        };
        let presenter = match o.get("presenter") {
            None => None,
            Some(v) => {
                let w = format!("{at}.presenter");
                let p = object(v, &w)?;
                only(p, &w, &["component", "props", "data", "resources"])?;
                Some(component(p, &w)?)
            }
        };
        if style.is_some() && presenter.is_some() {
            return Err(format!(
                "{at}: a presenter draws the captions its own way, so the track has no style"
            ));
        }
        let mut track = Track {
            id,
            kind,
            at: at.clone(),
            clips: Vec::new(),
            transitions: Vec::new(),
            sound,
            style,
            presenter,
        };
        match (o.get("items"), o.get("clips")) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "{at}: items (a sequence) or clips (placed by start), not both"
                ));
            }
            (Some(items), None) => self.sequence(&mut track, items)?,
            (None, Some(clips)) => {
                for (i, c) in array(clips, &format!("{at}.clips"))?.iter().enumerate() {
                    let w = format!("{at}.clips[{i}]");
                    let clip = self.clip_of(&track, c, &w, i, false)?;
                    track.clips.push(clip);
                }
            }
            (None, None) => {}
        }
        Ok(track)
    }

    /// A sequence's items: clips one after another, gaps, and transitions between two clips.
    fn sequence(&self, track: &mut Track, items: &Value) -> Result<(), Error> {
        let at = format!("{}.items", track.at);
        let fps = self.canvas.fps;
        let mut written = Tick::ZERO;
        // A transition waits for the clip after it.
        let mut pending: Option<Transition> = None;
        // Whether the item before is a clip.
        let mut after_clip = false;
        for (i, item) in array(items, &at)?.iter().enumerate() {
            let w = format!("{at}[{i}]");
            let o = object(item, &w)?;
            if let Some(gap) = o.get("gap") {
                only(o, &w, &["gap"])?;
                if pending.is_some() {
                    return Err(format!(
                        "{w}: a transition goes between two clips, not a gap"
                    ));
                }
                let gap = seconds(gap, &format!("{w}.gap"))?;
                if gap <= Tick::ZERO {
                    return Err(format!("{w}.gap: more than 0 seconds"));
                }
                written = written + gap;
                after_clip = false;
            } else if o.contains_key("transition") {
                if pending.is_some() {
                    return Err(format!(
                        "{w}: two transitions in a row; one goes between clips"
                    ));
                }
                let last = track.clips.len().checked_sub(1).filter(|_| after_clip);
                let (Some(last), Some(true)) = (last, next_is_clip(items, i)) else {
                    return Err(format!("{w}: a transition goes between two clips"));
                };
                pending = Some(self.transition(track.kind, o, &w, last)?);
                after_clip = false;
            } else {
                if o.contains_key("start") {
                    return Err(format!(
                        "{w}.start: a sequence places its clips one after another, so they have \
                         no start; use clips with start for clips placed at a time"
                    ));
                }
                let mut clip = self.clip_of(track, item, &w, i, true)?;
                let begin = fps.snap(written);
                written = written + clip.duration;
                clip.start = begin;
                clip.duration = fps.snap(written) - begin;
                if let Some(mut t) = pending.take() {
                    t.to = track.clips.len();
                    track.transitions.push(t);
                }
                track.clips.push(clip);
                after_clip = true;
            }
        }
        if pending.is_some() {
            return Err(format!(
                "{at}: the last item is a transition; it goes between clips"
            ));
        }
        Ok(())
    }

    fn transition(
        &self,
        kind: Kind,
        o: &Map<String, Value>,
        w: &str,
        from: usize,
    ) -> Result<Transition, Error> {
        only(
            o,
            w,
            &[
                "transition",
                "inOffset",
                "outOffset",
                "params",
                "component",
                "props",
                "data",
                "resources",
                "name",
                "metadata",
            ],
        )?;
        let name = text(&o["transition"], &format!("{w}.transition"))?;
        let offset = |k: &str| match o.get(k) {
            None => Ok(Tick::ZERO),
            Some(v) => {
                let t = seconds(v, &format!("{w}.{k}"))?;
                match t < Tick::ZERO {
                    true => Err(format!("{w}.{k}: 0 seconds or more")),
                    false => Ok(t),
                }
            }
        };
        let (in_offset, out_offset) = (offset("inOffset")?, offset("outOffset")?);
        if in_offset + out_offset <= Tick::ZERO {
            return Err(format!(
                "{w}: a transition plays over inOffset before the cut and outOffset after it; \
                 give either more than 0 seconds"
            ));
        }
        let params = match o.get("params") {
            None => Map::new(),
            Some(p) => object(p, &format!("{w}.params"))?.clone(),
        };
        let mut component_of = None;
        match kind {
            Kind::Audio if name != "crossfade" => {
                return Err(format!(
                    "{w}.transition: a sound transition is \"crossfade\", not {name:?}"
                ));
            }
            Kind::Audio => {
                if let Some(k) = ["params", "component", "props", "data", "resources"]
                    .iter()
                    .find(|k| o.contains_key(**k))
                {
                    return Err(format!("{w}: a crossfade has no {k}"));
                }
            }
            _ => match TRANSITIONS.iter().find(|(n, _)| *n == name) {
                None => {
                    let names: Vec<&str> = TRANSITIONS.iter().map(|(n, _)| *n).collect();
                    return Err(format!(
                        "{w}.transition: no transition {name:?}; the built-in ones are {}",
                        names.join(", ")
                    ));
                }
                Some(("motion", _)) => {
                    if o.contains_key("params") {
                        return Err(format!(
                            "{w}.params: a motion transition takes its component's props"
                        ));
                    }
                    component_of = Some(component(o, w)?);
                }
                Some((_, known)) => {
                    if let Some(k) = ["component", "props", "data", "resources"]
                        .iter()
                        .find(|k| o.contains_key(**k))
                    {
                        return Err(format!(
                            "{w}.{k}: only a motion transition has a component; {name} takes \
                             params"
                        ));
                    }
                    for (k, v) in &params {
                        let Some(&(_, _, low, high)) = known.iter().find(|p| p.0 == k) else {
                            let takes = match known.len() {
                                0 => "none".to_string(),
                                _ => (known.iter().map(|p| p.0).collect::<Vec<_>>()).join(", "),
                            };
                            return Err(format!(
                                "{w}.params: {name} has no param {k}; it takes {takes}"
                            ));
                        };
                        let x = v.as_f64().ok_or_else(|| {
                            format!("{w}.params.{k}: a number from {low} to {high}")
                        })?;
                        if !(low..=high).contains(&x) {
                            return Err(format!("{w}.params.{k}: from {low} to {high}, not {x}"));
                        }
                    }
                }
            },
        }
        Ok(Transition {
            at: w.to_string(),
            from,
            to: from + 1,
            kind: name.to_string(),
            in_offset,
            out_offset,
            params,
            component: component_of,
            name: optional_text(o, "name", &format!("{w}.name"))?,
            metadata: o.get("metadata").cloned(),
        })
    }

    fn clip_of(
        &self,
        track: &Track,
        v: &Value,
        w: &str,
        i: usize,
        in_sequence: bool,
    ) -> Result<Clip, Error> {
        let o = object(v, w)?;
        let common = [
            "id", "name", "markers", "metadata", "enabled", "start", "duration",
        ];
        let kind = match track.kind {
            Kind::Visual | Kind::Adjustment => Some(text(
                o.get("kind")
                    .ok_or(format!("{w}.kind: the clip has none"))?,
                &format!("{w}.kind"),
            )?),
            _ => None,
        };
        let look = [
            "position", "size", "scale", "rotation", "anchor", "opacity", "blend", "fit", "effects",
        ];
        let timing = ["trimStart", "rate", "timeMap", "end"];
        let mut allowed: Vec<&str> = common.to_vec();
        match (track.kind, kind) {
            (Kind::Visual, Some(k)) => {
                allowed.push("kind");
                allowed.extend(look);
                match k {
                    "solid" => allowed.push("color"),
                    "image" => allowed.push("src"),
                    "video" => allowed.extend(["src", "gain", "pan"].iter().chain(&timing)),
                    "motion" => allowed.extend([
                        "component",
                        "props",
                        "data",
                        "resources",
                        "trimStart",
                        "rate",
                        "end",
                    ]),
                    "timeline" => allowed.extend(["src", "gain", "pan"].iter().chain(&timing)),
                    _ => {
                        return Err(format!(
                            "{w}.kind: a visual clip is solid, image, video, motion or timeline, \
                             not {k:?}"
                        ));
                    }
                }
            }
            (Kind::Audio, _) => allowed.extend(["src", "gain", "pan"].iter().chain(&timing)),
            (Kind::Caption, _) => allowed.extend(["text", "runs"]),
            (Kind::Adjustment, Some(k)) => {
                allowed.push("kind");
                match k {
                    "color-grade" => allowed.push("temperature"),
                    "motion" => allowed.extend(["component", "props", "data", "resources"]),
                    _ => {
                        return Err(format!(
                            "{w}.kind: an adjustment is color-grade or motion, not {k:?}"
                        ));
                    }
                }
            }
            _ => unreachable!("visual and adjustment clips have a kind"),
        }
        only(o, w, &allowed)?;
        let id = match o.get("id") {
            Some(v) => Some(alias(v, &format!("{w}.id"))?),
            None if in_sequence => {
                return Err(format!(
                    "{w}.id: a clip in a sequence has an id, which anchors and edits name"
                ));
            }
            None => None,
        };
        let duration = seconds(
            o.get("duration")
                .ok_or(format!("{w}.duration: the clip has none"))?,
            &format!("{w}.duration"),
        )?;
        if duration <= Tick::ZERO {
            return Err(format!("{w}.duration: more than 0 seconds"));
        }
        let (start, anchor) = match o.get("start") {
            None if in_sequence => (Tick::ZERO, None),
            None => return Err(format!("{w}.start: a placed clip has a start")),
            Some(Value::Object(a)) => {
                let aw = format!("{w}.start");
                only(a, &aw, &["clip", "offset"])?;
                let to = text(a.get("clip").ok_or(format!("{aw}.clip: none"))?, &aw)?;
                let offset = match a.get("offset") {
                    None => Tick::ZERO,
                    Some(v) => seconds(v, &format!("{aw}.offset"))?,
                };
                (Tick::ZERO, Some((to.to_string(), offset)))
            }
            Some(v) => {
                let s = seconds(v, &format!("{w}.start"))?;
                if s < Tick::ZERO {
                    return Err(format!("{w}.start: 0 seconds or more"));
                }
                (s, None)
            }
        };
        let src = || -> Result<String, Error> {
            let v = o.get("src").ok_or(format!("{w}.src: the clip has none"))?;
            alias(v, &format!("{w}.src"))
        };
        let content = match (track.kind, kind) {
            (Kind::Visual, Some("solid")) => Content::Solid {
                color: text(
                    o.get("color").ok_or(format!("{w}.color: none"))?,
                    &format!("{w}.color"),
                )?
                .to_string(),
            },
            (Kind::Visual, Some("image")) => Content::Image { src: src()? },
            (Kind::Visual, Some("video")) => Content::Video { src: src()? },
            (Kind::Visual | Kind::Adjustment, Some("motion")) => Content::Motion(component(o, w)?),
            (Kind::Visual, Some("timeline")) => Content::Timeline { src: src()? },
            (Kind::Audio, _) => Content::Audio { src: src()? },
            (Kind::Caption, _) => caption(o, w, duration)?,
            (Kind::Adjustment, Some("color-grade")) => {
                let t = match o.get("temperature") {
                    None => 0.0,
                    Some(v) => v.as_f64().ok_or(format!("{w}.temperature: a number"))?,
                };
                if !(-1.0..=1.0).contains(&t) {
                    return Err(format!("{w}.temperature: from -1 to 1, not {t}"));
                }
                Content::Grade { temperature: t }
            }
            _ => unreachable!("kinds checked above"),
        };
        let source = source(o, w)?;
        let mut look = Look::default();
        for (k, slot) in [
            ("position", &mut look.position),
            ("size", &mut look.size),
            ("scale", &mut look.scale),
            ("rotation", &mut look.rotation),
            ("opacity", &mut look.opacity),
        ] {
            if let Some(v) = o.get(k) {
                *slot = Some(animated(v, &format!("{w}.{k}"))?);
            }
        }
        if let Some(v) = o.get("anchor") {
            look.anchor = Some(pair(v, &format!("{w}.anchor"))?);
        }
        if let Some(v) = o.get("blend") {
            let b = text(v, &format!("{w}.blend"))?;
            if !BLENDS.contains(&b) {
                return Err(format!(
                    "{w}.blend: one of {}, not {b:?}",
                    BLENDS.join(", ")
                ));
            }
            look.blend = Some(b.to_string());
        }
        if let Some(v) = o.get("fit") {
            look.fit = Some(match text(v, &format!("{w}.fit"))? {
                "contain" => Fit::Contain,
                "cover" => Fit::Cover,
                "fill" => Fit::Fill,
                "none" => Fit::None,
                f => {
                    return Err(format!("{w}.fit: contain, cover, fill or none, not {f:?}"));
                }
            });
        }
        if let Some(v) = o.get("effects") {
            for (k, e) in array(v, &format!("{w}.effects"))?.iter().enumerate() {
                let ew = format!("{w}.effects[{k}]");
                let eo = object(e, &ew)?;
                only(
                    eo,
                    &ew,
                    &["kind", "component", "props", "data", "resources"],
                )?;
                match eo
                    .get("kind")
                    .map(|k| text(k, &format!("{ew}.kind")))
                    .transpose()?
                {
                    Some("motion") => look.effects.push(component(eo, &ew)?),
                    other => {
                        return Err(format!(
                            "{ew}.kind: a clip effect is {{ \"kind\": \"motion\", \"component\" }}, \
                             not {other:?}"
                        ));
                    }
                }
            }
        }
        let sound = Sound {
            gain: o
                .get("gain")
                .map(|v| animated(v, &format!("{w}.gain")))
                .transpose()?,
            pan: o
                .get("pan")
                .map(|v| animated(v, &format!("{w}.pan")))
                .transpose()?,
            ..Sound::default()
        };
        let enabled = match o.get("enabled") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(format!("{w}.enabled: true or false")),
        };
        let named = id.is_some();
        Ok(Clip {
            id: id.unwrap_or_else(|| format!("{}.{}", track.id, i + 1)),
            named,
            at: w.to_string(),
            name: optional_text(o, "name", &format!("{w}.name"))?,
            markers: markers(o.get("markers"), &format!("{w}.markers"))?,
            metadata: o.get("metadata").cloned(),
            enabled,
            start,
            duration,
            anchor,
            content,
            source,
            look,
            sound,
        })
    }

    /// Places anchored clips after the clips they hang on.
    fn anchor(&mut self) -> Result<(), Error> {
        let mut ids: HashMap<String, (usize, usize)> = HashMap::new();
        for (t, track) in self.tracks.iter().enumerate() {
            for (c, clip) in track.clips.iter().enumerate() {
                if ids.insert(clip.id.clone(), (t, c)).is_some() {
                    return Err(format!(
                        "{}.id: another clip has the id {:?}; ids are unique in a document",
                        clip.at, clip.id
                    ));
                }
            }
        }
        let fps = self.canvas.fps;
        let mut placed: HashMap<(usize, usize), bool> = HashMap::new();
        fn place(
            doc: &mut Document,
            ids: &HashMap<String, (usize, usize)>,
            placed: &mut HashMap<(usize, usize), bool>,
            at: (usize, usize),
        ) -> Result<(), Error> {
            match placed.get(&at) {
                Some(true) => return Ok(()),
                Some(false) => {
                    let clip = &doc.tracks[at.0].clips[at.1];
                    return Err(format!(
                        "{}.start: anchors go round in a loop through {:?}",
                        clip.at, clip.id
                    ));
                }
                None => {}
            }
            let Some((to, offset)) = doc.tracks[at.0].clips[at.1].anchor.clone() else {
                placed.insert(at, true);
                return Ok(());
            };
            placed.insert(at, false);
            let Some(&target) = ids.get(&to) else {
                let clip = &doc.tracks[at.0].clips[at.1];
                return Err(format!("{}.start.clip: no clip has the id {to:?}", clip.at));
            };
            place(doc, ids, placed, target)?;
            let base = doc.tracks[target.0].clips[target.1].start;
            let clip = &mut doc.tracks[at.0].clips[at.1];
            let start = base + offset;
            if start < Tick::ZERO {
                return Err(format!(
                    "{}.start: the anchor puts the clip before the timeline's start",
                    clip.at
                ));
            }
            clip.start = start;
            placed.insert(at, true);
            Ok(())
        }
        let all: Vec<(usize, usize)> = ids.values().copied().collect();
        for at in all {
            place(self, &ids, &mut placed, at)?;
        }
        // Placed clips' cuts on the frames, now that anchors have moved them.
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                let start = fps.snap(clip.start);
                let end = fps.snap(clip.start + clip.duration);
                clip.start = start;
                clip.duration = end - start;
                if clip.duration <= Tick::ZERO {
                    return Err(format!("{}.duration: shorter than a frame", clip.at));
                }
            }
        }
        Ok(())
    }

    /// No two clips of a track overlap, except over a transition between them; transitions fit
    /// their clips.
    fn check_tracks(&self) -> Result<(), Error> {
        for track in &self.tracks {
            for pair in track.clips.windows(2) {
                if pair[1].start < pair[0].end() {
                    return Err(format!(
                        "{}: it overlaps {} on its track; put one on another track",
                        pair[1].at, pair[0].at
                    ));
                }
            }
            for t in &track.transitions {
                let (a, b) = (&track.clips[t.from], &track.clips[t.to]);
                if t.in_offset > a.duration || t.out_offset > b.duration {
                    return Err(format!(
                        "{}: the transition is longer than its clips: inOffset is at most {} s, \
                         outOffset at most {} s",
                        t.at,
                        a.duration.seconds_text(),
                        b.duration.seconds_text()
                    ));
                }
                if !a.enabled || !b.enabled {
                    return Err(format!("{}: a transition goes between enabled clips", t.at));
                }
            }
        }
        Ok(())
    }
}

/// Whether the item after `i` in `items` is a clip, not a gap or a transition.
fn next_is_clip(items: &Value, i: usize) -> Option<bool> {
    let next = items.as_array()?.get(i + 1)?.as_object()?;
    Some(!next.contains_key("gap") && !next.contains_key("transition"))
}

impl Canvas {
    fn read(v: &Value) -> Result<Canvas, Error> {
        let o = object(v, "canvas")?;
        only(o, "canvas", &["width", "height", "fps", "background"])?;
        let side = |k: &str| -> Result<u32, Error> {
            let n = o
                .get(k)
                .and_then(Value::as_u64)
                .filter(|n| (1..=16384).contains(n));
            n.map(|n| n as u32)
                .ok_or_else(|| format!("canvas.{k}: a whole number of px from 1 to 16384"))
        };
        let fps = match o.get("fps") {
            Some(Value::Number(n)) => Rate::parse(&n.to_string()),
            Some(Value::String(s)) => Rate::parse(s),
            _ => Err("a number or \"30000/1001\"".into()),
        }
        .map_err(|e| format!("canvas.fps: {e}"))?;
        Ok(Canvas {
            width: side("width")?,
            height: side("height")?,
            fps,
            background: optional_text(o, "background", "canvas.background")?
                .unwrap_or_else(|| "#000000ff".into()),
        })
    }
}

impl Resource {
    fn read(name: &str, v: &Value) -> Result<Resource, Error> {
        let w = format!("resources.{name}");
        alias(&Value::String(name.into()), "resources")?;
        let path = |v: &Value, w: &str| -> Result<String, Error> {
            let p = text(v, w)?;
            if p.starts_with("file://") {
                return Err(format!("{w}: write a path, not a file:// address"));
            }
            if p.is_empty() {
                return Err(format!("{w}: an empty path"));
            }
            Ok(p.to_string())
        };
        let mut r = Resource {
            files: Vec::new(),
            timecode: None,
            fps: None,
            duration: None,
            name: None,
        };
        match v {
            Value::String(_) => r.files.push(path(v, &w)?),
            Value::Array(files) => {
                // A font family's faces.
                for (i, f) in files.iter().enumerate() {
                    r.files.push(path(f, &format!("{w}[{i}]"))?);
                }
                if r.files.is_empty() {
                    return Err(format!("{w}: a font family has its files"));
                }
            }
            Value::Object(o) => {
                only(o, &w, &["path", "timecode", "fps", "duration", "name"])?;
                r.files
                    .push(path(o.get("path").ok_or(format!("{w}.path: none"))?, &w)?);
                r.timecode = optional_text(o, "timecode", &format!("{w}.timecode"))?;
                r.name = optional_text(o, "name", &format!("{w}.name"))?;
                let number = |k: &str| match o.get(k) {
                    None => Ok(None),
                    Some(v) => (v.as_f64().filter(|x| *x > 0.0))
                        .map(Some)
                        .ok_or_else(|| format!("{w}.{k}: a number above 0")),
                };
                r.fps = number("fps")?;
                r.duration = number("duration")?;
            }
            _ => {
                return Err(format!(
                    "{w}: a path, a list of a font family's files, or {{ path, timecode, fps, \
                     duration, name }}"
                ));
            }
        }
        Ok(r)
    }
}

fn source(o: &Map<String, Value>, w: &str) -> Result<Source, Error> {
    let end = match o.get("end") {
        None => End::Error,
        Some(v) => match text(v, &format!("{w}.end"))? {
            "error" => End::Error,
            "hold" => End::Hold,
            "loop" => End::Loop,
            e => return Err(format!("{w}.end: error, hold or loop, not {e:?}")),
        },
    };
    if let Some(map) = o.get("timeMap") {
        if o.contains_key("trimStart") || o.contains_key("rate") {
            return Err(format!(
                "{w}.timeMap: a time map replaces trimStart and rate; give one or the other"
            ));
        }
        let mw = format!("{w}.timeMap");
        let mut points = Vec::new();
        for (i, p) in array(map, &mw)?.iter().enumerate() {
            let pw = format!("{mw}[{i}]");
            let [x, y] = array(p, &pw)?.as_slice() else {
                return Err(format!("{pw}: [clip time, source time]"));
            };
            let (x, y) = (seconds(x, &pw)?, seconds(y, &pw)?);
            if y < Tick::ZERO {
                return Err(format!("{pw}: a source time of 0 seconds or more"));
            }
            if points.last().is_some_and(|&(l, _)| x <= l) {
                return Err(format!("{pw}: clip times rise from point to point"));
            }
            points.push((x, y));
        }
        if points.len() < 2 {
            return Err(format!("{mw}: two points or more"));
        }
        return Ok(Source {
            time: SourceTime::Map(points),
            end,
        });
    }
    let trim = match o.get("trimStart") {
        None => Tick::ZERO,
        Some(v) => seconds(v, &format!("{w}.trimStart"))?,
    };
    if trim < Tick::ZERO {
        return Err(format!("{w}.trimStart: 0 seconds or more"));
    }
    let rate = match o.get("rate") {
        None => 1.0,
        Some(v) => v.as_f64().ok_or(format!("{w}.rate: a number above 0"))?,
    };
    if rate.is_nan() || rate <= 0.0 {
        return Err(format!(
            "{w}.rate: above 0, not {rate}; a time map plays backwards or holds"
        ));
    }
    Ok(Source {
        time: SourceTime::Linear {
            trim,
            rate: Speed::from_f64(rate).map_err(|e| format!("{w}.rate: {e}"))?,
        },
        end,
    })
}

fn caption(o: &Map<String, Value>, w: &str, duration: Tick) -> Result<Content, Error> {
    match (o.get("text"), o.get("runs")) {
        (Some(t), None) => Ok(Content::Caption {
            text: text(t, &format!("{w}.text"))?.to_string(),
            runs: Vec::new(),
        }),
        (None, Some(runs)) => {
            let rw = format!("{w}.runs");
            let mut out: Vec<Run> = Vec::new();
            for (i, r) in array(runs, &rw)?.iter().enumerate() {
                let iw = format!("{rw}[{i}]");
                let ro = object(r, &iw)?;
                if ro.contains_key("fontSize") || ro.contains_key("color") {
                    return Err(format!(
                        "{iw}: a run has text, start and end; style words with a presenter"
                    ));
                }
                only(ro, &iw, &["text", "start", "end"])?;
                let get = |k: &str| ro.get(k).ok_or(format!("{iw}.{k}: the run has none"));
                let run = Run {
                    text: text(get("text")?, &format!("{iw}.text"))?.to_string(),
                    start: seconds(get("start")?, &format!("{iw}.start"))?,
                    end: seconds(get("end")?, &format!("{iw}.end"))?,
                };
                let after = out.last().map_or(Tick::ZERO, |l| l.end);
                if run.start < after || run.end <= run.start || run.end > duration {
                    return Err(format!(
                        "{iw}: runs are in order, each from its start to a later end, within the \
                         caption's {} s",
                        duration.seconds_text()
                    ));
                }
                out.push(run);
            }
            if out.is_empty() {
                return Err(format!("{rw}: a caption has a run or more"));
            }
            Ok(Content::Caption {
                text: out.iter().map(|r| r.text.as_str()).collect(),
                runs: out,
            })
        }
        _ => Err(format!("{w}: a caption has text or runs, one of them")),
    }
}

/// A component and what it is given, from `component`, `props`, `data` and `resources`.
fn component(o: &Map<String, Value>, w: &str) -> Result<Component, Error> {
    let name = alias(
        o.get("component").ok_or(format!("{w}.component: none"))?,
        &format!("{w}.component"),
    )?;
    let mut props = Vec::new();
    if let Some(p) = o.get("props") {
        for (k, v) in object(p, &format!("{w}.props"))? {
            props.push((k.clone(), animated(v, &format!("{w}.props.{k}"))?));
        }
    }
    let data = match o.get("data") {
        None => None,
        Some(d) => Some(object(d, &format!("{w}.data"))?.clone()),
    };
    let mut resources = BTreeMap::new();
    if let Some(r) = o.get("resources") {
        for (k, v) in object(r, &format!("{w}.resources"))? {
            resources.insert(k.clone(), alias(v, &format!("{w}.resources.{k}"))?);
        }
    }
    Ok(Component {
        component: name,
        props,
        data,
        resources,
    })
}

/// A constant, or `{ keyframes: [[t, value, easing?], …], interpolation }`.
fn animated(v: &Value, w: &str) -> Result<Animated, Error> {
    let Some(o) = v.as_object().filter(|o| o.contains_key("keyframes")) else {
        return Ok(Animated::Constant(v.clone()));
    };
    only(o, w, &["keyframes", "interpolation"])?;
    let step = match o
        .get("interpolation")
        .map(|i| text(i, &format!("{w}.interpolation")))
    {
        None | Some(Ok("linear")) => false,
        Some(Ok("step")) => true,
        Some(Ok(i)) => return Err(format!("{w}.interpolation: linear or step, not {i:?}")),
        Some(Err(e)) => return Err(e),
    };
    let kw = format!("{w}.keyframes");
    let list = array(&o["keyframes"], &kw)?;
    let mut keys: Vec<Key> = Vec::new();
    for (i, k) in list.iter().enumerate() {
        let iw = format!("{kw}[{i}]");
        let parts = array(k, &iw)?;
        let (at, value, easing) = match parts.as_slice() {
            [at, value] => (at, value, None),
            [at, value, easing] => (at, value, Some(text(easing, &format!("{iw}[2]"))?)),
            _ => return Err(format!("{iw}: [time, value] or [time, value, easing]")),
        };
        let at = seconds(at, &format!("{iw}[0]"))?;
        if keys.last().is_some_and(|l| at <= l.at) || at < Tick::ZERO {
            return Err(format!(
                "{iw}: keyframe times rise strictly from 0 seconds or more"
            ));
        }
        if easing.is_some() && i + 1 == list.len() {
            return Err(format!(
                "{iw}: an easing is of the stretch leaving a keyframe, so the last has none"
            ));
        }
        keys.push(Key {
            at,
            value: value.clone(),
            easing: easing.map(String::from),
        });
    }
    if keys.is_empty() {
        return Err(format!("{kw}: one keyframe or more"));
    }
    Ok(Animated::Keys { keys, step })
}

fn markers(v: Option<&Value>, w: &str) -> Result<Vec<Value>, Error> {
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    let list = array(v, w)?;
    for (i, m) in list.iter().enumerate() {
        let mw = format!("{w}[{i}]");
        let o = object(m, &mw)?;
        only(
            o,
            &mw,
            &["at", "duration", "name", "color", "comment", "metadata"],
        )?;
        seconds(
            o.get("at").ok_or(format!("{mw}.at: none"))?,
            &format!("{mw}.at"),
        )?;
        if let Some(d) = o.get("duration") {
            seconds(d, &format!("{mw}.duration"))?;
        }
    }
    Ok(list.clone())
}

/// Seconds, read exactly as the decimal written.
pub fn seconds(v: &Value, w: &str) -> Result<Tick, Error> {
    match v {
        Value::Number(n) => Tick::parse_seconds(&n.to_string()).map_err(|e| format!("{w}: {e}")),
        Value::String(_) => Err(format!(
            "{w}: times are seconds, a number; a timecode is only a label (startTimecode, a \
             resource's timecode)"
        )),
        _ => Err(format!("{w}: seconds, a number")),
    }
}

fn pair(v: &Value, w: &str) -> Result<[f64; 2], Error> {
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(format!("{w}: [x, y], two numbers")),
        },
        _ => Err(format!("{w}: [x, y], two numbers")),
    }
}

/// An id or a resource alias: 1–64 ASCII letters, digits, dots, underscores or hyphens.
fn alias(v: &Value, w: &str) -> Result<String, Error> {
    let s = text(v, w)?;
    let ok = (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    match ok {
        true => Ok(s.to_string()),
        false => Err(format!(
            "{w}: {s:?}: names are 1–64 ASCII letters, digits, dots, underscores or hyphens"
        )),
    }
}

fn object<'a>(v: &'a Value, w: &str) -> Result<&'a Map<String, Value>, Error> {
    v.as_object().ok_or_else(|| format!("{w}: an object"))
}

fn array<'a>(v: &'a Value, w: &str) -> Result<&'a Vec<Value>, Error> {
    v.as_array().ok_or_else(|| format!("{w}: a list"))
}

fn text<'a>(v: &'a Value, w: &str) -> Result<&'a str, Error> {
    v.as_str().ok_or_else(|| format!("{w}: a string"))
}

fn optional_text(o: &Map<String, Value>, k: &str, w: &str) -> Result<Option<String>, Error> {
    o.get(k).map(|v| text(v, w).map(String::from)).transpose()
}

fn optional_array(o: &Map<String, Value>, k: &str, at: &str) -> Result<Vec<Value>, Error> {
    match o.get(k) {
        None => Ok(Vec::new()),
        Some(v) => array(v, &format!("{at}.{k}")).cloned(),
    }
}

/// Errors on a field of `o` not in `allowed`.
fn only(o: &Map<String, Value>, w: &str, allowed: &[&str]) -> Result<(), Error> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        None => Ok(()),
        Some(k) => Err(format!(
            "{w}: no field {k:?}; it takes {}",
            allowed.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests;
