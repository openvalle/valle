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
use valle_timeline::{Clip, Component, Content, Document, End, Fit, SourceTime, Track};

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

    /// A visual clip's `<TimeScope clip>` and what it shows.
    fn clip(&mut self, track: &Track, clip: &Clip, fill: &str) -> Result<String, Error> {
        let at = clip.at.as_str();
        if !clip.look.effects.is_empty() {
            return Err(format!("{at}.effects: clip effects are not supported yet"));
        }
        let looks = [
            ("position", clip.look.position.is_some()),
            ("size", clip.look.size.is_some()),
            ("scale", clip.look.scale.is_some()),
            ("rotation", clip.look.rotation.is_some()),
            ("anchor", clip.look.anchor.is_some()),
            ("opacity", clip.look.opacity.is_some()),
            ("blend", clip.look.blend.is_some()),
        ];
        if let Some((k, _)) = looks.iter().find(|(_, given)| *given) {
            return Err(format!("{at}.{k}: not supported yet"));
        }
        let fit = clip.look.fit.unwrap_or(Fit::Contain).css();
        let mut scope = Scope::default();
        let content = match &clip.content {
            Content::Solid { color } => {
                format!(
                    "<View style={{{{ position: \"absolute\", inset: 0, backgroundColor: {} }}}} />",
                    js(color)
                )
            }
            Content::Image { src } => {
                let file = self.file(src, &format!("{at}.src"))?;
                let r = self.resource(Holds::Image, vec![file]);
                format!(
                    "<Image src={} style={{{{ position: \"absolute\", inset: 0, width: \"100%\", height: \"100%\", objectFit: {} }}}} />",
                    js(&format!("resource://{r}")),
                    js(&fit)
                )
            }
            Content::Video { src } => {
                let file = self.file(src, &format!("{at}.src"))?;
                let length = (self.host.length)(&file);
                scope.time = Some(source_time(clip, length, &file)?);
                let r = self.resource(Holds::Video, vec![file]);
                format!(
                    "<Video src={} style={{{{ position: \"absolute\", inset: 0, width: \"100%\", height: \"100%\", objectFit: {} }}}} />",
                    js(&format!("resource://{r}")),
                    js(&fit)
                )
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
        let time = scope.time.unwrap_or(ClipTime::Source {
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
            "        <TimeScope {}>\n          {content}\n        </TimeScope>\n",
            props.join(" ")
        ))
    }

    /// A motion clip's component, given its props, data and resources, and fitted into the
    /// clip's box, or laid out at its size when it is an overlay.
    fn motion(&mut self, clip: &Clip, c: &Component, scope: &mut Scope) -> Result<String, Error> {
        let at = clip.at.as_str();
        let (d, name) = self.component(&c.component, &format!("{at}.component"))?;
        let [w, h] = [self.doc.canvas.width, self.doc.canvas.height];
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
                let fit = clip.look.fit.unwrap_or(Fit::Contain);
                let [cw, ch] = [composition.width as f64, composition.height as f64];
                let (sx, sy) = scales(fit, [w as f64, h as f64], [cw, ch]);
                Ok(format!(
                    "<View style={{{{ position: \"absolute\", left: {}, top: {}, width: {cw}, height: {ch}, transformOrigin: \"0 0\", transform: \"scale({sx}, {sy})\", overflow: \"hidden\" }}}}><{name} {{...{props}}} /></View>",
                    (w as f64 - cw * sx) / 2.0,
                    (h as f64 - ch * sy) / 2.0,
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
                scope.composition = Some(format!(
                    "{{ width: {w}, height: {h}, duration: {}, clipDuration: {} }}",
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
        for (k, v) in &c.props {
            match v {
                valle_timeline::Animated::Constant(v) => {
                    inputs.props.insert(k.clone(), v.clone());
                }
                valle_timeline::Animated::Keys { .. } => {
                    return Err(format!("{at}.props.{k}: keyframes are not supported yet"));
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
        Ok(serde_json::to_string(&Value::Object(values)).unwrap_or_default())
    }
}

/// What a clip's TimeScope gives inside: its time, the composition components see, and the
/// names of the scene's resources that their own names stand for.
#[derive(Default)]
struct Scope {
    time: Option<ClipTime>,
    composition: Option<String>,
    names: Map<String, Value>,
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

/// The x and y scales that put a box `inner` big into one `outer` big as `fit` says.
fn scales(fit: Fit, outer: [f64; 2], inner: [f64; 2]) -> (f64, f64) {
    let [sx, sy] = [outer[0] / inner[0], outer[1] / inner[1]];
    match fit {
        Fit::Contain => (sx.min(sy), sx.min(sy)),
        Fit::Cover => (sx.max(sy), sx.max(sy)),
        Fit::Fill => (sx, sy),
        Fit::None => (1.0, 1.0),
    }
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
