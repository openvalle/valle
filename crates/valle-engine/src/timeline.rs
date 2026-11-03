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

/// The standard caption template, which the scene imports from beside the document.
const CAPTION: &str = include_str!("../templates/caption.motion.tsx");

/// Where the scene finds the caption template.
fn template_path(folder: &str) -> String {
    front::join(folder, "__valle/caption.motion.tsx")
}

/// A color grade's temperature, as the old engine had it: red times 2^(t / 4), blue divided by
/// it, of what is under it.
const GRADE: &str = "#version 300 es
precision highp float;
uniform sampler2D backdrop;
uniform float temperature;
in vec2 uv;
out vec4 color;
void main() {
  vec4 c = texture(backdrop, uv);
  float g = exp2(temperature * 0.25);
  color = vec4(c.r * g, c.g, c.b / g, c.a);
}";

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
        root: folder,
        prefix: String::new(),
        nesting: vec![path.to_string()],
        host,
        s: Shared::default(),
    };
    let [w, h] = [doc.canvas.width, doc.canvas.height];
    let body = l.body()?;

    let mut files = HashMap::new();
    if l.s.template {
        files.insert(template_path(folder), CAPTION.to_string());
    }
    let mut code = String::from("// The scene a Timeline document plays as.\n");
    for (name, file) in &l.s.imports {
        writeln!(
            code,
            "import {name} from {};",
            js(&import_key(folder, file))
        )
        .unwrap();
    }
    let slots: Vec<String> = (l.s.slots.iter())
        .map(|(name, kind)| format!("{}: resource({{ kind: {} }})", js(name), js(&kind.name())))
        .collect();
    for f in &l.s.functions {
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
        resources: l.s.bound,
        clips: Rc::new(l.s.clips),
        drawn: l.s.drawn,
        ..Inputs::default()
    };
    files.insert(entry.clone(), code);
    Ok(Lowered {
        files,
        entry,
        inputs,
    })
}

struct Lowering<'a> {
    doc: &'a Document,
    folder: &'a str,
    /// The folder of the document the scene plays, beside which it finds the caption template.
    root: &'a str,
    /// What the ids of the document's clips are prefixed with in the clip table: the ids of the
    /// clips it is nested in.
    prefix: String,
    /// The documents it is nested in, by file.
    nesting: Vec<String>,
    host: &'a Host<'a>,
    s: Shared,
}

/// What lowering a document and those nested in it makes, for the one scene.
#[derive(Default)]
struct Shared {
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
    /// The pictures boxes of the scene draw, `p0`, `p1`…
    drawn: Vec<String>,
    /// Whether a caption track draws with the standard template.
    template: bool,
}

