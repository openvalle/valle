//! A Timeline document lowered to the Motion scene it plays as, so an edit runs through the same
//! frontend, engine and renderer as any scene, on one clock. The scene lives in memory only.
//!
//! - A clip is a `<TimeScope clip>`: a box that shows while the clip is on, the clip's content
//!   inside on the clip's own time. When it is on and what time it gives come from the clip
//!   table, in ticks. Only active clips build content; their visibility guards select a new
//!   variant when clips come and go, including when seeking backwards.
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
const GRADE: &str = include_str!("../shaders/temperature.wgsl");

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
        if let Some(origin) = scope.origin {
            props.push(format!("origin={}", js(&origin)));
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

    /// Resolve font resources and check caption styles against the template's controls.
    /// Runs share the text controls; placement and animation belong to the whole caption.
    fn caption_style(
        &mut self,
        style: &Map<String, Value>,
        d: &Declared,
        at: &str,
        run: bool,
    ) -> Result<Map<String, Value>, Error> {
        let mut out = Map::new();
        for (k, v) in style {
            let w = format!("{at}.{k}");
            if run
                && matches!(
                    k.as_str(),
                    "background"
                        | "backgroundColor"
                        | "position"
                        | "margin"
                        | "maxWidth"
                        | "animation"
                        | "transition"
                        | "textAlign"
                )
            {
                return Err(format!(
                    "{w}: {k} belongs to the caption track's style, not a run"
                ));
            }
            if k == "font" {
                let alias = v
                    .as_str()
                    .ok_or_else(|| format!("{w}: the alias of a font resource"))?;
                let files = self.files(alias, &w)?;
                out.insert(
                    "fontFamily".into(),
                    Value::String(self.resource(Holds::Font, files)),
                );
                continue;
            }
            let p = d
                .props
                .iter()
                .find(|p| p.name == *k && k != "fontFamily")
                .ok_or_else(|| format!("{w}: a caption style has no {k}"))?;
            let value =
                front::controls::check(&p.kind, v.clone()).map_err(|e| format!("{w}: {e}"))?;
            out.insert(k.clone(), value);
        }
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
                if let Some(style) = &track.style {
                    c.props = self
                        .caption_style(style, &d, &format!("{at}.style"), false)?
                        .into_iter()
                        .map(|(k, v)| (k, Animated::Constant(v)))
                        .collect();
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
            let runs: Vec<Value> = runs
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut run = serde_json::json!({ "text": r.text,
                    "start": r.start.seconds(), "end": r.end.seconds() });
                    if !r.style.is_empty() {
                        let style = if made.is_some() {
                            self.caption_style(
                                &r.style,
                                &d,
                                &format!("{}.runs[{i}].style", clip.at),
                                true,
                            )?
                        } else {
                            r.style.clone()
                        };
                        run["style"] = Value::Object(style);
                    }
                    Ok(run)
                })
                .collect::<Result<_, Error>>()?;
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
        scope.origin = Some("~canvas");
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
                    "<View style={{{{ position: \"absolute\", inset: 0, overflow: \"hidden\" }}}}><View key=\"~canvas\" style={{{{ position: \"absolute\", left: (size[0] - {cw} * fit[0]) / 2, top: (size[1] - {ch} * fit[1]) / 2, width: {cw}, height: {ch}, transformOrigin: \"0 0\", transform: `scale(${{fit[0]}}, ${{fit[1]}})`, overflow: \"hidden\" }}}}><{name} {{...{props}}} /></View></View>"
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
                    "<View key=\"~canvas\" style={{{{ position: \"absolute\", inset: 0 }}}}><{name} {{...{props}}} /></View>"
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
    /// The fitted canvas whose origin bounds() uses, relative to the clip's key path.
    origin: Option<&'static str>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Item;
    use crate::scene::{Frame, Scene};
    use crate::template::Kind as NodeKind;
    use serde_json::json;

    fn run(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(test)
            .unwrap()
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p));
    }

    fn caption(style: Value, clips: Value) -> Scene {
        let mut props = json!({ "fontSize": 28, "margin": 24, "maxWidth": 360,
            "background": "none", "shadow": false, "transition": 0.2 });
        props
            .as_object_mut()
            .unwrap()
            .extend(style.as_object().unwrap().clone());
        scene(
            "captions.timeline.json",
            &json!({
                "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "caption": [{ "style": props, "clips": clips }] }
            }),
            &|_| None,
        )
    }

    fn scene(entry: &str, doc: &Value, read: &dyn Fn(&str) -> Option<String>) -> Scene {
        let doc = Document::read(doc).unwrap();
        let lowered = lower(
            entry,
            &doc,
            &Host {
                read,
                length: &|_| None,
            },
        )
        .unwrap();
        Scene::load(
            &lowered.entry,
            &|path| {
                lowered
                    .files
                    .get(path)
                    .cloned()
                    .or_else(|| read(path))
                    .map(String::into_bytes)
            },
            &lowered.inputs,
            crate::testing::fonts(),
        )
        .unwrap()
    }

    fn fixture(name: &str) -> Scene {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/resources/timeline/captions");
        // Scene paths use `/`, as the CLI gives them, on Windows too.
        let entry = dir
            .join(format!("{name}.timeline.json"))
            .to_string_lossy()
            .replace('\\', "/");
        let read = |path: &str| std::fs::read_to_string(path).ok();
        let value = serde_json::from_str(&read(&entry).unwrap()).unwrap();
        scene(&entry, &value, &read)
    }

    fn frame(scene: &mut Scene, seconds: f64) -> Frame {
        scene.frame(Tick::from_seconds(seconds).unwrap()).unwrap()
    }

    /// Count painted letters, excluding glyphs suppressed by a parent or their own alpha.
    fn letters(items: &[Item], alpha: f32, color: Option<[f32; 4]>) -> usize {
        items
            .iter()
            .map(|item| match item {
                Item::Glyphs {
                    glyphs, color: c, ..
                } if alpha * c[3] > 0.0 && color.is_none_or(|color| color == *c) => glyphs.len(),
                Item::Group { opacity, items, .. } => letters(items, alpha * opacity, color),
                Item::Transform { items, .. } | Item::Clip { items, .. } => {
                    letters(items, alpha, color)
                }
                _ => 0,
            })
            .sum()
    }

    /// The standard caption's outer box, found from its text rather than a generated node ID.
    fn outer(frame: &Frame) -> usize {
        let text = frame.layout.lines.iter().position(Option::is_some).unwrap();
        parent(frame, parent(frame, text))
    }

    fn parent(frame: &Frame, i: usize) -> usize {
        frame
            .template
            .nodes
            .iter()
            .position(|n| n.children.contains(&(i as u32)))
            .unwrap()
    }

    #[test]
    fn a_motion_clip_draws_inside_its_box() {
        run(|| {
            let motion = r#"
                export const composition = {width:320,height:180,fps:30,duration:4};
                export default function Scene() {
                    return <View>
                        <View key="target" style={{position:'absolute',left:40,top:50,width:80,height:40}} />
                    </View>;
                }
            "#;
            let doc = json!({
                "canvas":{"width":640,"height":360,"fps":30},
                "resources":{"scene":"scene.motion.tsx"},
                "tracks":{"visual":[{"clips":[
                    {"id":"a","kind":"motion","component":"scene","start":0,"duration":2,"position":[180,100],"size":[160,90]}
                ]}]}
            });
            let read = |path: &str| (path == "scene.motion.tsx").then(|| motion.to_string());
            let mut s = scene("edit.json", &doc, &read);
            let f = frame(&mut s, 0.5);
            let nodes = &f.template.nodes;
            let target = (nodes.iter())
                .position(|n| n.path.as_deref().is_some_and(|p| p.ends_with("/target")))
                .unwrap();
            let path = nodes[target].path.as_deref().unwrap();
            let mut ancestors = Vec::new();
            let mut i = target as u32;
            while let Some(p) = nodes.iter().position(|n| n.children.contains(&i)) {
                ancestors.push(p);
                i = p as u32;
            }
            // Every keyed box on the target's key path holds it, the clip's own box included: a
            // picture or effect of the clip reads that box.
            let mut boxes = 0;
            for (j, n) in nodes.iter().enumerate() {
                if let Some(p) = n.path.as_deref()
                    && path.starts_with(&format!("{p}/"))
                {
                    assert!(ancestors.contains(&j), "{path} is outside {p}");
                    boxes += 1;
                }
            }
            assert!(boxes > 0, "{path}");
        });
    }

    #[test]
    fn bounds_and_anchors_stay_local_to_each_motion_instance() {
        run(|| {
            let motion = r#"
                export const composition = {width:320,height:180,fps:30,duration:4};
                export default function Scene() {
                    const b = bounds('target');
                    const p = anchor('target', 'top-left');
                    return <View>
                        <View key="target" style={{position:'absolute',left:40,top:50,width:80,height:40}} />
                        <View key="marker" style={{position:'absolute',left:0,top:0,width:80,height:40,
                            transformOrigin:'0 0',translate:`${p.x}px ${p.y}px`,scale:`${b.width/80} ${b.height/40}`}} />
                    </View>;
                }
            "#;
            let base = json!({
                "canvas":{"width":640,"height":360,"fps":30},
                "resources":{"scene":"scene.motion.tsx"},
                "tracks":{"visual":[{"clips":[
                    {"id":"a","kind":"motion","component":"scene","start":0,"duration":2,"position":[180,100],"size":[320,180]}
                ]}, {"clips":[
                    {"id":"b","kind":"motion","component":"scene","start":0,"duration":2,"position":[470,240],"size":[200,90]}
                ]}]}
            });
            let nested = json!({"canvas":base["canvas"],"resources":{"part":"part.json"},
                "tracks":{"visual":[{"clips":[{"id":"nested","kind":"timeline","src":"part","start":0,"duration":2}]}]}});
            let transition = json!({"canvas":base["canvas"],"resources":base["resources"],
            "tracks":{"visual":[{"items":[
                {"id":"a","kind":"motion","component":"scene","duration":1},
                {"transition":"fade","inOffset":0.1,"outOffset":0.1},
                {"id":"b","kind":"motion","component":"scene","duration":1,"trimStart":0.2}
            ]}]}});
            let read = |path: &str| match path {
                "scene.motion.tsx" => Some(motion.to_string()),
                "part.json" => Some(base.to_string()),
                _ => None,
            };
            for doc in [&base, &nested, &transition] {
                let mut s = scene("edit.json", doc, &read);
                for time in [0.0, 0.95, 1.05, 1.5, 0.5] {
                    let f = frame(&mut s, time);
                    let mut count = 0;
                    for (i, node) in f.template.nodes.iter().enumerate() {
                        let Some(prefix) =
                            node.path.as_deref().and_then(|p| p.strip_suffix("/target"))
                        else {
                            continue;
                        };
                        let marker = format!("{prefix}/marker");
                        let j = f
                            .template
                            .nodes
                            .iter()
                            .position(|n| n.path.as_deref() == Some(&marker))
                            .unwrap();
                        let a = f.transforms[i].bounds(f.layout.boxes[i].border);
                        let b = f.transforms[j].bounds(f.layout.boxes[j].border);
                        assert!(
                            (a.x - b.x).abs() < 0.01
                                && (a.y - b.y).abs() < 0.01
                                && (a.width - b.width).abs() < 0.01
                                && (a.height - b.height).abs() < 0.01,
                            "{marker} at {time}: {a:?}, {b:?}"
                        );
                        count += 1;
                    }
                    assert!(count > 0);
                }
            }
        });
    }

    #[test]
    fn caption_runs_light_or_reveal_only_at_their_starts_and_seek_backwards() {
        run(|| {
            for animation in ["karaoke", "typewriter"] {
                let mut scene = caption(
                    json!({ "animation": animation, "highlight": "#00ff00" }),
                    json!([{ "start": 0.2, "duration": 1.2, "runs": [
                        { "text": "AB", "start": 0, "end": 0.3 },
                        { "text": "CD", "start": 0.4, "end": 0.7 },
                        { "text": "EF", "start": 0.8, "end": 1.2 }
                    ] }]),
                );
                for (time, active) in [
                    (0.1, 0),
                    (0.2, 2),
                    (0.59, 2),
                    (0.6, 4),
                    (0.99, 4),
                    (1.0, 6),
                    (1.39, 6),
                    (1.4, 0),
                    (0.6, 4),
                    (0.2, 2),
                ] {
                    let f = frame(&mut scene, time);
                    let color = (animation == "karaoke").then_some([0.0, 1.0, 0.0, 1.0]);
                    assert_eq!(
                        letters(&f.items, 1.0, color),
                        active,
                        "{animation} at {time}"
                    );
                    if animation == "karaoke" && active > 0 {
                        assert_eq!(letters(&f.items, 1.0, None), 6);
                    }
                }
            }
        });
    }

    #[test]
    fn caption_run_styles_inherit_override_and_reset_text_properties() {
        run(|| {
            let mut scene = caption(
                json!({ "fontSize": 28, "fontWeight": 500, "fontStyle": "italic",
                    "lineHeight": 1.8, "letterSpacing": 2, "wordSpacing": 3,
                    "strokeWidth": 2, "strokeColor": "blue", "shadow": true,
                    "textShadow": "2px 3px 4px red", "textDecoration": "underline",
                    "textUnderlineOffset": "12px", "textDecorationSkipInk": "none" }),
                json!([{ "start": 0, "duration": 3, "runs": [
                    { "text": "AB ", "start": 0, "end": 1 },
                    { "text": "CD ", "start": 1, "end": 2, "style": {
                        "fontSize": 40, "fontWeight": 800, "fontStyle": "normal", "textColor": "red",
                        "lineHeight": 1.2, "letterSpacing": 0, "wordSpacing": 1,
                        "strokeWidth": 4, "strokeColor": "lime", "shadow": false,
                        "textDecoration": "line-through", "textUnderlineOffset": "0.2em",
                        "textDecorationSkipInk": "all" } },
                    { "text": "EF", "start": 2, "end": 3,
                      "style": { "strokeWidth": 0, "textDecoration": "none" } }
                ] }]),
            );
            let f = frame(&mut scene, 0.5);
            let (_, lines) = caption_lines(&f, &["AB CD EF"]);
            for (cluster, size, weight, spacing, stroke, shadow, decoration) in [
                (0, 28.0, 500.0, 2.0, 2.0, true, Some([true, false, false])),
                (3, 40.0, 800.0, 0.0, 4.0, false, Some([false, false, true])),
                (6, 28.0, 500.0, 2.0, 0.0, true, None),
            ] {
                let g = lines.glyphs.iter().find(|g| g.cluster == cluster).unwrap();
                let s = &f.styles[g.node as usize];
                assert_eq!(s.font.size, size);
                assert_eq!(s.font.weight, weight);
                assert_eq!(s.font.spacing, spacing);
                assert_eq!(s.font.stroke, stroke);
                assert_eq!(
                    s.font
                        .underline_offset
                        .as_ref()
                        .unwrap()
                        .resolve(size as f32),
                    if cluster == 3 { 8.0 } else { 12.0 }
                );
                assert_eq!(
                    s.font.skip_ink,
                    if cluster == 3 {
                        crate::style::SkipInk::All
                    } else {
                        crate::style::SkipInk::None
                    }
                );
                assert_eq!(s.font.shadows.len(), usize::from(shadow));
                assert_eq!(s.decorations.last().map(|d| d.lines), decoration);
                assert_eq!(s.font.word_spacing, if cluster == 3 { 1.0 } else { 3.0 });
                assert_eq!(
                    s.font.line_height,
                    crate::style::LineHeight::Factor(if cluster == 3 { 1.2 } else { 1.8 })
                );
                assert_eq!(
                    s.font.style,
                    if cluster == 3 {
                        crate::style::FontStyle::Normal
                    } else {
                        crate::style::FontStyle::Italic
                    }
                );
                if cluster == 3 {
                    assert_eq!(s.color, [1.0, 0.0, 0.0, 1.0]);
                    assert_eq!(s.font.stroke_color, Some([0.0, 1.0, 0.0, 1.0]));
                } else {
                    assert_eq!(s.font.shadows[0].offset, [2.0, 3.0]);
                    assert_eq!(s.font.shadows[0].blur, 4.0);
                }
            }
        });
    }

    #[test]
    fn plain_captions_do_not_restart_decorations_at_each_character() {
        run(|| {
            for animation in ["none", "fade", "pop", "slide"] {
                let mut scene = caption(
                    json!({ "animation": animation, "fontSize": 40,
                        "textDecoration": "underline wavy 2px", "textUnderlineOffset": "12px" }),
                    json!([{ "text": "强调 Valle gyp", "start": 0, "duration": 2 }]),
                );
                let f = frame(&mut scene, 0.5);
                let (_, lines) = caption_lines(&f, &["强调 Valle gyp"]);
                let along = [
                    lines.runs.first().unwrap().x[0],
                    lines.runs.last().unwrap().x[1],
                ];
                assert!(!lines.decorations.is_empty());
                assert!(
                    lines.decorations.iter().all(|d| d.along == along),
                    "{animation}: every character must share the same decorating box"
                );
            }
        });
    }

    #[test]
    fn caption_run_style_errors_name_the_field_even_on_later_clips() {
        for (style, field) in [
            (json!({ "fontSize": -1 }), "fontSize"),
            (json!({ "fontWeight": 1001 }), "fontWeight"),
            (json!({ "textColor": "bad-color" }), "textColor"),
            (json!({ "shadow": 1 }), "shadow"),
            (json!({ "fontStyle": "bad-style" }), "fontStyle"),
            (json!({ "strokeWidth": -1 }), "strokeWidth"),
            (json!({ "animation": "pop" }), "animation"),
            (json!({ "bogus": true }), "bogus"),
        ] {
            let value = json!({ "canvas": { "width": 640, "height": 360, "fps": 30 },
                "tracks": { "caption": [{ "clips": [
                    { "text": "valid", "start": 0, "duration": 1 },
                    { "start": 20, "duration": 1, "runs": [
                        { "text": "later", "start": 0, "end": 1, "style": style }
                    ] }
                ] }] } });
            let doc = Document::read(&value).unwrap();
            let Err(error) = lower(
                "test.timeline.json",
                &doc,
                &Host {
                    read: &|_| None,
                    length: &|_| None,
                },
            ) else {
                panic!("accepted {style}");
            };
            assert!(
                error.contains(&format!("clips[1].runs[0].style.{field}")),
                "{error}"
            );
        }
    }

    #[test]
    fn styled_caption_runs_keep_layout_when_highlighting_revealing_and_seeking() {
        run(|| {
            let mut scene = fixture("styled");
            for start in [2.0, 4.0] {
                let last = frame(&mut scene, start + 59.0 / 30.0);
                let i = last.layout.lines.iter().position(Option::is_some).unwrap();
                let reference = last.layout.lines[i].as_ref().unwrap();
                for offset in [0.0, 0.5, 0.6, 1.0, 1.3, 1.8, 0.6, 0.0] {
                    let f = frame(&mut scene, start + offset);
                    let j = f.layout.lines.iter().position(Option::is_some).unwrap();
                    let lines = f.layout.lines[j].as_ref().unwrap();
                    assert_eq!(lines, reference, "at {}", start + offset);
                    assert_eq!(f.layout.boxes[j], last.layout.boxes[i]);
                    if start == 2.0 {
                        for (cluster, begins, before, after) in [
                            (0, 0.0, [148, 163, 184], [244, 114, 182]),
                            (7, 0.6, [100, 116, 139], [52, 211, 153]),
                            (14, 1.3, [255, 255, 255], [251, 191, 36]),
                        ] {
                            let glyph = lines.glyphs.iter().find(|g| g.cluster == cluster).unwrap();
                            let [r, g, b] = if offset < begins { before } else { after };
                            assert_eq!(
                                f.styles[glyph.node as usize].color,
                                [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0]
                            );
                        }
                    }
                    if start == 4.0 {
                        // A whole timed run, including its spaces' decorations, is hidden.
                        for (cluster, begins) in [(0, 0.0), (11, 0.6), (23, 1.3)] {
                            let g = lines.glyphs.iter().find(|g| g.cluster == cluster).unwrap();
                            assert_eq!(f.styles[g.node as usize].visible, offset >= begins);
                        }
                        assert_eq!(lines.text.len(), 2);
                        for word in ["Alpha", "beta", "gamma", "delta", "👩🏽‍💻"] {
                            assert!(
                                lines
                                    .text
                                    .iter()
                                    .any(|r| lines.string[r.clone()].contains(word))
                            );
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn caption_run_fonts_resolve_from_timeline_resources() {
        let doc = Document::read(&json!({
            "canvas": { "width": 640, "height": 360, "fps": 30 },
            "resources": { "body": "body.ttf", "accent": ["regular.ttf", "italic.ttf"] },
            "tracks": { "caption": [{ "style": { "font": "body" }, "clips": [
                { "start": 0, "duration": 1, "runs": [
                    { "text": "Hello", "start": 0, "end": 1, "style": { "font": "accent" } }
                ] }
            ] }] }
        }))
        .unwrap();
        let lowered = lower(
            "edit/timeline.json",
            &doc,
            &Host {
                read: &|_| None,
                length: &|_| None,
            },
        )
        .unwrap();
        assert_eq!(lowered.inputs.resources.len(), 2);
        assert!(
            lowered
                .inputs
                .resources
                .values()
                .any(|f| f == &["edit/body.ttf"])
        );
        assert!(
            lowered
                .inputs
                .resources
                .values()
                .any(|f| f == &["edit/regular.ttf", "edit/italic.ttf"])
        );
    }

    #[test]
    fn custom_caption_presenters_receive_run_styles_without_interpretation() {
        run(|| {
            let mut scene = scene(
                "edit.timeline.json",
                &json!({
                    "canvas": { "width": 640, "height": 360, "fps": 30 },
                    "resources": { "presenter": "words.tsx" },
                    "tracks": { "caption": [{ "presenter": { "component": "presenter" }, "clips": [
                        { "start": 0, "duration": 1, "runs": [
                            { "text": "A", "start": 0, "end": 1,
                              "style": { "ink": "red", "weight": 750 } }
                        ] }
                    ] }] }
                }),
                &|path| {
                    (path == "words.tsx").then(|| {
                        r#"
                export const composition = { width: 640, height: 360, fps: 30, duration: 1 };
                export const role = captionPresenter();
                export default function Words({ caption }) {
                  return <Text>{caption.runs.map((r, i) =>
                    <Span key={i} style={{ color: r.style.ink, fontWeight: r.style.weight }}>
                      {r.text}
                    </Span>
                  )}</Text>;
                }
            "#
                        .into()
                    })
                },
            );
            let f = frame(&mut scene, 0.5);
            let (_, lines) = caption_lines(&f, &["A"]);
            let s = &f.styles[lines.glyphs[0].node as usize];
            assert_eq!(s.color, [1.0, 0.0, 0.0, 1.0]);
            assert_eq!(s.font.weight, 750.0);
        });
    }

    #[test]
    fn caption_fade_pop_slide_and_none_have_their_expected_entry_and_exit() {
        run(|| {
            for animation in ["none", "fade", "pop", "slide"] {
                let mut scene = caption(
                    json!({ "animation": animation }),
                    json!([{ "text": "AB", "start": 0.2, "duration": 1.2 }]),
                );
                let still = frame(&mut scene, 0.4);
                let k = outer(&still);
                let rest = still.transforms[k].bounds(still.layout.boxes[k].border);
                for (time, opacity) in [(0.2, 0.0), (0.3, 0.5), (0.4, 1.0), (1.3, 0.5)] {
                    let f = frame(&mut scene, time);
                    let k = outer(&f);
                    let expected = if animation == "none" { 1.0 } else { opacity };
                    assert!(
                        (f.styles[k].opacity - expected).abs() < 1e-6,
                        "{animation} at {time}"
                    );
                    let b = f.transforms[k].bounds(f.layout.boxes[k].border);
                    assert!((b.x + b.width / 2.0 - 320.0).abs() < 0.02);
                    if animation == "slide" {
                        assert!((b.y - rest.y - (1.0 - opacity as f32) * 36.0).abs() < 0.02);
                    } else if animation == "pop" && time == 0.2 {
                        assert!((b.width / rest.width - 0.82).abs() < 0.001);
                    } else if animation == "pop" && time == 0.3 {
                        assert!(b.width > rest.width, "pop overshoots before settling");
                    } else {
                        assert!((b.width - rest.width).abs() < 0.02);
                    }
                }
                assert_eq!(letters(&frame(&mut scene, 1.4).items, 1.0, None), 0);
            }
        });
    }

    #[test]
    fn caption_multiline_blocks_are_centered_and_wrapped_text_fits_its_box() {
        run(|| {
            for text in [
                "One line",
                "First line\nSecond line",
                "中文与 English 123，一起检查长句自动换行，保持标点和阅读顺序。",
            ] {
                for position in ["top", "middle", "bottom"] {
                    let mut scene = caption(
                        json!({ "position": position, "background": "box" }),
                        json!([{ "text": text, "start": 0, "duration": 1 }]),
                    );
                    let f = frame(&mut scene, 0.5);
                    let k = outer(&f);
                    let b = f.transforms[k].bounds(f.layout.boxes[k].border);
                    let got = match position {
                        "top" => b.y,
                        "middle" => b.y + b.height / 2.0,
                        _ => b.y + b.height,
                    };
                    let expected = match position {
                        "top" => 24.0,
                        "middle" => 180.0,
                        _ => 336.0,
                    };
                    // Percentage transforms use the pixel-snapped border box. Its center may
                    // differ from the unrounded layout center by at most half a pixel.
                    let tolerance = if position == "middle" { 0.51 } else { 0.02 };
                    assert!(
                        (got - expected).abs() < tolerance,
                        "{position} {text:?}: {b:?}"
                    );
                    let (i, lines) = f
                        .layout
                        .lines
                        .iter()
                        .enumerate()
                        .find_map(|(i, l)| l.as_ref().map(|l| (i, l)))
                        .unwrap();
                    let content = f.layout.boxes[i].content;
                    assert!(content.width <= 300.0);
                    if text != "One line" {
                        assert!(lines.text.len() >= 2);
                    }
                    assert!(
                        lines
                            .runs
                            .iter()
                            .all(|run| run.x[0] >= -0.02 && run.x[1] <= content.width + 0.02)
                    );
                }
            }
        });
    }

    fn caption_lines<'a>(frame: &'a Frame, expected: &[&str]) -> (usize, &'a crate::text::Lines) {
        let (i, lines) = frame
            .layout
            .lines
            .iter()
            .enumerate()
            .find_map(|(i, lines)| lines.as_ref().map(|lines| (i, lines)))
            .unwrap();
        let text: Vec<_> = lines
            .text
            .iter()
            .map(|r| &lines.string[r.clone()])
            .collect();
        assert_eq!(text, expected);
        (i, lines)
    }

    #[test]
    fn caption_wraps_english_words_whole_in_plain_and_mixed_text() {
        run(|| {
            // The 220px box has 30px padding on each side: 160px for text. The English
            // case uses the same font, size and width as css/text/word-wrap.
            for (text, expected) in [
                ("Alpha beta gamma delta", ["Alpha beta ", "gamma delta"]),
                ("你好世界 English go", ["你好世界 ", "English go"]),
            ] {
                let mut scene = caption(
                    json!({ "fontSize": 24, "fontWeight": 400, "maxWidth": 220,
                        "background": "box", "animation": "none" }),
                    json!([{ "text": text, "start": 0, "duration": 1.2 }]),
                );
                let f = frame(&mut scene, 0.5);
                let (i, lines) = caption_lines(&f, &expected);
                assert!((f.layout.boxes[i].content.width - 160.0).abs() < 0.02);
                assert!(
                    lines
                        .runs
                        .iter()
                        .all(|r| r.x[0] >= -0.02 && r.x[1] <= 160.02)
                );
            }
        });
    }

    #[test]
    fn caption_word_animations_keep_wrapping_and_glyph_positions() {
        run(|| {
            for (words, expected) in [
                (
                    vec!["Alpha ", "beta ", "gamma ", "delta"],
                    ["Alpha beta ", "gamma delta"],
                ),
                (
                    vec!["你好世界 ", "English ", "go"],
                    ["你好世界 ", "English go"],
                ),
            ] {
                for animation in ["karaoke", "typewriter"] {
                    for timed in [false, true] {
                        let mut clip = json!({ "start": 0, "duration": 1.2 });
                        if timed {
                            clip["runs"] = Value::Array(
                                words
                                    .iter()
                                    .enumerate()
                                    .map(|(i, text)| {
                                        json!({
                                            "text": text,
                                            "start": i as f64 * 0.3,
                                            "end": (i + 1) as f64 * 0.3
                                        })
                                    })
                                    .collect(),
                            );
                        } else {
                            clip["text"] = json!(words.concat());
                        }
                        let mut scene = caption(
                            json!({ "fontSize": 24, "fontWeight": 400, "maxWidth": 220,
                                "background": "box", "animation": animation }),
                            json!([clip]),
                        );
                        let last = frame(&mut scene, 35.0 / 30.0);
                        let (i, reference) = caption_lines(&last, &expected);
                        // Before and on run starts, fully revealed, and after seeking back.
                        for index in [0, 8, 9, 17, 18, 26, 27, 35, 18, 0] {
                            let f = frame(&mut scene, index as f64 / 30.0);
                            let (j, lines) = caption_lines(&f, &expected);
                            assert_eq!(
                                lines, reference,
                                "{animation}, timed={timed}, frame {index}"
                            );
                            assert_eq!(f.layout.boxes[j], last.layout.boxes[i]);
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn caption_keeps_an_overlong_word_whole_with_default_wrapping() {
        run(|| {
            let word = "Supercalifragilisticexpialidocious";
            let mut scene = caption(
                json!({ "fontSize": 24, "fontWeight": 400, "maxWidth": 220,
                    "background": "box", "animation": "none" }),
                json!([{ "text": word, "start": 0, "duration": 1 }]),
            );
            let f = frame(&mut scene, 0.5);
            let (i, lines) = caption_lines(&f, &[word]);
            let content = f.layout.boxes[i].content;
            assert!((content.width - 160.0).abs() < 0.02);
            // The template leaves word-break/overflow-wrap at normal. Overflow is visible;
            // it must not silently introduce break-all or discard part of the word.
            let left = lines
                .runs
                .iter()
                .map(|r| r.x[0])
                .fold(f32::INFINITY, f32::min);
            let right = lines
                .runs
                .iter()
                .map(|r| r.x[1])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(right - left > content.width);
            assert_eq!(letters(&f.items, 1.0, None), word.len());
        });
    }

    #[test]
    fn caption_layout_applies_background_stroke_shadow_and_keeps_emoji_whole() {
        run(|| {
            let mut scene = fixture("layout");
            for (time, size, stroke, shadow, background) in [
                (0.5, 28.0, 0.0, false, "box"),
                (2.5, 28.0, 1.0, true, "none"),
                (4.5, 24.0, 0.0, true, "bar"),
            ] {
                let f = frame(&mut scene, time);
                let (i, lines) = f
                    .layout
                    .lines
                    .iter()
                    .enumerate()
                    .find_map(|(i, lines)| lines.as_ref().map(|l| (i, l)))
                    .unwrap();
                let font = &f.styles[i].font;
                assert_eq!(font.size, size);
                assert_eq!(font.stroke, stroke);
                assert_eq!(
                    f.styles[i].paint_order,
                    [
                        crate::style::PaintPart::Stroke,
                        crate::style::PaintPart::Fill,
                        crate::style::PaintPart::Markers,
                    ]
                );
                // Split characters inherit the caption's order too, so karaoke and
                // typewriter paint the same clean letter shapes as a plain caption.
                assert!(
                    lines
                        .runs
                        .iter()
                        .all(|r| f.styles[r.node as usize].paint_order == f.styles[i].paint_order)
                );
                assert_eq!(font.shadows.len(), usize::from(shadow));
                if shadow {
                    assert_eq!(font.shadows[0].offset, [0.0, 4.0]);
                    assert_eq!(font.shadows[0].blur, 14.0);
                }
                let box_index = parent(&f, i);
                let expected = if background == "none" {
                    0.0
                } else {
                    166.0 / 255.0
                };
                assert!((f.styles[box_index].background[3] - expected).abs() < 1e-6);
                if background == "bar" {
                    assert_eq!(f.layout.boxes[box_index].border.width, 640.0);
                    let emoji = "👩🏽‍💻";
                    let start = lines.string.find(emoji).unwrap();
                    let range = start..start + emoji.len();
                    let joined: Vec<_> = lines
                        .glyphs
                        .iter()
                        .filter(|g| range.contains(&(g.cluster as usize)))
                        .collect();
                    assert_eq!(joined.len(), 1, "the joined emoji is one glyph");
                    assert_eq!(joined[0].cluster as usize, start);
                    assert_ne!(joined[0].id, 0, "the emoji is not a missing-glyph box");
                    assert!(
                        lines
                            .text
                            .iter()
                            .any(|r| r.start <= range.start && r.end >= range.end)
                    );
                    let glyph = joined[0];
                    assert_eq!(
                        crate::text::color::colored(&scene.faces()[glyph.face as usize], glyph.id),
                        Ok(true)
                    );
                } else if background == "box" {
                    assert!(f.layout.boxes[box_index].border.width <= 360.0);
                } else {
                    caption_lines(&f, &["第一行 / line one\n", "第二行 / line two"]);
                }
            }
        });
    }

    #[test]
    fn caption_presenter_receives_runs_props_and_local_time_across_overlapping_tracks() {
        run(|| {
            let mut scene = fixture("boundaries");
            let checks = [
                (42, 0.0, 1),
                (45, 0.5, 1),
                (48, 1.0, 1),
                (56, 1.0, 1),
                (57, 1.0, 2),
                (63, 1.0, 2),
                (74, 1.0, 2),
                (75, 1.0, 3),
                (96, 1.0, 3),
                (99, 0.5, 3),
            ];
            for (index, opacity, lit) in checks.into_iter().chain(checks.into_iter().rev()) {
                let f = frame(&mut scene, index as f64 / 30.0);
                for (word, text) in ["你好 ", "👩🏽‍💻 ", "Valle!"].into_iter().enumerate()
                {
                    let i = f
                        .layout
                        .lines
                        .iter()
                        .position(|l| {
                            l.as_ref()
                                .is_some_and(|l| l.string.as_ref() == text.trim_end())
                        })
                        .unwrap();
                    let color = if word < lit {
                        [124.0, 58.0, 237.0, 255.0]
                    } else {
                        [51.0, 65.0, 85.0, 255.0]
                    }
                    .map(|c| c / 255.0);
                    assert_eq!(f.styles[i].background, color, "frame {index}, word {word}");
                    assert!((f.styles[parent(&f, i)].opacity - opacity).abs() < 1e-6);
                }
                if index == 63 {
                    let i = f
                        .layout
                        .lines
                        .iter()
                        .position(|l| {
                            l.as_ref()
                                .is_some_and(|l| l.string.as_ref() == "两个字幕轨同时出现")
                        })
                        .unwrap();
                    assert!((f.styles[parent(&f, parent(&f, i))].opacity - 0.5).abs() < 1e-6);
                }
            }
            for time in [0.1, 1.2, 1.3, 3.4, 3.9] {
                assert_eq!(letters(&frame(&mut scene, time).items, 1.0, None), 0);
            }
        });
    }

    #[test]
    fn adjacent_captions_exchange_at_the_exclusive_end_without_a_blank_frame() {
        run(|| {
            let mut scene = caption(
                json!({ "animation": "none" }),
                json!([
                    { "text": "A", "start": 0.2, "duration": 0.5 },
                    { "text": "BC", "start": 0.7, "duration": 0.5 }
                ]),
            );
            for (index, count) in [(5, 0), (6, 1), (20, 1), (21, 2), (35, 2), (36, 0), (20, 1)] {
                let at = scene.composition().fps.frame_start(index).unwrap();
                let f = scene.frame(at).unwrap();
                assert_eq!(letters(&f.items, 1.0, None), count, "frame {index}");
            }
        });
    }

    #[test]
    fn long_caption_tracks_build_only_the_visible_text_and_can_seek() {
        run(|| {
            let clips: Vec<_> = (0..1200)
                .map(|i| json!({ "text": format!("Caption {i}"), "start": i * 3, "duration": 2 }))
                .collect();
            let mut scene = caption(json!({ "animation": "none" }), json!(clips));
            // More variants than the cache keeps, then jump to the end and back.
            for i in (0..70).chain([599, 1199, 599, 0]) {
                let f = frame(&mut scene, (i * 3) as f64 + 0.5);
                let text: Vec<_> = f
                    .layout
                    .lines
                    .iter()
                    .flatten()
                    .map(|l| &*l.string)
                    .collect();
                assert_eq!(text, [format!("Caption {i}")]);
                assert_eq!(
                    f.template
                        .nodes
                        .iter()
                        .filter(|n| matches!(n.kind, NodeKind::Text { .. }))
                        .count(),
                    1,
                    "inactive captions must not retain their text trees"
                );
                assert!(f.template.nodes.len() < 32);
                // Reuse within a clip; a gap is empty, including on backwards seeks.
                let traces = scene.traces().0;
                frame(&mut scene, (i * 3) as f64 + 1.0);
                assert_eq!(scene.traces().0, traces);
                let gap = frame(&mut scene, (i * 3 + 2) as f64);
                assert_eq!(letters(&gap.items, 1.0, None), 0);
                assert!(
                    !gap.template
                        .nodes
                        .iter()
                        .any(|n| matches!(n.kind, NodeKind::Text { .. }))
                );
            }
        });
    }

    #[test]
    fn checking_a_later_caption_still_reports_its_presenter_error() {
        run(|| {
            let source = r#"
                export const composition = { width: 640, height: 360, fps: 30, duration: 3 };
                export const role = captionPresenter({ intro: 0, outro: 0 });
                export default function Caption({ caption }) {
                    return <Text style={{ width: caption.text === "broken" ? "bad" : 200 }}>
                        {caption.text}
                    </Text>;
                }
            "#;
            let scene = scene(
                "captions.timeline.json",
                &json!({
                    "canvas": { "width": 640, "height": 360, "fps": 30 },
                    "resources": { "words": "./words.motion.tsx" },
                    "tracks": { "caption": [{
                        "presenter": { "component": "words" },
                        "clips": [
                            { "text": "valid", "start": 0, "duration": 1 },
                            { "text": "broken", "start": 2, "duration": 1 }
                        ]
                    }] }
                }),
                &|p| p.ends_with("words.motion.tsx").then(|| source.to_string()),
            );
            let mut checker = scene.into_checker();
            for time in [0.0, 0.5, 1.0, 1.5] {
                checker.check(Tick::from_seconds(time).unwrap()).unwrap();
            }
            let error = checker.check(Tick::from_seconds(2.0).unwrap()).unwrap_err();
            assert!(error.contains("bad"), "{error}");
            checker.check(Tick::ZERO).unwrap();
        });
    }
}
