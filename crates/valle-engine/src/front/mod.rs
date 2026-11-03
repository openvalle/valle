//! The Motion frontend. A scene's modules are rewritten (`rewrite`) and run in a sandboxed QuickJS
//! under the tracing runtime (`runtime.js`). Tracing the scene at a time leaves a variant, lowered
//! (`lower`) into a frame template that serves every frame its guards hold for.
//!
//! The sandbox holds only what animation needs: no clock, no promises, no proxies, no weak
//! references, no code made at run time, and every built-in frozen. Its limits are counts, not
//! times, so a scene stops at the same place on every machine: interrupt polls, script memory by
//! requested size, a 12 MiB stack, and per variant the ops, nodes and guards (`runtime.js`).
//!
//! A `Front` runs scene code on the thread that calls it, which needs a stack of more than 12 MiB.

pub mod controls;
mod lower;
mod rewrite;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ptr;
use std::rc::Rc;

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::context::{EvalOptions, intrinsic};
use rquickjs::{Array, Context, Ctx, Exception, Function, Object, Runtime, Value};

use crate::eval::{self, Clock};
use crate::style::color;
use crate::template::{Error, Template, V};
use crate::time::{Rate, Tick};

pub use controls::{Inputs, Prop};
use lower::{Arg, At};
pub use rewrite::{BUILT_IN_FAMILIES, Holds, Slot};

const RUNTIME: &str = include_str!("runtime.js");
/// The built-in charts, d3's subset, which run as scene code.
const CHARTS: (&str, &str) = ("valle/charts.js", include_str!("charts.js"));

/// Interrupt polls one trace may take. QuickJS polls every 10 000 calls and backward jumps, so
/// this is about 2·10⁸ steps; the first poll of a trace may come early by what the last one left.
/// Provisional until the test scenes exist to set it at 100 times the heaviest.
const STEPS: u32 = 20_000;
/// Bytes scripts may hold.
const MEMORY: usize = 256 << 20;
const STACK: usize = 12 << 20;
/// Variants kept; the least recently used goes first.
const VARIANTS: usize = 64;
/// Extensions an import resolves through, in order.
const EXTENSIONS: [&str; 4] = [".tsx", ".ts", ".jsx", ".js"];

/// The `composition` a scene exports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Composition {
    pub width: u32,
    pub height: u32,
    pub fps: Rate,
    pub duration: Tick,
    pub looped: bool,
}

/// What lays text out for `measureText` and `textOutline`, with the scene's fonts: a text alone
/// in a style, its declarations as JSON pairs.
pub trait Typeset {
    /// Its box's width and height, and its count of lines.
    fn measure(&self, text: &str, style: &str) -> Result<[f64; 3], Error>;
    /// Its glyphs' outlines as `textOutline` gives them, as JSON: the first line's baseline at
    /// `origin`, from the left, the middle or the right of its box as `align` says.
    fn outline(
        &self,
        text: &str,
        style: &str,
        origin: [f64; 2],
        align: &str,
    ) -> Result<String, Error>;
}

/// One frame: the variant that holds at its time, and its expressions' values there.
pub struct Frame {
    pub template: Rc<Template>,
    pub values: Vec<V>,
}

struct Module {
    path: String,
    /// The source line of each line of the evaluated code.
    lines: Vec<u32>,
}

pub struct Front {
    pub composition: Composition,
    /// The props the scene declares.
    pub props: Vec<Prop>,
    /// The data it declares.
    pub data: Vec<Prop>,
    /// The resources it declares, each default a path joined to the scene's folder or a web
    /// address.
    pub resources: Vec<Slot>,
    /// What its root component takes: the props' and data's values, as JSON.
    values: String,
    entry: String,
    modules: Vec<Module>,
    /// `path:line:col` of each site the rewrite numbered.
    sites: Vec<Rc<str>>,
    steps: Rc<Cell<u32>>,
    /// Most recently used first.
    variants: VecDeque<Rc<Template>>,
    traces: usize,
    context: Context,
    /// What lays out text for `measureText` and `textOutline`, once the scene gives it.
    typeset: Rc<RefCell<Option<Box<dyn Typeset>>>>,
    /// The code of its shader resources, by name, once the scene reads them.
    shaders: RefCell<HashMap<String, Rc<str>>>,
}

