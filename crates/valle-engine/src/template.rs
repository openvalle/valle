//! The frame template: what tracing one variant of a scene leaves for its frames. A tree of nodes
//! whose props are constants or expressions, one expression program in dependency order, and the
//! guards under which the variant holds. It lives in memory only; `{:?}` is a debugging aid, not a
//! format.

use std::borrow::Cow;
use std::rc::Rc;

use crate::ease::Easing;
use crate::format::Format;
use crate::front::Role;
use crate::noise::Wiggle;
use crate::particles::{Field, System};
use crate::path::edit::Join;
use crate::spring::Spring;
use crate::style::color::Mixing;
use crate::time::{Rate, Speed, Tick};

use crate::js::{self, MathFn};

pub type Error = String;

/// A value of the expression language: what a JavaScript number, boolean or string traces into.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Num(f64),
    Bool(bool),
    Str(Rc<str>),
}

impl V {
    /// JavaScript `String(v)`.
    pub fn text(&self) -> Cow<'_, str> {
        match self {
            V::Num(x) => Cow::Owned(js::number_to_string(*x)),
            V::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            V::Str(s) => Cow::Borrowed(s),
        }
    }

    /// JavaScript `Number(v)` for numbers and booleans; NaN otherwise.
    pub fn num(&self) -> f64 {
        match self {
            V::Num(x) => *x,
            V::Bool(b) => f64::from(u8::from(*b)),
            V::Str(_) => f64::NAN,
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            V::Num(x) => *x != 0.0 && !x.is_nan(),
            V::Bool(b) => *b,
            V::Str(s) => !s.is_empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// Literal text or an operand of a template.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(Rc<str>),
    Expr(u32),
}

/// One expression. Operands are indices of earlier expressions in the same program.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Const(V),
    /// The local clock in seconds.
    Seconds,
    /// `seconds / duration`, within [0, 1].
    Progress,
    /// Numbers only; `+` on text traces as a template.
    Bin(Bin, u32, u32),
    /// Numbers only.
    Cmp(Cmp, u32, u32),
    Neg(u32),
    /// The second operand where the first is truthy, else the third.
    Select(u32, u32, u32),
    Math(MathFn, Vec<u32>),
    /// The parts joined as a template literal joins them.
    Template(Vec<Part>),
    Interp(Rc<Interp>),
    /// A stage's progress at the input time.
    Stage(u32, Stage),
    /// The input time held still from the first time to the second, and later less that span.
    Freeze(u32, [f64; 2]),
    /// A `TimeScope`'s clock from its parent's, in ticks: `(t − offset) × rate`, rounded down.
    Scope(u32, Tick, Speed),
    /// The root clock at another output time, for `Echo` and `Shutter`.
    Sample(u32, Sample),
    /// A Timeline clip's measure at its parent's time.
    Clip(u32, Rc<Clip>, ClipPart),
    /// A field of a simulation at the input time.
    Table(u32, Rc<Table>),
    Wiggle(u32, Wiggle),
    /// A field of one particle of a `particles()` system at the input time.
    Particle(u32, Rc<System>, u32, Field),
    /// A number as text.
    Format(u32, Format),
    /// A path function over its operands: path data and numbers.
    Path(PathFn, Vec<u32>),
    /// `rangeSelector`'s weight of a unit in [0, 1]: over its position, start, end, offset and
    /// softness.
    Range(Shape, Vec<u32>),
    /// `noise1d(seed, x)` with one operand, `noise2d(seed, x, y)` with two.
    Noise(u64, Vec<u32>),
    /// `spring(t)`, or its velocity.
    Spring {
        input: u32,
        spring: Spring,
        velocity: bool,
    },
    /// `bounds(path)`: a measure of the border box of the element with this key path, in canvas
    /// px, as laid out. It is known only after layout, as is every value made from it (`late`).
    Bounds(Rc<str>, Measure),
}

/// A clip of a Timeline, as the scene it lowers to reads it from the clip table: where its time
/// starts and ends on its parent's clock, when it shows, `[on[0], on[1])`, and the time inside
/// it. A transition's progress steps over its frames at `frames`.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub start: Tick,
    pub end: Tick,
    pub on: [Tick; 2],
    pub time: ClipTime,
    pub frames: Option<Rate>,
}

