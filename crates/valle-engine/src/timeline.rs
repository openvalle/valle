//! A Timeline document lowered to the Motion scene it plays as, so an edit runs through the same
//! frontend, engine and renderer as any scene, on one clock. The scene lives in memory only.
//!
//! - A clip is a `<TimeScope clip>`: a box that shows while the clip is on, the clip's content
//!   inside on the clip's own time. When it is on and what time it gives come from the clip
//!   table, in ticks, never from JS: clips coming and going are not guards and trace nothing
//!   again.
//! - Later tracks paint over earlier ones.
//! - Each video clip reads its file as a resource of its own, so two clips of one file decode
//!   apart; a Motion component's resource slots are resources of the scene, which the names
//!   inside the component stand for.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::rc::Rc;

use serde_json::{Map, Value};
use valle_timeline::{Animated, Clip, Component, Content, Document, End, Fit, SourceTime, Track};

use crate::front::controls::Kind;
use crate::front::{self, Declared, Holds, Inputs, Role};
use crate::template::{self, ClipTime, Clips, Error};
use crate::time::{Speed, Tick};

/// The scene a document lowers to: its path, beside the document, the sources of the modules
/// made for it by path, and what its controls are bound to, the clip table among them.
pub struct Lowered {
    pub entry: String,
    pub files: HashMap<String, String>,
    pub inputs: Inputs,
}

/// What lowering reads from the host: a file's text, and a video file's length.
pub struct Host<'a> {
    pub read: &'a dyn Fn(&str) -> Option<String>,
    pub length: &'a dyn Fn(&str) -> Option<Tick>,
}