impl Front {
    /// Loads the scene at `entry` and the project modules it imports, its controls bound to
    /// `inputs`; `read` gives a file's text by its `/`-separated path.
    pub fn load(
        entry: &str,
        read: &dyn Fn(&str) -> Option<String>,
        inputs: &Inputs,
    ) -> Result<Front, Error> {
        let text = read(entry).ok_or_else(|| format!("{entry}: not found"))?;
        let mut loader = Loader {
            read,
            loaded: Vec::new(),
            by_path: HashMap::new(),
            visiting: Vec::new(),
            sites: Vec::new(),
        };
        loader.visit(entry.to_string(), text)?;
        let charts = loader.builtin(CHARTS.0, CHARTS.1)?;
        let scene = &loader.loaded[loader.by_path[entry]].code;
        let folder = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
        let resources = (scene.resources.iter())
            .map(|slot| Slot {
                name: slot.name.clone(),
                kind: slot.kind,
                default: (slot.default.iter())
                    .map(|d| match d.starts_with('/') || rewrite::web(d) {
                        true => d.clone(),
                        false => rewrite::join(folder, d),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        if let Some(k) =
            (inputs.resources.keys()).find(|k| !resources.iter().any(|s| s.name == **k))
        {
            return Err(format!(
                "{entry}: resources: the scene declares no resource {k} in controls.resources"
            ));
        }
        for slot in &resources {
            let Some(files) = inputs.resources.get(&slot.name) else {
                continue;
            };
            let e = match (slot.kind, files.len()) {
                (_, 0) => Err("nothing is bound to it".to_string()),
                (Holds::Image, 2..) => Err("an image resource has one file".to_string()),
                (Holds::Shader, 2..) => Err("a shader resource has one file".to_string()),
                _ => files.iter().try_for_each(|f| rewrite::located(f)),
            };
            if let Err(e) = e {
                return Err(format!("{entry}: resource {}: {e}", slot.name));
            }
        }
        let (props, data) = (scene.props.clone(), scene.data.clone());
        let values =
            controls::values(&props, &data, inputs).map_err(|e| format!("{entry}: {e}"))?;
        let values = serde_json::Value::Object(values).to_string();
        let composition = scene.composition.ok_or_else(|| {
            format!(
                "{entry}: a scene exports its composition, such as `export const composition \
                     = {{ width: 1920, height: 1080, fps: 30, duration: 5 }}`"
            )
        })?;

        let runtime = Runtime::new_with_alloc(Counted(0)).map_err(|e| e.to_string())?;
        runtime.set_max_stack_size(STACK);
        let steps = Rc::new(Cell::new(0));
        let counter = steps.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            counter.set(counter.get() + 1);
            counter.get() > STEPS
        })));
        let context = Context::custom::<(
            intrinsic::Eval,
            intrinsic::RegExpCompiler,
            intrinsic::RegExp,
            intrinsic::Json,
            intrinsic::MapSet,
            intrinsic::TypedArrays,
        )>(&runtime)
        .map_err(|e| e.to_string())?;

        let modules = loader
            .loaded
            .iter()
            .map(|m| Module {
                path: m.path.clone(),
                lines: m.code.lines.clone(),
            })
            .collect();
        let front = Front {
            composition,
            props,
            data,
            resources,
            values,
            entry: entry.to_string(),
            modules,
            sites: loader.sites,
            steps,
            variants: VecDeque::new(),
            traces: 0,
            context,
            typeset: Rc::default(),
            shaders: RefCell::default(),
        };
        front.context.with(|ctx| -> Result<(), Error> {
            install_host(&ctx, front.typeset.clone()).map_err(|e| e.to_string())?;
            front.run(&ctx, RUNTIME.into(), "runtime.js")?;
            front.run(&ctx, format!("__builtin({})", charts.code), "charts.js")?;
            let mut free: Vec<&str> = loader
                .loaded
                .iter()
                .flat_map(|m| m.code.free.iter().map(String::as_str))
                .collect();
            free.sort_unstable();
            free.dedup();
            front.run(&ctx, format!("__declareFree({})", json(&free)), "free.js")?;
            for (i, m) in loader.loaded.iter().enumerate() {
                let code = format!("__define({}, () => {})", json(&m.keys), m.code.code);
                front.run(&ctx, code, &format!("m{i}.js"))?;
            }
            front.run(&ctx, "__lockdown()".into(), "lockdown.js")
        })?;
        Ok(front)
    }