/// The time a clip gives what it holds.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipTime {
    /// Its source's: `trim + t × rate`, or a time map's points (clip time, source time) straight
    /// between them and held past the ends; then past the source's length held on its last tick
    /// (`looped` false) or looped.
    Source {
        trim: Tick,
        rate: Speed,
        map: Vec<(Tick, Tick)>,
        length: Option<(Tick, bool)>,
    },
    /// A scene's in a role, for a scene `duration` long (`Role::time`).
    Role { role: Role, duration: Tick },
}

/// The clip table: a Timeline's clips by id.
pub type Clips = std::collections::HashMap<String, Rc<Clip>>;

/// What an `Op::Clip` gives: the time inside the clip, in seconds; the time since its start;
/// how far through it is; or whether it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipPart {
    Time,
    Local,
    Progress,
    On,
}

impl Clip {
    /// Its `part` at its parent's time `t`.
    pub fn at(&self, part: ClipPart, t: Tick) -> Result<V, Error> {
        let local = t - self.start;
        let length = self.end - self.start;
        Ok(match part {
            ClipPart::On => V::Bool(self.on[0] <= t && t < self.on[1]),
            ClipPart::Local => V::Num(local.seconds()),
            // Frame i of N at i / (N − 1), a lone frame at 0.5.
            ClipPart::Progress => V::Num(match self.frames {
                Some(fps) => {
                    let first = fps.frame_at(Tick(self.start.0 - 1)) + 1;
                    let n = fps.frame_at(Tick(self.end.0 - 1)) - first + 1;
                    match n {
                        ..=1 => 0.5,
                        _ => ((fps.frame_at(t) - first) as f64 / (n - 1) as f64).clamp(0.0, 1.0),
                    }
                }
                None if length.0 > 0 => (local.seconds() / length.seconds()).clamp(0.0, 1.0),
                None => 0.0,
            }),
            ClipPart::Time => V::Num(match &self.time {
                ClipTime::Source {
                    trim,
                    rate,
                    map,
                    length,
                } => {
                    let source = match map.as_slice() {
                        [] => *trim + rate.apply(local)?,
                        points => mapped(points, local),
                    };
                    match length {
                        Some((length, _)) if length.0 <= 0 => Tick::ZERO,
                        Some((length, false)) => source.clamp(Tick::ZERO, Tick(length.0 - 1)),
                        Some((length, true)) => Tick(source.0.rem_euclid(length.0)),
                        None => source,
                    }
                    .seconds()
                }
                ClipTime::Role { role, duration } => role
                    .time(*duration, length, local.clamp(Tick::ZERO, length))?
                    .seconds(),
            }),
        })
    }
}

/// A time map's source time at clip time `t`.
fn mapped(points: &[(Tick, Tick)], t: Tick) -> Tick {
    let (first, last) = (points[0], points[points.len() - 1]);
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let k = points.partition_point(|p| p.0 <= t);
    let ((x0, y0), (x1, y1)) = (points[k - 1], points[k]);
    let into = i128::from((t - x0).0) * i128::from((y1 - y0).0);
    Tick(y0.0 + into.div_euclid(i128::from((x1 - x0).0)) as i64)
}

/// An output time other than the frame's: moved by `offset`, then held inside the composition's
/// `duration`, or wrapped round it when it loops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub offset: Tick,
    pub duration: Tick,
    pub looped: bool,
}

impl Sample {
    /// The time `self` samples for the frame at `t`.
    pub fn at(&self, t: Tick) -> Tick {
        let t = t.0.saturating_add(self.offset.0);
        let end = self.duration.0;
        Tick(match self.looped {
            true => t.rem_euclid(end),
            false => t.clamp(0, end),
        })
    }
}

/// How a `rangeSelector` weighs the units in its range: all or none (`Square`), or with its edges
/// feathered over its softness, straight (`Ramp`) or eased (`Smooth`), or rising to its middle and
/// falling from it (`Triangle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Ramp,
    Smooth,
    Triangle,
}