/// The scene the document at `path` (`/`-separated) plays as.
pub fn lower(path: &str, doc: &Document, host: &Host<'_>) -> Result<Lowered, Error> {
    let folder = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut l = Lowering {
        doc,
        folder,
        host,
        slots: Vec::new(),
        bound: BTreeMap::new(),
        imports: Vec::new(),
        declared: HashMap::new(),
        clips: Clips::new(),
        functions: Vec::new(),
    };
    let [w, h] = [doc.canvas.width, doc.canvas.height];
    let fill = format!("{{ position: \"absolute\", left: 0, top: 0, width: {w}, height: {h} }}");
    let mut body = String::new();
    for track in &doc.tracks {
        match track.kind {
            valle_timeline::Kind::Visual => {}
            // Sound is the mix's.
            valle_timeline::Kind::Audio => continue,
            _ => {
                return Err(format!(
                    "{}: {} tracks are not supported yet",
                    track.at,
                    track.kind.name()
                ));
            }
        }
        if let Some(t) = track.transitions.first() {
            return Err(format!("{}: transitions are not supported yet", t.at));
        }
        writeln!(body, "      <View key={} style={{{fill}}}>", js(&track.id)).unwrap();
        for clip in track.clips.iter().filter(|c| c.enabled) {
            let shown = l.clip(track, clip, &fill)?;
            body.push_str(&shown);
        }
        body.push_str("      </View>\n");
    }

    let mut code = String::from("// The scene a Timeline document plays as.\n");
    for (name, file) in &l.imports {
        writeln!(
            code,
            "import {name} from {};",
            js(&import_key(folder, file))
        )
        .unwrap();
    }
    let slots: Vec<String> = (l.slots.iter())
        .map(|(name, kind)| format!("{}: resource({{ kind: {} }})", js(name), js(&kind.name())))
        .collect();
    for f in &l.functions {
        write!(code, "\n{f}").unwrap();
    }
    write!(
        code,
        "
export const composition = {{ width: {w}, height: {h}, fps: {fps}, duration: {duration} }};

export const controls = {{ resources: {{ {slots} }} }};

export default function Timeline() {{
  return (
    <View style={{{{ position: \"relative\", width: {w}, height: {h}, overflow: \"hidden\", backgroundColor: {background} }}}}>
{body}    </View>
  );
}}
",
        fps = js(&doc.canvas.fps.to_string()),
        duration = doc.duration.seconds_text(),
        slots = slots.join(", "),
        background = js(&doc.canvas.background),
    )
    .unwrap();
    let entry = format!("{path}.tsx");
    let inputs = Inputs {
        resources: l.bound,
        clips: Rc::new(l.clips),
        ..Inputs::default()
    };
    Ok(Lowered {
        files: HashMap::from([(entry.clone(), code)]),
        entry,
        inputs,
    })
}

struct Lowering<'a> {
    doc: &'a Document,
    folder: &'a str,
    host: &'a Host<'a>,
    /// The scene's resources, `r0`, `r1`…, and what each holds.
    slots: Vec<(String, Holds)>,
    /// The files bound to each.
    bound: BTreeMap<String, Vec<String>>,
    /// The components imported, by the name the scene gives each, and their files.
    imports: Vec<(String, String)>,
    declared: HashMap<String, Rc<Declared>>,
    clips: Clips,
    /// The components that draw the clips.
    functions: Vec<String>,
}

impl Lowering<'_> {
    /// A new resource of the scene holding `kind`, bound to `files`.
    fn resource(&mut self, kind: Holds, files: Vec<String>) -> String {
        let name = format!("r{}", self.slots.len());
        self.slots.push((name.clone(), kind));
        self.bound.insert(name.clone(), files);
        name
    }

    /// The files of the document's resource `alias`, as `read` takes them.
    fn files(&self, alias: &str, at: &str) -> Result<Vec<String>, Error> {
        let r = (self.doc.resources.get(alias))
            .ok_or_else(|| format!("{at}: no resource {alias:?} in resources"))?;
        Ok(r.files.iter().map(|f| self.path(f)).collect())
    }

    /// A path of the document, as `read` takes it: joined to its folder.
    fn path(&self, file: &str) -> String {
        match file.starts_with('/') || front::web(file) {
            true => file.to_string(),
            false => front::join(self.folder, file),
        }
    }

    /// The single file of resource `alias`, which is not a font family.
    fn file(&self, alias: &str, at: &str) -> Result<String, Error> {
        match self.files(alias, at)?.as_slice() {
            [one] => Ok(one.clone()),
            _ => Err(format!(
                "{at}: resource {alias:?} is a font family's files; the clip shows one file"
            )),
        }
    }

    /// What component `alias` declares, and the name the scene imports it by.
    fn component(&mut self, alias: &str, at: &str) -> Result<(Rc<Declared>, String), Error> {
        let file = self.file(alias, at)?;
        if !file.ends_with(".tsx") {
            return Err(format!(
                "{at}: resource {alias:?} is {file}, not a Motion component (.motion.tsx)"
            ));
        }
        if let Some(d) = self.declared.get(&file) {
            let name = (self.imports.iter())
                .find(|(_, f)| *f == file)
                .map(|(n, _)| n.clone())
                .expect("imported with it");
            return Ok((d.clone(), name));
        }
        let d = Rc::new(front::declared(&file, self.host.read)?);
        let name = format!("C{}", self.imports.len());
        self.imports.push((name.clone(), file.clone()));
        self.declared.insert(file, d.clone());
        Ok((d, name))
    }

    /// A visual clip's `<TimeScope clip>`, and the component it shows that draws its box and
    /// what is in it, on the clip's time.
    fn clip(&mut self, track: &Track, clip: &Clip, fill: &str) -> Result<String, Error> {
        let at = clip.at.as_str();
        if !clip.look.effects.is_empty() {
            return Err(format!("{at}.effects: clip effects are not supported yet"));
        }
        let fit = clip.look.fit.unwrap_or(Fit::Contain);
        let mut scope = Scope::default();
        let media = |r: &str, tag: &str| {
            format!(
                "<{tag} src={} style={{{{ position: \"absolute\", inset: 0, width: \"100%\", height: \"100%\", objectFit: {} }}}} />",
                js(&format!("resource://{r}")),
                js(&fit.css())
            )
        };
        let content = match &clip.content {
            Content::Solid { color } => {
                self.color(color, &format!("{at}.color"))?;
                format!(
                    "<View style={{{{ position: \"absolute\", inset: 0, backgroundColor: {} }}}} />",
                    js(color)
                )
            }
            Content::Image { src } => {
                let file = self.file(src, &format!("{at}.src"))?;
                let r = self.resource(Holds::Image, vec![file]);
                media(&r, "Image")
            }
            Content::Video { src } => {
                let file = self.file(src, &format!("{at}.src"))?;
                let length = (self.host.length)(&file);
                scope.time = Some(source_time(clip, length, &file)?);
                let r = self.resource(Holds::Video, vec![file]);
                media(&r, "Video")
            }
            Content::Motion(c) => self.motion(clip, c, &mut scope)?,
            Content::Timeline { .. } => {
                return Err(format!("{at}: nested timelines are not supported yet"));
            }
            Content::Audio { .. } | Content::Caption { .. } | Content::Grade { .. } => {
                unreachable!("not on a visual track: {}", track.at)
            }
        };
        let id = clip.id.clone();
        let time = scope.time.take().unwrap_or(ClipTime::Source {
            trim: Tick::ZERO,
            rate: Speed::from_f64(1.0)?,
            map: Vec::new(),
            length: None,
        });
        self.clips.insert(
            id.clone(),
            Rc::new(template::Clip {
                start: clip.start,
                end: clip.end(),
                time,
            }),
        );
        let shows = format!("K{}", self.functions.len());
        let body = self.look(clip, &scope.lets, &content)?;
        self.functions.push(format!(
            "function {shows}() {{\n  const t = useCurrentTime().clipSeconds;\n{body}}}\n"
        ));
        let mut props = vec![
            format!("key={}", js(&id)),
            format!("clip={}", js(&id)),
            format!("style={{{fill}}}"),
        ];
        if let Some(c) = scope.composition {
            props.push(format!("composition={{{c}}}"));
        }
        if !scope.names.is_empty() {
            props.push(format!("resources={{{}}}", js(&scope.names)));
        }
        Ok(format!(
            "        <TimeScope {}>\n          <{shows} />\n        </TimeScope>\n",
            props.join(" ")
        ))
    }

    /// The body of a clip's component: its box where `position`, `size`, `anchor`, `scale` and
    /// `rotation` put it, at its `opacity` and `blend`, and `content` in it. `t` is the clip's
    /// time, which keyframes are over.
    fn look(&self, clip: &Clip, lets: &str, content: &str) -> Result<String, Error> {
        let at = clip.at.as_str();
        let l = &clip.look;
        let [w, h] = [self.doc.canvas.width as f64, self.doc.canvas.height as f64];
        let pair = |v: &Option<Animated>, k: &str, default: [f64; 2], positive: bool| match v {
            None => Ok(format!("[{}, {}]", num(default[0]), num(default[1]))),
            Some(v) => curve(v, &format!("{at}.{k}"), &|v, w| {
                let [a, b] = pair_of(v, w)?;
                if positive && (a <= 0.0 || b <= 0.0) {
                    return Err(format!("{w}: two numbers above 0"));
                }
                Ok(Js::Pair(a, b))
            }),
        };
        let number = |v: &Option<Animated>, k: &str, default: f64, range: Option<[f64; 2]>| match v
        {
            None => Ok(num(default)),
            Some(v) => curve(v, &format!("{at}.{k}"), &|v, w| {
                let x = v.as_f64().ok_or_else(|| format!("{w}: a number"))?;
                match range {
                    Some([lo, hi]) if !(lo..=hi).contains(&x) => {
                        Err(format!("{w}: from {lo} to {hi}, not {x}"))
                    }
                    _ => Ok(Js::Number(x)),
                }
            }),
        };
        let [ax, ay] = l.anchor.unwrap_or([0.5, 0.5]);
        let mut out = String::new();
        writeln!(
            out,
            "  const size = {};",
            pair(&l.size, "size", [w, h], true)?
        )
        .unwrap();
        writeln!(
            out,
            "  const position = {};",
            pair(&l.position, "position", [w / 2.0, h / 2.0], false)?
        )
        .unwrap();
        out.push_str(lets);
        let mut style = vec![
            "position: \"absolute\"".to_string(),
            format!("left: position[0] - {} * size[0]", num(ax)),
            format!("top: position[1] - {} * size[1]", num(ay)),
            "width: size[0]".into(),
            "height: size[1]".into(),
        ];
        if l.scale.is_some() || l.rotation.is_some() {
            writeln!(
                out,
                "  const scale = {};",
                pair(&l.scale, "scale", [1.0, 1.0], false)?
            )
            .unwrap();
            writeln!(
                out,
                "  const rotation = {};",
                number(&l.rotation, "rotation", 0.0, None)?
            )
            .unwrap();
            style.push(format!(
                "transformOrigin: \"{}% {}%\"",
                num(ax * 100.0),
                num(ay * 100.0)
            ));
            style
                .push("transform: `rotate(${rotation}deg) scale(${scale[0]}, ${scale[1]})`".into());
        }
        if l.opacity.is_some() {
            let o = number(&l.opacity, "opacity", 1.0, Some([0.0, 1.0]))?;
            style.push(format!("opacity: {o}"));
        }
        if let Some(b) = &l.blend {
            style.push(format!("mixBlendMode: {}", js(b)));
        }
        writeln!(
            out,
            "  return (\n    <View style={{{{ {} }}}}>\n      {content}\n    </View>\n  );",
            style.join(", ")
        )
        .unwrap();
        Ok(out)
    }

    /// A color written in the document, checked.
    fn color(&self, color: &str, at: &str) -> Result<(), Error> {
        crate::style::color::parse(color)
            .map(drop)
            .map_err(|e| format!("{at}: {e}"))
    }

    /// A motion clip's component, given its props, data and resources, and fitted into the
    /// clip's box, or laid out at its size when it is an overlay.
    fn motion(&mut self, clip: &Clip, c: &Component, scope: &mut Scope) -> Result<String, Error> {
        let at = clip.at.as_str();
        let (d, name) = self.component(&c.component, &format!("{at}.component"))?;
        let props = self.given(c, &d, at, None, scope)?;
        let composition = d.composition;
        match d.role {
            Role::Clip => {
                let length = Some(composition.duration);
                scope.time = Some(source_time(clip, length, &c.component)?);
                scope.composition = Some(format!(
                    "{{ width: {}, height: {}, duration: {}, clipDuration: {} }}",
                    composition.width,
                    composition.height,
                    composition.duration.seconds_text(),
                    clip.duration.seconds_text()
                ));
                let [cw, ch] = [composition.width, composition.height];
                let (sx, sy) = match clip.look.fit.unwrap_or(Fit::Contain) {
                    Fit::Contain => {
                        let s = format!("Math.min(size[0] / {cw}, size[1] / {ch})");
                        (s.clone(), s)
                    }
                    Fit::Cover => {
                        let s = format!("Math.max(size[0] / {cw}, size[1] / {ch})");
                        (s.clone(), s)
                    }
                    Fit::Fill => (format!("size[0] / {cw}"), format!("size[1] / {ch}")),
                    Fit::None => ("1".into(), "1".into()),
                };
                writeln!(scope.lets, "  const fit = [{sx}, {sy}];").unwrap();
                Ok(format!(
                    "<View style={{{{ position: \"absolute\", left: (size[0] - {cw} * fit[0]) / 2, top: (size[1] - {ch} * fit[1]) / 2, width: {cw}, height: {ch}, transformOrigin: \"0 0\", transform: `scale(${{fit[0]}}, ${{fit[1]}})`, overflow: \"hidden\" }}}}><{name} {{...{props}}} /></View>"
                ))
            }
            Role::Overlay { .. } => {
                for (k, given) in [
                    ("fit", clip.look.fit.is_some()),
                    ("end", clip.source.end != End::Error),
                    (
                        "trimStart or rate",
                        !matches!(&clip.source.time, SourceTime::Linear { trim, rate } if *trim == Tick::ZERO && *rate == Speed::from_f64(1.0).unwrap_or(*rate)),
                    ),
                ] {
                    if given {
                        return Err(format!(
                            "{at}: {} is an overlay(), laid out at the clip's size and timed by \
                             its intro and outro, so the clip has no {k}",
                            c.component
                        ));
                    }
                }
                if let Some((intro, outro)) = d.role.ends()
                    && clip.duration < intro + outro
                {
                    return Err(format!(
                        "{at}.duration: {} s is shorter than the overlay's intro and outro, {} s",
                        clip.duration.seconds_text(),
                        (intro + outro).seconds_text()
                    ));
                }
                scope.time = Some(ClipTime::Role {
                    role: d.role,
                    duration: composition.duration,
                });
                // Its size: the clip's, where it starts.
                let [w, h] = match &clip.look.size {
                    Some(v) => pair_of(first(v), &format!("{at}.size"))?,
                    None => [self.doc.canvas.width as f64, self.doc.canvas.height as f64],
                };
                scope.composition = Some(format!(
                    "{{ width: {}, height: {}, duration: {}, clipDuration: {} }}",
                    num(w),
                    num(h),
                    composition.duration.seconds_text(),
                    clip.duration.seconds_text()
                ));
                Ok(format!(
                    "<View style={{{{ position: \"absolute\", inset: 0 }}}}><{name} {{...{props}}} /></View>"
                ))
            }
            Role::Transition | Role::Effect | Role::CaptionPresenter { .. } => Err(format!(
                "{at}.component: {} is a {}, which {}",
                c.component,
                match d.role {
                    Role::Transition => "transition()",
                    Role::Effect => "effect()",
                    _ => "captionPresenter()",
                },
                match d.role {
                    Role::Transition =>
                        "goes between two clips of a sequence: { \"transition\": \"motion\", \"component\": … }",
                    Role::Effect => "goes in a clip's effects or on an adjustment track",
                    _ => "goes on a caption track as its presenter",
                }
            )),
        }
    }

    /// What `c` gives component `d` as JSON: its props' and data's values, its declared
    /// defaults where not given; and its resource slots as resources of the scene, bound to the
    /// document's aliases or to their defaults, in `scope.names`.
    fn given(
        &mut self,
        c: &Component,
        d: &Declared,
        at: &str,
        caption: Option<Value>,
        scope: &mut Scope,
    ) -> Result<String, Error> {
        let mut inputs = Inputs {
            caption,
            ..Inputs::default()
        };
        // Keyframed props, as JS over the clip's time, in place of their constants.
        let mut curves = Vec::new();
        for (k, v) in &c.props {
            let w = format!("{at}.props.{k}");
            let Some(p) = d.props.iter().find(|p| p.name == *k) else {
                return Err(format!(
                    "{w}: {} declares no prop {k} in controls.props",
                    c.component
                ));
            };
            match v {
                Animated::Constant(v) => {
                    inputs.props.insert(k.clone(), prop_value(&p.kind, v));
                }
                Animated::Keys { .. } => {
                    let kind = p.kind.clone();
                    let js = curve(v, &w, &|v, w| {
                        let value = front::controls::check(&kind, prop_value(&kind, v))
                            .map_err(|e| format!("{w}: {e}"))?;
                        Js::of(&kind, &value).ok_or_else(|| {
                            format!(
                                "{w}: keyframes are for number, length, angle, point, rect and \
                                 color props; a {} prop is a constant",
                                kind_name(&kind)
                            )
                        })
                    })?;
                    curves.push(format!("{}: {js}", js_key(k)));
                }
            }
        }
        if let Some(data) = &c.data {
            inputs.data = data.clone();
        }
        let values = front::controls::values(&d.props, &d.data, &inputs)
            .map_err(|e| format!("{at}: {}: {e}", c.component))?;
        if let Some(k) = (c.resources.keys()).find(|k| !d.resources.iter().any(|s| s.name == **k)) {
            return Err(format!(
                "{at}.resources.{k}: {} declares no resource {k} in controls.resources",
                c.component
            ));
        }
        for slot in &d.resources {
            let files = match c.resources.get(&slot.name) {
                Some(alias) => self.files(alias, &format!("{at}.resources.{}", slot.name))?,
                None => slot.default.clone(),
            };
            if files.is_empty() {
                return Err(format!(
                    "{at}.resources: {} has no default for its resource {}, so the clip binds one",
                    c.component, slot.name
                ));
            }
            let r = self.resource(slot.kind, files);
            scope.names.insert(slot.name.clone(), Value::String(r));
        }
        let values = serde_json::to_string(&Value::Object(values)).unwrap_or_default();
        Ok(match curves.is_empty() {
            true => values,
            false => format!("{{ ...{values}, {} }}", curves.join(", ")),
        })
    }
}

/// What a clip's TimeScope gives inside: its time, the composition components see, and the
/// names of the scene's resources that their own names stand for.
#[derive(Default)]
struct Scope {
    time: Option<ClipTime>,
    composition: Option<String>,
    names: Map<String, Value>,
    /// Constants of the clip's component its content reads, as JS.
    lets: String,
}

/// A source's time in a clip: its trim and rate or its time map, held or looped past `length`
/// as the clip's `end` says; with `end: "error"`, an error when the clip reads past it.
fn source_time(clip: &Clip, length: Option<Tick>, what: &str) -> Result<ClipTime, Error> {
    let at = &clip.at;
    let (trim, rate, map) = match &clip.source.time {
        SourceTime::Linear { trim, rate } => (*trim, *rate, Vec::new()),
        SourceTime::Map(points) => (Tick::ZERO, Speed::from_f64(1.0)?, points.clone()),
    };
    // How far into the source the clip reads, and whether it reads that tick itself.
    let (reach, inclusive) = match map.as_slice() {
        [] => (trim + rate.apply(clip.duration)?, false),
        points => (points.iter().map(|p| p.1).max().unwrap_or_default(), true),
    };
    let length = match (clip.source.end, length) {
        (End::Error, Some(length)) if reach > length || (inclusive && reach == length) => {
            return Err(format!(
                "{at}: the clip reads {what} to {} s, past its end at {} s; trim it, or set end \
                 to \"hold\" or \"loop\"",
                reach.seconds_text(),
                length.seconds_text()
            ));
        }
        (End::Error, _) => None,
        (End::Hold | End::Loop, None) => {
            return Err(format!("{at}.end: the length of {what} is not known"));
        }
        (End::Hold, Some(l)) => Some((l, false)),
        (End::Loop, Some(l)) => Some((l, true)),
    };
    Ok(ClipTime::Source {
        trim,
        rate,
        map,
        length,
    })
}

/// The import key of a component file from the scene beside the document: relative to its
/// folder.
fn import_key(folder: &str, file: &str) -> String {
    let from: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    let to: Vec<&str> = file.split('/').filter(|p| !p.is_empty()).collect();
    let same = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut key = match from.len() - same {
        0 => "./".to_string(),
        up => "../".repeat(up),
    };
    key.push_str(&to[same..].join("/"));
    key
}

fn js(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// A key of a JS object literal.
fn js_key(k: &str) -> String {
    let plain = k.chars().enumerate().all(|(i, c)| {
        c == '_' || c == '$' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
    });
    match plain && !k.is_empty() {
        true => k.to_string(),
        false => js(&k),
    }
}

/// A number as JS: its shortest decimal.
fn num(x: f64) -> String {
    crate::js::number_to_string(x)
}

/// A value at one keyframe, as JS: a number, text a Motion `interpolate` takes (a color, a
/// length, an angle), or fields of numbers.
#[derive(Clone)]
enum Js {
    Number(f64),
    Pair(f64, f64),
    Text(String),
    Fields(Vec<(&'static str, f64)>),
}

impl Js {
    /// A prop's value of `kind` to interpolate, if it is one.
    fn of(kind: &Kind, v: &Value) -> Option<Js> {
        let fields = |names: &[&'static str]| {
            let o = v.as_object()?;
            names
                .iter()
                .map(|n| Some((*n, o.get(*n)?.as_f64()?)))
                .collect::<Option<Vec<_>>>()
        };
        match kind {
            Kind::Number { .. } => v.as_f64().map(Js::Number),
            Kind::Color | Kind::Length | Kind::Angle => v.as_str().map(|s| Js::Text(s.into())),
            Kind::Point => fields(&["x", "y"]).map(Js::Fields),
            Kind::Rect => fields(&["x", "y", "width", "height"]).map(Js::Fields),
            _ => None,
        }
    }
}

/// `v` as JS over the clip time `t`: a constant, or its keyframes, each stretch eased as the
/// keyframe that starts it says, or held with `interpolation: "step"`. `value` checks a value
/// and makes it JS.
fn curve(
    v: &Animated,
    at: &str,
    value: &dyn Fn(&Value, &str) -> Result<Js, Error>,
) -> Result<String, Error> {
    let (keys, step) = match v {
        Animated::Constant(c) => return Ok(literal(&value(c, at)?)),
        Animated::Keys { keys, step } => (keys, *step),
    };
    let mut points = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let w = format!("{at}.keyframes[{i}]");
        let easing = match (&k.easing, step) {
            (_, true) => "step-end".to_string(),
            (Some(e), false) => {
                crate::ease::Easing::parse(e).map_err(|e| format!("{w}[2]: {e}"))?;
                e.clone()
            }
            (None, false) => "linear".into(),
        };
        points.push((
            k.at.seconds_text(),
            value(&k.value, &format!("{w}[1]"))?,
            easing,
        ));
    }
    // Each part of the value on its own.
    let parts = |j: &Js| -> Vec<Js> {
        match j {
            Js::Pair(a, b) => vec![Js::Number(*a), Js::Number(*b)],
            Js::Fields(f) => f.iter().map(|(_, x)| Js::Number(*x)).collect(),
            other => vec![other.clone()],
        }
    };
    let shape = &points[0].1;
    let count = parts(shape).len();
    if (points.iter()).any(|(_, v, _)| parts(v).len() != count) {
        return Err(format!("{at}: every keyframe's value is of one kind"));
    }
    let mut each = Vec::new();
    for p in 0..count {
        let at_part: Vec<(String, String, String)> = (points.iter())
            .map(|(t, v, e)| (t.clone(), literal(&parts(v)[p]), e.clone()))
            .collect();
        each.push(stretches(&at_part));
    }
    Ok(match shape {
        Js::Pair(..) => format!("[{}, {}]", each[0], each[1]),
        Js::Fields(f) => {
            let fields: Vec<String> = (f.iter().zip(&each))
                .map(|((n, _), e)| format!("{n}: {e}"))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        _ => each.remove(0),
    })
}

/// Keyframes of one number or text as JS: one `interpolate` when one easing eases them all,
/// else a stretch at a time.
fn stretches(points: &[(String, String, String)]) -> String {
    if points.len() == 1 {
        return points[0].1.clone();
    }
    let segment = |i: usize| {
        let ((t0, v0, e), (t1, v1, _)) = (&points[i], &points[i + 1]);
        format!(
            "interpolate(t, [{t0}, {t1}], [{v0}, {v1}], {{ easing: {} }})",
            js(e)
        )
    };
    let one = points[..points.len() - 1]
        .iter()
        .all(|p| p.2 == points[0].2);
    if one {
        let times: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
        let values: Vec<&str> = points.iter().map(|p| p.1.as_str()).collect();
        return format!(
            "interpolate(t, [{}], [{}], {{ easing: {} }})",
            times.join(", "),
            values.join(", "),
            js(&points[0].2)
        );
    }
    let mut out = segment(points.len() - 2);
    for i in (0..points.len() - 2).rev() {
        out = format!("(t < {} ? {} : {out})", points[i + 1].0, segment(i));
    }
    out
}

fn literal(j: &Js) -> String {
    match j {
        Js::Number(x) => num(*x),
        Js::Pair(a, b) => format!("[{}, {}]", num(*a), num(*b)),
        Js::Text(s) => js(s),
        Js::Fields(f) => {
            let fields: Vec<String> = f.iter().map(|(n, x)| format!("{n}: {}", num(*x))).collect();
            format!("{{ {} }}", fields.join(", "))
        }
    }
}

/// `[x, y]` as two numbers.
fn pair_of(v: &Value, at: &str) -> Result<[f64; 2], Error> {
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(format!("{at}: [x, y], two numbers")),
        },
        _ => Err(format!("{at}: [x, y], two numbers")),
    }
}

/// A value's first: its constant, or its first keyframe's.
fn first(v: &Animated) -> &Value {
    match v {
        Animated::Constant(c) => c,
        Animated::Keys { keys, .. } => &keys[0].value,
    }
}

/// A prop's value as the Timeline writes it, as Motion takes it: a point `[x, y]` as `{ x, y }`,
/// a rect as `{ x, y, width, height }`, a length's px and an angle's degrees as numbers.
fn prop_value(kind: &Kind, v: &Value) -> Value {
    let numbers = |n: usize| {
        let list = v.as_array().filter(|a| a.len() == n)?;
        list.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()
    };
    match kind {
        Kind::Point => match numbers(2) {
            Some(p) => serde_json::json!({ "x": p[0], "y": p[1] }),
            None => v.clone(),
        },
        Kind::Rect => match numbers(4) {
            Some(r) => serde_json::json!({ "x": r[0], "y": r[1], "width": r[2], "height": r[3] }),
            None => v.clone(),
        },
        Kind::Length => match v.as_f64() {
            Some(x) => Value::String(format!("{}px", num(x))),
            None => v.clone(),
        },
        Kind::Angle => match v.as_f64() {
            Some(x) => Value::String(format!("{}deg", num(x))),
            None => v.clone(),
        },
        _ => v.clone(),
    }
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Number { .. } => "number",
        Kind::String => "string",
        Kind::Boolean => "boolean",
        Kind::Color => "color",
        Kind::Select(_) => "select",
        Kind::Point => "point",
        Kind::Length => "length",
        Kind::Angle => "angle",
        Kind::Rect => "rect",
        Kind::Json => "data",
    }
}