    /// Gives shader resource `name` its code.
    pub fn set_shader(&self, name: &str, code: Rc<str>) {
        self.shaders.borrow_mut().insert(name.to_string(), code);
    }

    /// Gives `measureText` and `textOutline` what lays out text.
    pub fn set_typeset(&self, typeset: Box<dyn Typeset>) {
        *self.typeset.borrow_mut() = Some(typeset);
    }

    /// The frame at `at`: a variant whose guards hold there, traced when none does.
    pub fn frame(&mut self, at: Tick) -> Result<Frame, Error> {
        let seconds = at.seconds();
        let clock = Clock {
            seconds,
            progress: (seconds / self.composition.duration.seconds()).clamp(0.0, 1.0),
        };
        for i in 0..self.variants.len() {
            // A variant that fails to evaluate here does not hold here either; a fresh trace
            // reports the error if it is real.
            let Ok(values) = eval::run(&self.variants[i].program, clock) else {
                continue;
            };
            if self.variants[i].holds(&values) {
                let template = self.variants.remove(i).expect("an index in range");
                self.variants.push_front(template.clone());
                return Ok(Frame { template, values });
            }
        }
        let template = Rc::new(self.trace(clock)?);
        let values = eval::run(&template.program, clock)?;
        if !template.holds(&values) {
            return Err(format!(
                "engine bug: the variant traced at {seconds} s fails there"
            ));
        }
        self.variants.push_front(template.clone());
        self.variants.truncate(VARIANTS);
        Ok(Frame { template, values })
    }

    /// How many times the scene was traced.
    pub fn traces(&self) -> usize {
        self.traces
    }

    fn trace(&mut self, clock: Clock) -> Result<Template, Error> {
        self.traces += 1;
        self.steps.set(0);
        let traced = self.context.with(|ctx| -> Result<String, Error> {
            let c = &self.composition;
            let composition = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            let set = |k: &str, v: f64| composition.set(k, v).map_err(|e| e.to_string());
            set("width", c.width.into())?;
            set("height", c.height.into())?;
            set("fps", c.fps.as_f64())?;
            set("duration", c.duration.seconds())?;
            // Exactly, for the times Echo and Shutter sample; useComposition() does not give them.
            let exact = |k: &str, v: String| composition.set(k, v).map_err(|e| e.to_string());
            exact("rate", c.fps.to_string())?;
            exact("ticks", c.duration.seconds_text())?;
            composition
                .set("looped", c.looped)
                .map_err(|e| e.to_string())?;
            let trace: Function = ctx.globals().get("__trace").map_err(|e| e.to_string())?;
            trace
                .call((
                    self.entry.as_str(),
                    clock.seconds,
                    clock.progress,
                    composition,
                    self.values.as_str(),
                ))
                .map_err(|e| self.describe(&ctx, e))
        })?;
        lower::variant(
            &traced,
            &|at| self.place(at),
            &self.resources,
            &self.shaders.borrow(),
        )
    }

    fn run(&self, ctx: &Ctx<'_>, source: String, file: &str) -> Result<(), Error> {
        self.steps.set(0);
        // Strict by default.
        let mut options = EvalOptions::default();
        options.filename = Some(file.to_string());
        ctx.eval_with_options::<(), _>(source, options)
            .map_err(|e| self.describe(ctx, e))
    }

    fn place(&self, at: &At) -> Rc<str> {
        match at {
            At::Site(s) => self
                .sites
                .get(*s as usize)
                .cloned()
                .unwrap_or_else(|| "?".into()),
            At::Stack(s) => s
                .split_once(':')
                .and_then(|(file, line)| self.line(file, line.parse().ok()?))
                .unwrap_or_else(|| s.as_str().into()),
        }
    }

    /// `path:line` of a line of an evaluated module file, `m3.js`.
    fn line(&self, file: &str, line: usize) -> Option<Rc<str>> {
        let k: usize = file.strip_prefix('m')?.strip_suffix(".js")?.parse().ok()?;
        let m = self.modules.get(k)?;
        Some(match m.lines.get(line.wrapping_sub(1)) {
            Some(&l) if l > 0 => format!("{}:{l}", m.path).into(),
            _ => m.path.as_str().into(),
        })
    }