/// What `bounds()` gives of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    X,
    Y,
    Width,
    Height,
}

impl Op {
    /// Calls `f` with each operand.
    pub fn operands(&self, mut f: impl FnMut(u32)) {
        match self {
            Op::Const(_) | Op::Seconds | Op::Progress | Op::Bounds(..) => {}
            Op::Bin(_, a, b) | Op::Cmp(_, a, b) => {
                f(*a);
                f(*b);
            }
            Op::Neg(a)
            | Op::Stage(a, _)
            | Op::Freeze(a, _)
            | Op::Scope(a, ..)
            | Op::Sample(a, _)
            | Op::Clip(a, ..)
            | Op::Table(a, _)
            | Op::Wiggle(a, _)
            | Op::Particle(a, ..)
            | Op::Format(a, _)
            | Op::Spring { input: a, .. } => f(*a),
            Op::Select(a, b, c) => [a, b, c].into_iter().for_each(|x| f(*x)),
            Op::Math(_, args) | Op::Path(_, args) | Op::Noise(_, args) | Op::Range(_, args) => {
                args.iter().for_each(|x| f(*x));
            }
            Op::Template(parts) => {
                for p in parts {
                    if let Part::Expr(i) = p {
                        f(*i);
                    }
                }
            }
            Op::Interp(i) => {
                f(i.input);
                for &(a, b) in &i.stops {
                    f(a);
                    f(b);
                }
            }
        }
    }
}

/// A stage of a sequence and how its progress goes: from 0 to 1 once, `count` times, or there and
/// back `count` times.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub start: f64,
    pub duration: f64,
    pub count: f64,
    pub yoyo: bool,
}

impl Stage {
    /// From `start|duration|count|once`, `…|repeat` or `…|yoyo`.
    pub fn parse(spec: &str) -> Result<Stage, String> {
        let bad = || format!("stage {spec}");
        let [start, duration, count, how] = spec.split('|').collect::<Vec<_>>()[..] else {
            return Err(bad());
        };
        let number = |s: &str| {
            s.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(bad)
        };
        let (start, duration, count) = (number(start)?, number(duration)?, number(count)?);
        if duration <= 0.0 || count <= 0.0 || !["once", "repeat", "yoyo"].contains(&how) {
            return Err(bad());
        }
        let count = if how == "once" { 1.0 } else { count };
        let yoyo = how == "yoyo";
        Ok(Stage {
            start,
            duration,
            count,
            yoyo,
        })
    }

    /// The progress at `t` seconds: within [0, 1], and back at 0 when a yoyo ends; a repeat ends
    /// at 1, not at the start of another turn.
    pub fn at(&self, t: f64) -> f64 {
        let p = ((t - self.start) / self.duration).clamp(0.0, 1.0);
        if self.yoyo {
            let leg = (p * self.count * 2.0) % 2.0;
            return if leg > 1.0 { 2.0 - leg } else { leg };
        }
        if p >= 1.0 {
            1.0
        } else {
            (p * self.count).fract()
        }
    }
}

/// A field of `simulate()`: its value every `dt` seconds from 0, read linearly between them up to
/// `duration`.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub dt: f64,
    pub duration: f64,
    pub values: Vec<f64>,
}

impl Table {
    /// From `dt|duration|values`, the values space separated.
    pub fn parse(spec: &str) -> Result<Table, String> {
        let bad = || "a simulation table: dt|duration|values".to_string();
        let [dt, duration, values] = spec.splitn(3, '|').collect::<Vec<_>>()[..] else {
            return Err(bad());
        };
        let number = |s: &str| {
            s.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(bad)
        };
        let (dt, duration) = (number(dt)?, number(duration)?);
        let values: Vec<f64> = values.split(' ').map(number).collect::<Result<_, _>>()?;
        // The steps cover the duration, as `simulate()` takes them.
        if dt <= 0.0
            || duration <= 0.0
            || values.len() < 2
            || (values.len() - 1) as f64 * dt < duration
        {
            return Err(bad());
        }
        Ok(Table {
            dt,
            duration,
            values,
        })
    }