impl Lowering<'_> {
    /// What the document's tracks show, each over those before it: its visual tracks, its
    /// adjustment tracks over them, and its captions over all.
    fn body(&mut self) -> Result<String, Error> {
        let doc = self.doc;
        let [w, h] = [doc.canvas.width, doc.canvas.height];
        let fill =
            format!("{{ position: \"absolute\", left: 0, top: 0, width: {w}, height: {h} }}");
        let mut body = String::new();
        for track in &doc.tracks {
            match track.kind {
                valle_timeline::Kind::Visual => body.push_str(&self.track(track, &fill)?),
                // Sound is the mix's.
                valle_timeline::Kind::Audio => {}
                valle_timeline::Kind::Adjustment => body = self.adjustment(track, body, &fill)?,
                // Over all the rest.
                valle_timeline::Kind::Caption => {}
            }
        }
        for track in &doc.tracks {
            if track.kind == valle_timeline::Kind::Caption {
                body.push_str(&self.captions(track, &fill)?);
            }
        }
        Ok(body)
    }

    /// A nested timeline's content: the document in file `file` fitted into the clip's box as a
    /// Motion clip is, on its own background; its clips on the clip's source time.
    fn nested(&mut self, clip: &Clip, file: &str, scope: &mut Scope) -> Result<String, Error> {
        let at = clip.at.as_str();
        if self.nesting.iter().any(|f| f == file) {
            return Err(format!(
                "{at}.src: {file} is nested in itself: {} → {file}",
                self.nesting.join(" → ")
            ));
        }
        let text = (self.host.read)(file).ok_or_else(|| format!("{at}.src: {file}: not found"))?;
        let doc = valle_timeline::parse(&text).map_err(|e| format!("{file}: {e}"))?;
        let mut nesting = self.nesting.clone();
        nesting.push(file.to_string());
        let mut inner = Lowering {
            doc: &doc,
            folder: file.rsplit_once('/').map_or("", |(dir, _)| dir),
            root: self.root,
            prefix: format!("{}{}/", self.prefix, clip.id),
            nesting,
            host: self.host,
            s: std::mem::take(&mut self.s),
        };
        let body = inner.body();
        self.s = inner.s;
        let body = body?;
        let [cw, ch] = [doc.canvas.width, doc.canvas.height];
        writeln!(scope.lets, "  const fit = [{}];", fit_scale(clip, [cw, ch])).unwrap();
        Ok(format!(
            "<View style={{{{ position: \"absolute\", inset: 0, overflow: \"hidden\" }}}}><View style={{{{ position: \"absolute\", left: (size[0] - {cw} * fit[0]) / 2, top: (size[1] - {ch} * fit[1]) / 2, width: {cw}, height: {ch}, transformOrigin: \"0 0\", transform: `scale(${{fit[0]}}, ${{fit[1]}})`, overflow: \"hidden\", backgroundColor: {} }}}}>\n{body}      </View></View>",
            js(&doc.canvas.background)
        ))
    }

    /// A new resource of the scene holding `kind`, bound to `files`.
    fn resource(&mut self, kind: Holds, files: Vec<String>) -> String {
        let name = format!("r{}", self.s.slots.len());
        self.s.slots.push((name.clone(), kind));
        self.s.bound.insert(name.clone(), files);
        name
    }

    /// A new picture of the scene, which a box draws.
    fn picture(&mut self) -> String {
        let name = format!("p{}", self.s.drawn.len());
        self.s.drawn.push(name.clone());
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
        if let Some(d) = self.s.declared.get(&file) {
            let name = (self.s.imports.iter())
                .find(|(_, f)| *f == file)
                .map(|(n, _)| n.clone())
                .expect("imported with it");
            return Ok((d.clone(), name));
        }
        let d = Rc::new(front::declared(&file, self.host.read)?);
        let name = format!("C{}", self.s.imports.len());
        self.s.imports.push((name.clone(), file.clone()));
        self.s.declared.insert(file, d.clone());
        Ok((d, name))
    }

    /// A visual clip's `<TimeScope clip>`, and the component it shows that draws its box and
    /// what is in it, on the clip's time.
    fn clip(&mut self, track: &Track, clip: &Clip, reads: [Tick; 2]) -> Result<Drawn, Error> {
        let at = clip.at.as_str();
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
                scope.time = Some(source_time(clip, reads, length, &file)?);
                let r = self.resource(Holds::Video, vec![file]);
                media(&r, "Video")
            }
            Content::Motion(c) => self.motion(clip, c, reads, &mut scope)?,
            Content::Timeline { src } => {
                let file = self.file(src, &format!("{at}.src"))?;
                let text = (self.host.read)(&file);
                let length = text
                    .as_deref()
                    .and_then(|t| valle_timeline::parse(t).ok())
                    .map(|d| d.duration);
                scope.time = Some(source_time(clip, reads, length, &file)?);
                self.nested(clip, &file, &mut scope)?
            }
            Content::Audio { .. } | Content::Caption { .. } | Content::Grade { .. } => {
                unreachable!("not on a visual track: {}", track.at)
            }
        };
        let time = scope.time.take().unwrap_or(ClipTime::Source {
            trim: Tick::ZERO,
            rate: Speed::ONE,
            map: Vec::new(),
            length: None,
        });
        // Its effects work on it before its opacity and its blend.
        let finished = clip.look.effects.is_empty();
        let shows = format!("K{}", self.s.functions.len());
        let body = self.look(clip, &scope.lets, &content, finished)?;
        self.s.functions.push(format!(
            "function {shows}() {{\n  const t = useCurrentTime().clipSeconds;\n{body}}}\n"
        ));
        let mut effects = Vec::new();
        for (k, e) in clip.look.effects.iter().enumerate() {
            let at = format!("{at}.effects[{k}]");
            let (d, name) = self.component(&e.component, &format!("{at}.component"))?;
            if d.role != Role::Effect {
                return Err(format!(
                    "{at}.component: {} is not an effect(); an effect declares `export const \
                     role = effect()` and reads the clip as input://source",
                    e.component
                ));
            }
            let mut scope = Scope::default();
            let props = self.given(e, &d, &at, None, &mut scope)?;
            let mut style = vec!["position: \"absolute\"".to_string(), "inset: 0".into()];
            if k + 1 == clip.look.effects.len() {
                style.extend(self.finish(clip)?);
            }
            let shows = format!("K{}", self.s.functions.len());
            self.s.functions.push(format!(
                "function {shows}() {{\n  const t = useCurrentTime().clipSeconds;\n  return (\n    <View style={{{{ {} }}}}>\n      <{name} {{...{props}}} />\n    </View>\n  );\n}}\n",
                style.join(", ")
            ));
            effects.push(Effect {
                shows,
                props: self.role_props(&d, clip.duration, &scope),
                time: ClipTime::Role {
                    role: d.role,
                    duration: d.composition.duration,
                },
            });
        }
        let mut props = Vec::new();
        if let Some(c) = scope.composition {
            props.push(format!("composition={{{c}}}"));
        }
        if !scope.names.is_empty() {
            props.push(format!("resources={{{}}}", js(&scope.names)));
        }
        Ok(Drawn {
            shows,
            props,
            time,
            effects,
        })
    }

    /// The props of the TimeScope of a role's component in a clip `duration` long: the
    /// composition it sees, of the canvas's size, and the scene's resources its own stand for.
    fn role_props(&self, d: &Declared, duration: Tick, scope: &Scope) -> Vec<String> {
        let mut props = vec![format!(
            "composition={{{{ width: {}, height: {}, duration: {}, clipDuration: {} }}}}",
            self.doc.canvas.width,
            self.doc.canvas.height,
            d.composition.duration.seconds_text(),
            duration.seconds_text()
        )];
        if !scope.names.is_empty() {
            props.push(format!("resources={{{}}}", js(&scope.names)));
        }
        props
    }

    /// A clip shown over `on`, as clip `id` of the table, with `extra` props on its TimeScope:
    /// with effects, its picture drawn, then each effect reading the one before.
    #[allow(clippy::too_many_arguments)]
    fn instance(
        &mut self,
        clip: &Clip,
        d: &Drawn,
        id: &str,
        key: &str,
        on: [Tick; 2],
        extra: &[String],
        fill: &str,
    ) -> String {
        let table = |time: &ClipTime| template::Clip {
            start: clip.start,
            end: clip.end(),
            on,
            time: time.clone(),
            frames: None,
        };
        let shows = format!("<{} />", d.shows);
        if d.effects.is_empty() {
            let props: Vec<String> = d.props.iter().chain(extra).cloned().collect();
            return self.scope(id, key, table(&d.time), &props, fill, &shows);
        }
        let mut source = self.picture();
        let mut props = d.props.clone();
        props.push(format!("picture={}", js(&source)));
        let mut inside = self.scope(
            &format!("{id}~source"),
            "source",
            table(&d.time),
            &props,
            fill,
            &shows,
        );
        for (k, e) in d.effects.iter().enumerate() {
            let mut props = e.props.clone();
            props.push(format!("inputs={{{{ source: {} }}}}", js(&source)));
            if k + 1 < d.effects.len() {
                source = self.picture();
                props.push(format!("picture={}", js(&source)));
            }
            let shows = format!("<{} />", e.shows);
            inside.push_str(&self.scope(
                &format!("{id}~e{k}"),
                &format!("e{k}"),
                table(&e.time),
                &props,
                fill,
                &shows,
            ));
        }
        // All on the timeline's time.
        let whole = template::Clip {
            start: Tick::ZERO,
            end: self.doc.duration,
            on,
            time: same_time(),
            frames: None,
        };
        self.scope(id, key, whole, extra, fill, &inside)
    }

    /// A clip of the table, `id`, shown over `on` and timed as `clip`, and its TimeScope with
    /// `inside` in it.
    fn scope(
        &mut self,
        id: &str,
        key: &str,
        clip: template::Clip,
        props: &[String],
        fill: &str,
        inside: &str,
    ) -> String {
        let id = format!("{}{id}", self.prefix);
        self.s.clips.insert(id.clone(), Rc::new(clip));
        let mut all = vec![
            format!("key={}", js(&key)),
            format!("clip={}", js(&id)),
            format!("style={{{fill}}}"),
        ];
        all.extend(props.iter().cloned());
        format!("<TimeScope {}>{inside}</TimeScope>", all.join(" "))
    }

    /// A visual track: each clip where it shows alone, and each transition with the two clips it
    /// goes between, both on their own time over its span, in a `<Transition>`.
    fn track(&mut self, track: &Track, fill: &str) -> Result<String, Error> {
        let n = track.clips.len();
        // How much of each clip its transitions take at its head and its tail.
        let (mut head, mut tail) = (vec![Tick::ZERO; n], vec![Tick::ZERO; n]);
        for t in &track.transitions {
            tail[t.from] = t.in_offset;
            head[t.to] = t.out_offset;
        }
        let mut out = format!("      <View key={} style={{{fill}}}>\n", js(&track.id));
        let mut drawn: Vec<Option<Drawn>> = Vec::new();
        for (i, clip) in track.clips.iter().enumerate() {
            if !clip.enabled {
                drawn.push(None);
                continue;
            }
            // The clip reads its source from before its start into the transition before it, to
            // past its end into the one after it: the handles.
            let into = (track.transitions.iter())
                .find(|t| t.to == i)
                .map(|t| t.in_offset);
            let past = (track.transitions.iter())
                .find(|t| t.from == i)
                .map(|t| t.out_offset);
            let reads = [
                Tick::ZERO - into.unwrap_or_default(),
                clip.duration + past.unwrap_or_default(),
            ];
            let d = self.clip(track, clip, reads)?;
            let on = [clip.start + head[i], clip.end() - tail[i]];
            if on[0] < on[1] {
                let shown = self.instance(clip, &d, &clip.id, &clip.id, on, &[], fill);
                writeln!(out, "        {shown}").unwrap();
            }
            drawn.push(Some(d));
        }
        for (k, t) in track.transitions.iter().enumerate() {
            let (lo, hi) = t.span(track);
            let id = format!("{}~{k}", track.id);
            if let Some(c) = &t.component {
                // The two clips drawn as pictures, which it reads.
                let mut names = Map::new();
                for (side, i) in [("from", t.from), ("to", t.to)] {
                    let d = drawn[i]
                        .as_ref()
                        .expect("transitions go between enabled clips");
                    let p = self.picture();
                    let extra = [format!("picture={}", js(&p))];
                    let shown = self.instance(
                        &track.clips[i],
                        d,
                        &format!("{id}~{side}"),
                        side,
                        [lo, hi],
                        &extra,
                        fill,
                    );
                    writeln!(out, "        {shown}").unwrap();
                    names.insert(side.to_string(), Value::String(p));
                }
                let at = format!("{}.component", t.at);
                let (d, name) = self.component(&c.component, &at)?;
                if d.role != Role::Transition {
                    return Err(format!(
                        "{at}: {} is not a transition(); a transition declares `export const \
                         role = transition()` and reads input://from and input://to",
                        c.component
                    ));
                }
                let mut scope = Scope::default();
                let props = self.given(c, &d, &t.at, None, &mut scope)?;
                let shows = format!("K{}", self.s.functions.len());
                self.s.functions.push(format!(
                    "function {shows}() {{\n  const t = useCurrentTime().clipSeconds;\n  return (\n    <View style={{{{ position: \"absolute\", inset: 0 }}}}>\n      <{name} {{...{props}}} />\n    </View>\n  );\n}}\n"
                ));
                let mut props = self.role_props(&d, hi - lo, &scope);
                props.push(format!("inputs={{{}}}", js(&names)));
                let table = template::Clip {
                    start: lo,
                    end: hi,
                    on: [lo, hi],
                    time: ClipTime::Role {
                        role: d.role,
                        duration: d.composition.duration,
                    },
                    frames: Some(self.doc.canvas.fps),
                };
                let shown = self.scope(&id, &id, table, &props, fill, &format!("<{shows} />"));
                writeln!(out, "        {shown}").unwrap();
                continue;
            }
            let mut sides = Vec::new();
            for (side, i) in [("from", t.from), ("to", t.to)] {
                let d = drawn[i]
                    .as_ref()
                    .expect("transitions go between enabled clips");
                let clip = &track.clips[i];
                sides.push(self.instance(
                    clip,
                    d,
                    &format!("{id}~{side}"),
                    side,
                    [lo, hi],
                    &[],
                    fill,
                ));
            }
            let shows = format!("K{}", self.s.functions.len());
            self.s.functions.push(format!(
                "function {shows}() {{
  const progress = useCurrentTime().clipProgress;
  return (
    <Transition kind={} progress={{progress}} params={{{}}} style={{{fill}}}>
      {}
      {}
    </Transition>
  );
}}
",
                js(&t.kind),
                js(&t.params),
                sides[0],
                sides[1],
            ));
            // The transition's own clip: the timeline's time, its progress over its frames.
            let table = template::Clip {
                start: lo,
                end: hi,
                on: [lo, hi],
                time: ClipTime::Source {
                    trim: lo,
                    rate: Speed::ONE,
                    map: Vec::new(),
                    length: None,
                },
                frames: Some(self.doc.canvas.fps),
            };
            let inside = format!("<{shows} />");
            let shown = self.scope(&id, &id, table, &[], fill, &inside);
            writeln!(out, "        {shown}").unwrap();
        }
        out.push_str("      </View>\n");
        Ok(out)
    }

    /// A caption track: each caption a clip of the table, drawn by its presenter or by the
    /// standard template in the track's style, given its `caption`, `{ text, runs }`.
    fn captions(&mut self, track: &Track, fill: &str) -> Result<String, Error> {
        let at = track.at.as_str();
        let (made, d, name) = match &track.presenter {
            Some(c) => {
                let w = format!("{at}.presenter.component");
                let (d, name) = self.component(&c.component, &w)?;
                if !matches!(d.role, Role::CaptionPresenter { .. }) {
                    return Err(format!(
                        "{w}: {} is not a captionPresenter(); a presenter declares `export const \
                         role = captionPresenter({{ intro, outro }})` and reads its caption prop",
                        c.component
                    ));
                }
                (None, d, name)
            }
            None => {
                let path = template_path(self.root);
                let read = |p: &str| match p == path {
                    true => Some(CAPTION.to_string()),
                    false => (self.host.read)(p),
                };
                let d = Rc::new(front::declared(&path, &read)?);
                let name = format!("C{}", self.s.imports.len());
                self.s.imports.push((name.clone(), path));
                self.s.template = true;
                // The track's style as the template's props; its font a resource of the scene.
                let mut c = Component {
                    component: "the caption template".into(),
                    props: Vec::new(),
                    data: None,
                    resources: BTreeMap::new(),
                };
                for (k, v) in track.style.iter().flatten() {
                    let w = format!("{at}.style.{k}");
                    let value = match k.as_str() {
                        "font" => {
                            let alias = v
                                .as_str()
                                .ok_or_else(|| format!("{w}: the alias of a font resource"))?;
                            let files = self.files(alias, &w)?;
                            Value::String(self.resource(Holds::Font, files))
                        }
                        _ if k != "fontFamily" && d.props.iter().any(|p| p.name == *k) => v.clone(),
                        _ => {
                            let mut names: Vec<&str> = (d.props.iter())
                                .map(|p| p.name.as_str())
                                .filter(|n| *n != "fontFamily")
                                .collect();
                            names.insert(0, "font");
                            return Err(format!(
                                "{w}: a caption style has no {k}; it takes {}",
                                names.join(", ")
                            ));
                        }
                    };
                    let k = if k == "font" { "fontFamily" } else { k };
                    c.props.push((k.to_string(), Animated::Constant(value)));
                }
                (Some(c), d, name)
            }
        };
        let c = (made.as_ref())
            .or(track.presenter.as_ref())
            .expect("a presenter or the template's");
        let mut scope = Scope::default();
        let props = self.given(c, &d, at, None, &mut scope)?;
        let mut captions = Vec::new();
        let mut out = format!("      <View key={} style={{{fill}}}>\n", js(&track.id));
        let list = format!("CAPTIONS{}", self.s.functions.len());
        let shows = format!("K{}", self.s.functions.len() + 1);
        for clip in track.clips.iter().filter(|c| c.enabled) {
            let Content::Caption { text, runs } = &clip.content else {
                unreachable!("a caption track holds captions: {}", clip.at)
            };
            if let Some((intro, outro)) = d.role.ends()
                && clip.duration < intro + outro
            {
                return Err(format!(
                    "{}.duration: {} s is shorter than the presenter's intro and outro, {} s",
                    clip.at,
                    clip.duration.seconds_text(),
                    (intro + outro).seconds_text()
                ));
            }
            let runs: Vec<Value> = (runs.iter())
                .map(|r| {
                    serde_json::json!({
                        "text": r.text,
                        "start": r.start.seconds(),
                        "end": r.end.seconds(),
                    })
                })
                .collect();
            captions.push(serde_json::json!({ "text": text, "runs": runs }));
            let table = template::Clip {
                start: clip.start,
                end: clip.end(),
                on: [clip.start, clip.end()],
                time: ClipTime::Role {
                    role: d.role,
                    duration: d.composition.duration,
                },
                frames: None,
            };
            let props = self.role_props(&d, clip.duration, &scope);
            let inside = format!("<{shows} k={{{}}} />", captions.len() - 1);
            let shown = self.scope(&clip.id, &clip.id, table, &props, fill, &inside);
            writeln!(out, "        {shown}").unwrap();
        }
        self.s
            .functions
            .push(format!("const {list} = {};\n", js(&captions)));
        self.s.functions.push(format!(
            "function {shows}({{ k }}) {{\n  const t = useCurrentTime().clipSeconds;\n  return (\n    <View style={{{{ position: \"absolute\", inset: 0 }}}}>\n      <{name} {{...{props}}} caption={{{list}[k]}} />\n    </View>\n  );\n}}\n"
        ));
        out.push_str("      </View>\n");
        Ok(out)
    }

    /// An adjustment track over `below`, all the tracks before it: a color grade over what is
    /// under it; for an effect, `below` drawn as a picture, which the effect reads and replaces,
    /// and shown where no effect is.
    fn adjustment(&mut self, track: &Track, below: String, fill: &str) -> Result<String, Error> {
        let clips: Vec<&Clip> = track.clips.iter().filter(|c| c.enabled).collect();
        let effects = clips
            .iter()
            .any(|c| matches!(c.content, Content::Motion(_)));
        let mut out = below;
        let picture = match effects {
            true => {
                let p = self.picture();
                let whole = template::Clip {
                    start: Tick::ZERO,
                    end: self.doc.duration,
                    on: [Tick::ZERO, self.doc.duration],
                    time: same_time(),
                    frames: None,
                };
                let extra = [format!("picture={}", js(&p))];
                let id = format!("{}~below", track.id);
                // On the canvas's background, as all is.
                let background = fill.replacen(
                    " }",
                    &format!(", backgroundColor: {} }}", js(&self.doc.canvas.background)),
                    1,
                );
                out = format!(
                    "        {}\n",
                    self.scope(&id, &id, whole, &extra, &background, &out)
                );
                p
            }
            false => String::new(),
        };
        writeln!(out, "      <View key={} style={{{fill}}}>", js(&track.id)).unwrap();
        // Where no effect is, what is under it.
        let mut shown = Tick::ZERO;
        let mut gaps = Vec::new();
        let mut spans: Vec<[Tick; 2]> = (clips.iter())
            .filter(|c| matches!(c.content, Content::Motion(_)))
            .map(|c| [c.start, c.end()])
            .collect();
        spans.sort();
        for [start, end] in spans.iter().copied().chain([[self.doc.duration; 2]]) {
            if shown < start {
                gaps.push([shown, start]);
            }
            shown = shown.max(end);
        }
        if effects {
            for (k, [start, end]) in gaps.into_iter().enumerate() {
                let table = template::Clip {
                    start,
                    end,
                    on: [start, end],
                    time: same_time(),
                    frames: None,
                };
                let id = format!("{}~under{k}", track.id);
                let image = format!(
                    "<Image src={} style={{{{ position: \"absolute\", inset: 0, width: \"100%\", height: \"100%\" }}}} />",
                    js(&format!("input://{picture}"))
                );
                writeln!(
                    out,
                    "        {}",
                    self.scope(&id, &id, table, &[], fill, &image)
                )
                .unwrap();
            }
        }
        for clip in clips {
            let at = clip.at.as_str();
            let (time, props, inside) = match &clip.content {
                Content::Grade { temperature } => {
                    if !self
                        .s
                        .functions
                        .iter()
                        .any(|f| f.starts_with("const GRADE"))
                    {
                        self.s
                            .functions
                            .insert(0, format!("const GRADE = {};\n", js(&GRADE)));
                    }
                    let grade = format!(
                        "<ShaderLayer code={{GRADE}} uniforms={{{{ temperature: {} }}}} style={{{{ position: \"absolute\", inset: 0 }}}} />",
                        num(*temperature)
                    );
                    (same_time(), Vec::new(), grade)
                }
                Content::Motion(c) => {
                    let w = format!("{at}.component");
                    let (d, name) = self.component(&c.component, &w)?;
                    if d.role != Role::Effect {
                        return Err(format!(
                            "{w}: {} is not an effect(); an adjustment declares `export const \
                             role = effect()` and reads what is under it as input://source",
                            c.component
                        ));
                    }
                    let mut scope = Scope::default();
                    let given = self.given(c, &d, at, None, &mut scope)?;
                    let shows = format!("K{}", self.s.functions.len());
                    self.s.functions.push(format!(
                        "function {shows}() {{\n  const t = useCurrentTime().clipSeconds;\n  return (\n    <View style={{{{ position: \"absolute\", inset: 0 }}}}>\n      <{name} {{...{given}}} />\n    </View>\n  );\n}}\n"
                    ));
                    let mut props = self.role_props(&d, clip.duration, &scope);
                    props.push(format!("inputs={{{{ source: {} }}}}", js(&picture)));
                    let time = ClipTime::Role {
                        role: d.role,
                        duration: d.composition.duration,
                    };
                    (time, props, format!("<{shows} />"))
                }
                _ => unreachable!("an adjustment is a grade or an effect: {at}"),
            };
            let table = template::Clip {
                start: clip.start,
                end: clip.end(),
                on: [clip.start, clip.end()],
                time,
                frames: None,
            };
            writeln!(
                out,
                "        {}",
                self.scope(&clip.id, &clip.id, table, &props, fill, &inside)
            )
            .unwrap();
        }
        out.push_str("      </View>\n");
        Ok(out)
    }

    /// The body of a clip's component: its box where `position`, `size`, `anchor`, `scale` and
    /// `rotation` put it, at its `opacity` and `blend` once `finished`, and `content` in it. `t`
    /// is the clip's time, which keyframes are over.
    fn look(
        &self,
        clip: &Clip,
        lets: &str,
        content: &str,
        finished: bool,
    ) -> Result<String, Error> {
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
        if finished {
            style.extend(self.finish(clip)?);
        }
        writeln!(
            out,
            "  return (\n    <View style={{{{ {} }}}}>\n      {content}\n    </View>\n  );",
            style.join(", ")
        )
        .unwrap();
        Ok(out)
    }

    /// The style of a clip's `opacity` and `blend`, which go after its effects.
    fn finish(&self, clip: &Clip) -> Result<Vec<String>, Error> {
        let mut style = Vec::new();
        if let Some(v) = &clip.look.opacity {
            let o = curve(v, &format!("{}.opacity", clip.at), &|v, w| {
                let x = v.as_f64().ok_or_else(|| format!("{w}: a number"))?;
                match (0.0..=1.0).contains(&x) {
                    true => Ok(Js::Number(x)),
                    false => Err(format!("{w}: from 0 to 1, not {x}")),
                }
            })?;
            style.push(format!("opacity: {o}"));
        }
        if let Some(b) = &clip.look.blend {
            style.push(format!("mixBlendMode: {}", js(b)));
        }
        Ok(style)
    }

    /// A color written in the document, checked.
    fn color(&self, color: &str, at: &str) -> Result<(), Error> {
        crate::style::color::parse(color)
            .map(drop)
            .map_err(|e| format!("{at}: {e}"))
    }

    /// A motion clip's component, given its props, data and resources, and fitted into the
    /// clip's box, or laid out at its size when it is an overlay.
    fn motion(
        &mut self,
        clip: &Clip,
        c: &Component,
        reads: [Tick; 2],
        scope: &mut Scope,
    ) -> Result<String, Error> {
        let at = clip.at.as_str();
        let (d, name) = self.component(&c.component, &format!("{at}.component"))?;
        let props = self.given(c, &d, at, None, scope)?;
        let composition = d.composition;
        match d.role {
            Role::Clip => {
                let length = Some(composition.duration);
                scope.time = Some(source_time(clip, reads, length, &c.component)?);
                scope.composition = Some(format!(
                    "{{ width: {}, height: {}, duration: {}, clipDuration: {} }}",
                    composition.width,
                    composition.height,
                    composition.duration.seconds_text(),
                    clip.duration.seconds_text()
                ));
                let [cw, ch] = [composition.width, composition.height];
                writeln!(scope.lets, "  const fit = [{}];", fit_scale(clip, [cw, ch])).unwrap();
                Ok(format!(
                    "<View style={{{{ position: \"absolute\", inset: 0, overflow: \"hidden\" }}}}><View style={{{{ position: \"absolute\", left: (size[0] - {cw} * fit[0]) / 2, top: (size[1] - {ch} * fit[1]) / 2, width: {cw}, height: {ch}, transformOrigin: \"0 0\", transform: `scale(${{fit[0]}}, ${{fit[1]}})`, overflow: \"hidden\" }}}}><{name} {{...{props}}} /></View></View>"
                ))
            }
            Role::Overlay { .. } => {
                for (k, given) in [
                    ("fit", clip.look.fit.is_some()),
                    ("end", clip.source.end != End::Error),
                    (
                        "trimStart or rate",
                        !matches!(&clip.source.time, SourceTime::Linear { trim, rate } if *trim == Tick::ZERO && *rate == Speed::ONE),
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

/// A clip as drawn: the component that draws it, the props of its TimeScopes, and its time.
struct Drawn {
    shows: String,
    props: Vec<String>,
    time: ClipTime,
    effects: Vec<Effect>,
}

/// One of a clip's effects as drawn: the component that draws it, the props of its TimeScope,
/// and its time.
struct Effect {
    shows: String,
    props: Vec<String>,
    time: ClipTime,
}

/// The time of a TimeScope that gives what is in it the time around it.
fn same_time() -> ClipTime {
    ClipTime::Source {
        trim: Tick::ZERO,
        rate: Speed::ONE,
        map: Vec::new(),
        length: None,
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

/// How a clip scales content `w` × `h` into its box, `size`, as its fit says: x and y, as JS.
fn fit_scale(clip: &Clip, [w, h]: [u32; 2]) -> String {
    match clip.look.fit.unwrap_or(Fit::Contain) {
        Fit::Contain => {
            let s = format!("Math.min(size[0] / {w}, size[1] / {h})");
            format!("{s}, {s}")
        }
        Fit::Cover => {
            let s = format!("Math.max(size[0] / {w}, size[1] / {h})");
            format!("{s}, {s}")
        }
        Fit::Fill => format!("size[0] / {w}, size[1] / {h}"),
        Fit::None => "1, 1".into(),
    }
}

/// A source's time in a clip: its trim and rate or its time map, held or looped past `length`
/// as the clip's `end` says; with `end: "error"`, an error when the clip reads past it.
fn source_time(
    clip: &Clip,
    reads: [Tick; 2],
    length: Option<Tick>,
    what: &str,
) -> Result<ClipTime, Error> {
    let at = &clip.at;
    let (trim, rate, map) = match &clip.source.time {
        SourceTime::Linear { trim, rate } => (*trim, *rate, Vec::new()),
        SourceTime::Map(points) => (Tick::ZERO, Speed::ONE, points.clone()),
    };
    // How far into the source the clip reads, from where, and whether it reads that far tick
    // itself; its handles in a transition too.
    let (from, reach, inclusive) = match map.as_slice() {
        [] => (
            trim + rate.apply(reads[0])?,
            trim + rate.apply(reads[1])?,
            false,
        ),
        points => (
            points.iter().map(|p| p.1).min().unwrap_or_default(),
            points.iter().map(|p| p.1).max().unwrap_or_default(),
            true,
        ),
    };
    if clip.source.end == End::Error && from < Tick::ZERO {
        return Err(format!(
            "{at}: the transition before it reads {what} from {} s, before its start; trim \
             later into it, or set end to \"hold\" or \"loop\"",
            from.seconds_text()
        ));
    }
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