    /// A thrown error as `path:line: Name: message`, with the scene's frames of the stack.
    fn describe(&self, ctx: &Ctx<'_>, error: rquickjs::Error) -> Error {
        if !error.is_exception() {
            return error.to_string();
        }
        let value: Value = ctx.catch();
        let Some(object) = value.as_object() else {
            return match value.as_string() {
                Some(s) => s.to_string().unwrap_or_default(),
                None => format!("a thrown {}", value.type_of()),
            };
        };
        let text = |k: &str| object.get::<_, Option<String>>(k).ok().flatten();
        let name = text("name").unwrap_or_default();
        let mut message = text("message").unwrap_or_default();
        if message == "interrupted" && self.steps.get() > STEPS {
            message = "the script ran past its step limit, likely an endless loop".into();
        }
        let message = if name.is_empty() {
            message
        } else {
            format!("{name}: {message}")
        };
        let stack = text("stack").unwrap_or_default();
        let frames: Vec<(String, Rc<str>)> =
            stack.lines().filter_map(|l| self.frame_of(l)).collect();
        let site = object.get::<_, Option<u32>>("site").ok().flatten();
        let first = match site {
            Some(s) => Some(self.place(&At::Site(s))),
            None => frames.first().map(|f| f.1.clone()),
        };
        let mut out = match first {
            Some(place) => format!("{place}: {message}"),
            None => message,
        };
        for (function, place) in &frames {
            out.push_str(&format!("\n    at {function} ({place})"));
        }
        out
    }

    /// A stack line of a scene module, `at name (m3.js:12:5)`, as (name, `path:line`).
    fn frame_of(&self, line: &str) -> Option<(String, Rc<str>)> {
        let line = line.trim().strip_prefix("at ")?;
        let (function, place) = match line.split_once(" (") {
            Some((function, place)) => (function, place.strip_suffix(')')?),
            None => ("<module>", line),
        };
        let mut parts = place.split(':');
        let (file, number) = (parts.next()?, parts.next()?.parse().ok()?);
        Some((function.to_string(), self.line(file, number)?))
    }
}

struct Loaded {
    path: String,
    /// The import keys that resolve to the module, its path first.
    keys: Vec<String>,
    code: rewrite::Rewritten,
}

struct Loader<'r> {
    read: &'r dyn Fn(&str) -> Option<String>,
    loaded: Vec<Loaded>,
    by_path: HashMap<String, usize>,
    visiting: Vec<String>,
    sites: Vec<Rc<str>>,
}

impl Loader<'_> {
    /// Rewrites the module at `path` and, depth first, the modules it imports.
    fn visit(&mut self, path: String, text: String) -> Result<usize, Error> {
        self.visiting.push(path.clone());
        let code = rewrite::rewrite(&path, &text, self.sites.len()).map_err(|e| {
            let sep = if e.starts_with(|c: char| c.is_ascii_digit()) {
                ":"
            } else {
                ": "
            };
            format!("{path}{sep}{e}")
        })?;
        for s in &code.sites {
            self.sites.push(match s.line {
                0 => path.as_str().into(),
                line => format!("{path}:{line}:{}", s.col).into(),
            });
        }
        let mut imports = Vec::new();
        for key in &code.imports {
            let (target, text) = self.resolve(key).ok_or_else(|| {
                format!(
                    "{path}: cannot import {key:?}: no such file (tried {key}.tsx, .ts, .jsx, .js)"
                )
            })?;
            if let Some(i) = self.visiting.iter().position(|p| *p == target) {
                let mut cycle = self.visiting[i..].to_vec();
                cycle.push(target);
                return Err(format!(
                    "import cycle: {}; modules cannot import each other in a circle",
                    cycle.join(" → ")
                ));
            }
            let index = match self.by_path.get(&target) {
                Some(&i) => i,
                None => self.visit(target, text)?,
            };
            imports.push((key.clone(), index));
        }
        self.visiting.pop();
        self.loaded.push(Loaded {
            path: path.clone(),
            keys: vec![path.clone()],
            code,
        });
        let index = self.loaded.len() - 1;
        self.by_path.insert(path, index);
        for (key, i) in imports {
            let keys = &mut self.loaded[i].keys;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(index)
    }

    /// Rewrites a built-in module, which the runtime makes globals of.
    fn builtin(&mut self, path: &str, text: &str) -> Result<rewrite::Rewritten, Error> {
        let code = rewrite::rewrite(path, text, self.sites.len())
            .map_err(|e| format!("engine bug: {path}: {e}"))?;
        for s in &code.sites {
            self.sites
                .push(format!("{path}:{}:{}", s.line, s.col).into());
        }
        Ok(code)
    }

    /// The file an import key names: the key itself when it has an extension, else the first of
    /// the key with each extension.
    fn resolve(&self, key: &str) -> Option<(String, String)> {
        let exact = EXTENSIONS
            .iter()
            .any(|e| key.ends_with(e))
            .then(|| key.to_string());
        exact
            .into_iter()
            .chain(EXTENSIONS.iter().map(|e| format!("{key}{e}")))
            .find_map(|path| (self.read)(&path).map(|text| (path, text)))
    }
}