    pub fn at(&self, t: f64) -> f64 {
        if t.is_nan() {
            return t;
        }
        let at = t.clamp(0.0, self.duration) / self.dt;
        let i = (at.floor() as usize).min(self.values.len() - 2);
        let (a, b) = (self.values[i], self.values[i + 1]);
        a + (b - a) * (at - i as f64)
    }
}

/// What a path function gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathFn {
    /// `pathLength(path)`.
    Length,
    /// `pointAt(path, progress)`, x or y.
    Point(Axis),
    /// `tangentAt(path, progress)`: the unit direction's x or y, or its angle in degrees.
    Tangent(Axis),
    Angle,
    /// `roundCorners(path, radius)`.
    RoundCorners,
    /// `puckerBloat(path, amount)`.
    PuckerBloat,
    /// `twist(path, angle)`.
    Twist,
    /// `zigzag(path, { size, ridges })`, the two as operands.
    Zigzag,
    /// `inflatePath(path, distance, { join, miterLimit })`.
    Inflate(Join),
    /// `pathBoolean(a, b, op)`, both paths operands.
    Boolean(Boolean),
    /// `morph(from, to, progress, { samples })`.
    Morph(Option<u32>),
}

/// `pathBoolean`'s ops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boolean {
    Union,
    Intersect,
    Subtract,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl PathFn {
    pub fn parse(name: &str) -> Option<PathFn> {
        if let Some(samples) = name.strip_prefix("morph|") {
            return Some(PathFn::Morph(match samples {
                "" => None,
                n => Some(n.parse().ok().filter(|n| (2..=4096).contains(n))?),
            }));
        }
        if let Some(join) = name.strip_prefix("inflate|") {
            return Some(PathFn::Inflate(match join.split_once('|')? {
                ("miter", limit) => Join::Miter(limit.parse().ok().filter(|l: &f64| *l >= 1.0)?),
                ("round", "") => Join::Round,
                ("bevel", "") => Join::Bevel,
                _ => return None,
            }));
        }
        Some(match name {
            "length" => PathFn::Length,
            "x" => PathFn::Point(Axis::X),
            "y" => PathFn::Point(Axis::Y),
            "dx" => PathFn::Tangent(Axis::X),
            "dy" => PathFn::Tangent(Axis::Y),
            "angle" => PathFn::Angle,
            "roundCorners" => PathFn::RoundCorners,
            "puckerBloat" => PathFn::PuckerBloat,
            "twist" => PathFn::Twist,
            "zigzag" => PathFn::Zigzag,
            "union" => PathFn::Boolean(Boolean::Union),
            "intersect" => PathFn::Boolean(Boolean::Intersect),
            "subtract" => PathFn::Boolean(Boolean::Subtract),
            "xor" => PathFn::Boolean(Boolean::Xor),
            _ => return None,
        })
    }

    /// How many operands it takes: the path, then numbers.
    pub fn operands(self) -> usize {
        match self {
            PathFn::Length => 1,
            PathFn::Zigzag | PathFn::Morph(_) => 3,
            _ => 2,
        }
    }
}

/// `interpolate(input, inputs, outputs, options)` over numbers. Stops are operands, so traced
/// stops work; the inputs increase.
#[derive(Clone, Debug, PartialEq)]
pub struct Interp {
    pub input: u32,
    /// (input, output) operand pairs.
    pub stops: Vec<(u32, u32)>,
    /// Past the first stop and past the last: extend the end segment (true) or clamp (false).
    pub extend: [bool; 2],
    /// Between each two stops.
    pub easing: Easing,
    /// How colors mix, if not as a transition mixes them.
    pub mixing: Option<Mixing>,
}

/// Expressions in dependency order: every operand has a lower index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub ops: Vec<Op>,
}

impl Program {
    /// Which expressions are late: known only after layout, as they read a box.
    pub fn late(&self) -> Vec<bool> {
        let mut late = vec![false; self.ops.len()];
        for (i, op) in self.ops.iter().enumerate() {
            let mut l = matches!(op, Op::Bounds(..));
            op.operands(|j| l |= late[j as usize]);
            late[i] = l;
        }
        late
    }