fn json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("names serialize")
}

const NO_FONTS: &str = "it lays text out with a scene's fonts, which this has not";

/// The units `<Text split>` splits `text` into, by `split`: each unit's start and end one after
/// another, in UTF-16 code units as JavaScript indexes strings. `char` gives grapheme clusters, but white space;
/// `word` runs of characters between white space; `line` the lines between line feeds, but empty
/// ones.
fn units(text: &str, split: &str) -> Vec<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    // Each char boundary's UTF-16 index, by its byte index.
    let mut index = vec![0; text.len() + 1];
    for (i, c) in text.char_indices() {
        index[i + c.len_utf8()] = index[i] + c.len_utf16();
    }
    let utf16 = |byte: usize| index[byte];
    let pieces: Vec<(usize, &str)> = match split {
        "char" => text.grapheme_indices(true).collect(),
        "word" => {
            let mut out = Vec::new();
            let mut start = None;
            for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
                match (c.is_whitespace(), start) {
                    (false, None) => start = Some(i),
                    (true, Some(s)) => {
                        out.push((s, &text[s..i]));
                        start = None;
                    }
                    _ => {}
                }
            }
            out
        }
        _ => {
            let mut at = 0;
            (text.split('\n'))
                .map(|line| {
                    let piece = (at, line);
                    at += line.len() + 1;
                    piece
                })
                .collect()
        }
    };
    (pieces.into_iter())
        .filter(|(_, p)| !p.trim().is_empty())
        .flat_map(|(i, p)| [utf16(i), utf16(i + p.len())])
        .collect()
}

/// `__host.op(name, args)`: one op's value over concrete operands, through the evaluator frames
/// use, so tracing sees the values frames compute; `__host.svg`, the SVG tables as JSON;
/// `__host.random(seed, count, low, high)`, checked by the runtime; `__host.units(text, split)`,
/// the units of a split text; and `__host.measure(text, style)` and `__host.outline(text, style,
/// x, y, align)`, what `measureText` and `textOutline` give.
fn install_host<'js>(
    ctx: &Ctx<'js>,
    typeset: Rc<RefCell<Option<Box<dyn Typeset>>>>,
) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    let set = typeset.clone();
    host.set(
        "measure",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String, style: String| {
                let measured = match &*set.borrow() {
                    Some(t) => t.measure(&text, &style).map(Vec::from),
                    None => Err(NO_FONTS.into()),
                };
                measured.map_err(|e| Exception::throw_type(&ctx, &format!("measureText: {e}")))
            },
        )?,
    )?;
    host.set(
        "outline",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String, style: String, x: f64, y: f64, align: String| {
                let outlined = match &*typeset.borrow() {
                    Some(t) => t.outline(&text, &style, [x, y], &align),
                    None => Err(NO_FONTS.into()),
                };
                outlined.map_err(|e| Exception::throw_type(&ctx, &format!("textOutline: {e}")))
            },
        )?,
    )?;
    host.set(
        "op",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, name: String, args: Array<'js>| {
                op(&ctx, &name, &args).map_err(|e| Exception::throw_type(&ctx, &e))
            },
        )?,
    )?;
    host.set("svg", crate::svg::tables())?;
    host.set(
        "units",
        Function::new(ctx.clone(), |text: String, split: String| {
            units(&text, &split)
        })?,
    )?;
    // A color as d3 reads one: sRGB written as CSS had it before color spaces, as 0–255 channels
    // and an opacity; one with no opacity has no channels either, NaN.
    host.set(
        "color",
        Function::new(ctx.clone(), |text: String| {
            let text = text.trim().to_ascii_lowercase();
            let c = color::legacy(&text)
                .then(|| color::wide(&text).ok())
                .flatten()?;
            let channel = |x: f64| {
                if c[3] <= 0.0 { f64::NAN } else { x * 255.0 }
            };
            Some(vec![channel(c[0]), channel(c[1]), channel(c[2]), c[3]])
        })?,
    )?;
    host.set(
        "random",
        Function::new(ctx.clone(), |seed: f64, count: f64, low: f64, high: f64| {
            crate::noise::random(seed as u64, count as usize, [low, high])
        })?,
    )?;
    ctx.globals().set("__host", host)
}

fn op<'js>(ctx: &Ctx<'js>, name: &str, args: &Array<'js>) -> Result<Value<'js>, Error> {
    let mut values = Vec::new();
    let mut operands = Vec::new();
    for a in args.iter::<Value>() {
        let a = a.map_err(|e| e.to_string())?;
        let v = if let Some(x) = a.as_number() {
            V::Num(x)
        } else if let Some(b) = a.as_bool() {
            V::Bool(b)
        } else if let Some(s) = a.as_string() {
            V::Str(s.to_string().map_err(|e| e.to_string())?.into())
        } else if let Some(o) = a.as_object() {
            let text: String = o.get("text").map_err(|_| format!("op {name}: an object"))?;
            operands.push(Arg::Text(text.into()));
            continue;
        } else {
            return Err(format!("op {name}: cannot take {}", a.type_of()));
        };
        operands.push(Arg::Expr(values.len() as u32));
        values.push(v);
    }
    let op = lower::make(name, &operands)?;
    let clock = Clock {
        seconds: 0.0,
        progress: 0.0,
    };
    let value = match eval::value(&op, &values, clock)? {
        V::Num(x) => Value::new_number(ctx.clone(), x),
        V::Bool(b) => Value::new_bool(ctx.clone(), b),
        V::Str(s) => rquickjs::String::from_str(ctx.clone(), &s)
            .map_err(|e| e.to_string())?
            .into_value(),
    };
    Ok(value)
}

/// QuickJS's allocator: Rust's, counting the bytes scripts hold and failing past `MEMORY`. Sizes
/// are counted as requested (rounded up to 8 bytes), so platforms of one pointer width fail at the
/// same allocation; QuickJS's own limit counts the platform allocator's blocks.
struct Counted(usize);

impl Counted {
    fn fits(&self, freed: usize, size: usize) -> bool {
        size.checked_next_multiple_of(8)
            .is_some_and(|size| self.0 - freed + size <= MEMORY)
    }

    fn count(&mut self, freed: usize, p: *mut u8) -> *mut u8 {
        if !p.is_null() {
            // SAFETY: `p` came from `RustAllocator`.
            self.0 = self.0 - freed + unsafe { RustAllocator::usable_size(p) };
        }
        p
    }
}

// SAFETY: every block comes from `RustAllocator`, which upholds the contract.
unsafe impl Allocator for Counted {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if !self.fits(0, size) {
            return ptr::null_mut();
        }
        let p = RustAllocator.alloc(size);
        self.count(0, p)
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        match count.checked_mul(size) {
            Some(total) if self.fits(0, total) => {
                let p = RustAllocator.calloc(count, size);
                self.count(0, p)
            }
            _ => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        // SAFETY: the caller passes a block of this allocator.
        unsafe {
            self.0 -= RustAllocator::usable_size(ptr);
            RustAllocator.dealloc(ptr);
        }
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        // SAFETY: the caller passes a block of this allocator.
        let old = unsafe { RustAllocator::usable_size(ptr) };
        if !self.fits(old, new_size) {
            return ptr::null_mut();
        }
        // SAFETY: as above; on failure the old block stays, as C's realloc leaves it.
        let p = unsafe { RustAllocator.realloc(ptr, new_size) };
        self.count(old, p)
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        // SAFETY: the caller passes a block of this allocator.
        unsafe { RustAllocator::usable_size(ptr) }
    }
}