    /// Which expressions the `roots` read, they among them.
    pub fn reading(&self, roots: impl IntoIterator<Item = u32>) -> Vec<bool> {
        let mut read = vec![false; self.ops.len()];
        for r in roots {
            read[r as usize] = true;
        }
        // Operands come first.
        for i in (0..self.ops.len()).rev() {
            if read[i] {
                self.ops[i].operands(|j| read[j as usize] = true);
            }
        }
        read
    }
}

/// A node's prop: known at tracing, or an expression evaluated per frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Const(V),
    Expr(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    View,
    /// A paragraph: a box of the text and the Spans in it.
    Text {
        content: Vec<Piece>,
    },
    /// Part of a Text styled on its own, not a box: its text and the Spans in it. A `unit` is
    /// one of the pieces a `<Text split>` is split into, which also takes an opacity, a
    /// transform and filters, applied to its glyphs as an inline-block's are to its box.
    Span {
        content: Vec<Piece>,
        unit: bool,
    },
    /// A box whose source masks all it paints: the root node `source` (not a child), which is
    /// laid out in its border box as the canvas is, shown by its alpha or its luminance, and
    /// inverted or not.
    Mask {
        source: u32,
        luminance: bool,
        invert: bool,
    },
    /// A box that shows an image: the scene's resource of this index, as the frontend lists
    /// them. A `<Video>`'s shows the frame of its video at `video`, seconds into it.
    Image {
        resource: u32,
        video: Option<Prop>,
    },
    /// A box a shader authors write paints (`shader.rs`): its GLSL code, the values its uniforms
    /// take by name, a number, a boolean or a color string each, or a vector's numbers, the
    /// images it reads by name, the scene's resources of these indices, and the px it paints out
    /// past its border box on each side. What the box and the boxes in it paint is the image the
    /// shader reads as `content`, and what is painted under it its `backdrop`. A `<Transition>`'s
    /// reads its two children instead, as `transition.rs` has it, and is a built-in `kind` or
    /// code of its own.
    Shader {
        code: Rc<str>,
        uniforms: Vec<(Rc<str>, Vec<Prop>)>,
        inputs: Vec<(Rc<str>, u32)>,
        outset: f64,
        transition: bool,
        kind: Option<Rc<str>>,
    },
    /// An inline SVG element: an `<svg>` is a box, and the elements in it, its descendants, are
    /// not. Its attributes are as written, camelCase.
    Svg {
        tag: Tag,
        attributes: Vec<(Rc<str>, Prop)>,
    },
}

/// The SVG elements Valle draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Svg,
    G,
    Path,
    Circle,
    Ellipse,
    Rect,
    Line,
    Polyline,
    Polygon,
    Defs,
    LinearGradient,
    RadialGradient,
    Stop,
    ClipPath,
    Mask,
    Use,
    Symbol,
    Marker,
    Image,
}

const TAGS: [(&str, Tag); 19] = [
    ("svg", Tag::Svg),
    ("g", Tag::G),
    ("path", Tag::Path),
    ("circle", Tag::Circle),
    ("ellipse", Tag::Ellipse),
    ("rect", Tag::Rect),
    ("line", Tag::Line),
    ("polyline", Tag::Polyline),
    ("polygon", Tag::Polygon),
    ("defs", Tag::Defs),
    ("linearGradient", Tag::LinearGradient),
    ("radialGradient", Tag::RadialGradient),
    ("stop", Tag::Stop),
    ("clipPath", Tag::ClipPath),
    ("mask", Tag::Mask),
    ("use", Tag::Use),
    ("symbol", Tag::Symbol),
    ("marker", Tag::Marker),
    ("image", Tag::Image),
];

impl Tag {
    pub const ALL: [Tag; TAGS.len()] = {
        let mut all = [Tag::Svg; TAGS.len()];
        let mut k = 0;
        while k < TAGS.len() {
            all[k] = TAGS[k].1;
            k += 1;
        }
        all
    };

    pub fn by_name(name: &str) -> Option<Tag> {
        TAGS.iter().find(|(n, _)| *n == name).map(|&(_, t)| t)
    }

    pub fn name(self) -> &'static str {
        TAGS.iter().find(|(_, t)| *t == self).map_or("", |(n, _)| n)
    }
}

/// What a Text or a Span holds, in order: text, or a Span, which is also one of its children.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(Prop),
    Span(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// The path of keys down to the node (`card/title`), when it has a key.
    pub path: Option<Rc<str>>,
    pub classes: Vec<Rc<str>>,
    /// Style properties as written, camelCase.
    pub style: Vec<(Rc<str>, Prop)>,
    pub children: Vec<u32>,
    /// Where its JSX is written, `path:line:col`.
    pub site: Option<Rc<str>>,
}

impl Node {
    /// Where errors about the node point: its site, else its tag.
    pub fn place(&self) -> Rc<str> {
        match (&self.site, &self.kind) {
            (Some(site), _) => site.clone(),
            (None, Kind::View) => "<View>".into(),
            (None, Kind::Text { .. }) => "<Text>".into(),
            (None, Kind::Span { .. }) => "<Span>".into(),
            (None, Kind::Mask { .. }) => "<Mask>".into(),
            (None, Kind::Image { video: Some(_), .. }) => "<Video>".into(),
            (None, Kind::Image { .. }) => "<Image>".into(),
            (
                None,
                Kind::Shader {
                    transition: true, ..
                },
            ) => "<Transition>".into(),
            (None, Kind::Shader { .. }) => "<ShaderLayer>".into(),
            (None, Kind::Svg { tag, .. }) => format!("<{}>", tag.name()).into(),
        }
    }
}

/// A structural decision tracing took on a traced value. The variant holds while every guard does.
#[derive(Clone, Debug, PartialEq)]
pub struct Guard {
    pub expr: u32,
    pub want: Want,
    /// The guard only applies while each of these expressions has the given truthiness: it was
    /// taken inside that side of a conditional.
    pub when: Vec<(u32, bool)>,
    /// What decided (`if`, `loop`, `select`, `index`, `escape`, …) and where, `path:line`.
    pub kind: Rc<str>,
    pub at: Rc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    Truthy(bool),
    Eq(V),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Template {
    pub program: Program,
    /// In preorder: a parent comes before its children, and a mask before its source.
    pub nodes: Vec<Node>,
    /// What the scene's root component returned, in order; the canvas holds them.
    pub roots: Vec<u32>,
    pub guards: Vec<Guard>,
    /// The boxes drawn as pictures rather than where they are, each with the picture it draws:
    /// the canvas as the box alone paints it.
    pub pictures: Vec<(u32, u32)>,
}

impl Template {
    /// Whether the variant holds for these expression values (the program's, at one frame).
    pub fn holds(&self, values: &[V]) -> bool {
        self.failing(values).is_none()
    }

    /// The first of its guards that does not hold with `values`.
    pub fn failing(&self, values: &[V]) -> Option<&Guard> {
        self.guards.iter().find(|g| {
            let applies = g
                .when
                .iter()
                .all(|&(c, side)| values[c as usize].truthy() == side);
            applies
                && !match &g.want {
                    Want::Truthy(t) => values[g.expr as usize].truthy() == *t,
                    Want::Eq(v) => same(&values[g.expr as usize], v),
                }
        })
    }

    /// The expressions its guards read.
    pub fn decided(&self) -> impl Iterator<Item = u32> + '_ {
        (self.guards.iter()).flat_map(|g| g.when.iter().map(|&(c, _)| c).chain([g.expr]))
    }

    /// Where a traced value reached code that cannot trace it, so the scene is traced again
    /// whenever that value changes.
    pub fn escapes(&self) -> impl Iterator<Item = &Guard> {
        self.guards.iter().filter(|g| &*g.kind == "escape")
    }
}

/// Equal as an exact-value guard compares: as `===`, except that NaN holds as NaN. (-0 and +0 are
/// one value here; JSON, which carries the traced value, does not tell them apart.)
fn same(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}
