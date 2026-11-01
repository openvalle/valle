//! The style layer. A variant's inline styles parse once into declared values; each frame sets
//! the declarations bound to expressions and computes every node top down, as CSS computes
//! styles: text properties inherit, `em` resolves against the font size and `vw` against the
//! canvas, which is the viewport, and `display` blockifies in flex and grid containers. Layout
//! reads a Taffy style; paint reads the rest.
//!
//! What the engine does not take is an error, never ignored. Properties Valle will never take say
//! why and what to write instead.

pub mod background;
pub mod color;
mod css;
pub mod filter;
pub mod gradient;
mod grid;
pub mod math;
pub mod position;
pub mod shape;
pub mod transform;

use std::borrow::Cow;
use std::rc::Rc;

use taffy::{
    AlignContent, AlignItems, BoxSizing, Dimension, Display, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, LengthPercentage, LengthPercentageAuto, Line, Overflow, Position,
};

use color::Rgba;
use math::{Calc, Math};

use crate::template::{Error, Kind, Node, Prop, Template, V};

/// The root font size: `rem`.
const REM: f64 = 16.0;

/// A node's computed style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub layout: taffy::Style,
    /// Where a flex or grid item goes among its siblings, which `layout::ordered` sorts by.
    pub order: i32,
    /// `z-index`; `None` is `auto`.
    pub z_index: Option<i32>,
    pub background: Rgba,
    /// The background's images, the first on top, with their sizes, positions and repeats.
    pub layers: Vec<background::Layer>,
    /// `border-radius`: each corner's horizontal and vertical radius, from the top left
    /// clockwise, as px and fractions of the border box's width and height.
    pub radii: [[Calc; 2]; 4],
    /// The border's sides, top, right, bottom and left: their widths in whole px, 0 without a
    /// line, and their colors.
    pub border: [f64; 4],
    pub border_colors: [Rgba; 4],
    /// The outline's width in whole px, 0 without a line, its color, and how far outside the
    /// border box it is, in whole px toward 0, as Chrome draws it.
    pub outline: f64,
    pub outline_color: Rgba,
    pub outline_offset: f64,
    /// `box-shadow`, the first on top.
    pub shadows: Vec<Shadow>,
    /// Within [0, 1].
    pub opacity: f64,
    /// `translate`, `rotate` and `scale`, then `transform`: one list, empty when all are `none`;
    /// and `transform-origin`, from the border box's top left, and its z in px.
    pub transform: Vec<transform::Function>,
    pub transform_origin: [position::Offset; 2],
    pub transform_origin_z: f64,
    /// `perspective` in px, `None` for `none`, which its children's transforms are seen in, from
    /// `perspective-origin`.
    pub perspective: Option<f64>,
    pub perspective_origin: [position::Offset; 2],
    /// `backface-visibility: visible`; a box that shows its back otherwise hides with all in it.
    pub backface_visible: bool,
    /// `transform-style: preserve-3d`: its children with transforms stay in its space rather
    /// than being flattened onto its plane, unless what is in it is grouped.
    pub preserve_3d: bool,
    /// `mix-blend-mode`, and `isolation: isolate`, which keeps the boxes in it from blending
    /// with what is under it.
    pub blend: Blend,
    pub isolate: bool,
    /// `filter` and `backdrop-filter`: empty for `none`.
    pub filter: Vec<filter::Function>,
    pub backdrop: Vec<filter::Function>,
    /// `clip-path`; `None` for `none`.
    pub clip: Option<shape::Clip>,
    /// `mask-*`: its layers, the first on top; none when every image is `none`.
    pub masks: Vec<background::Layer>,
    /// A `<Mask>`'s source, which masks all the box paints after its `mask-*` do.
    pub source: Option<Source>,
    /// `visibility: visible`; hidden boxes paint nothing of their own.
    pub visible: bool,
    pub color: Rgba,
    pub font: Font,
    /// The text decorations its text takes: those of the boxes around it, outermost first, then
    /// its own. A box out of flow takes none from around it.
    pub decorations: Vec<Decoration>,
    /// `text-overflow: ellipsis`: a Text that clips what overflows it ends a line that overflows
    /// with an ellipsis.
    pub ellipsis: bool,
    /// How many lines a Text shows before an ellipsis, its height theirs: its
    /// `-webkit-line-clamp`, with `display: -webkit-box` and `-webkit-box-orient: vertical`.
    pub clamp: Option<u32>,
    /// The math functions with percentages in them that `layout` holds handles to.
    pub calcs: Vec<Calc>,
}

/// A `<Mask>`'s source: the root node whose painting masks the box, by its alpha or its
/// luminance, inverted or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub root: usize,
    pub luminance: bool,
    pub invert: bool,
}

impl Style {
    /// Whether the box is the containing block of every box in it, fixed ones too, and a
    /// stacking context: it has a transform, a perspective, a filter or a backdrop filter, or
    /// preserves 3D, as CSS has it.
    pub fn contains_all(&self) -> bool {
        !self.transform.is_empty()
            || self.perspective.is_some()
            || self.preserve_3d
            || !self.filter.is_empty()
            || !self.backdrop.is_empty()
    }

    /// Whether its children with transforms stay in its space: it preserves 3D and nothing
    /// groups what is in it into one flat layer, as an opacity, a clip, a clip path, a mask or a
    /// `<Mask>` source, a blend, isolation or a filter of it or of its backdrop does.
    pub fn keeps_3d(&self) -> bool {
        let o = self.layout.overflow;
        self.preserve_3d
            && self.opacity >= 1.0
            && self.filter.is_empty()
            && self.backdrop.is_empty()
            && self.blend == Blend::Normal
            && !self.isolate
            && self.clip.is_none()
            && self.masks.is_empty()
            && self.source.is_none()
            && o.x == taffy::Overflow::Visible
            && o.y == taffy::Overflow::Visible
    }
}

/// A variable font's axis, by its tag, set to a value, as `font-variation-settings` sets it.
#[derive(Clone, Copy, Debug)]
pub struct Variation {
    pub tag: [u8; 4],
    pub value: f32,
}

impl PartialEq for Variation {
    fn eq(&self, other: &Variation) -> bool {
        (self.tag, self.value.to_bits()) == (other.tag, other.value.to_bits())
    }
}

impl Eq for Variation {}

impl std::hash::Hash for Variation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.tag, self.value.to_bits()).hash(state);
    }
}

/// How many of [`Variant`] there are.
pub const VARIANTS: usize = 7;

/// `font-kerning` and the `font-variant-*` longhands, which `Font::variants` holds in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    Kerning,
    Ligatures,
    Caps,
    Numeric,
    EastAsian,
    Alternates,
    Position,
}

/// An OpenType feature shaping turns on, by its tag, with its value: 1 on, 0 off, or which of a
/// set of alternates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
    pub tag: [u8; 4],
    pub value: u16,
}

/// A `text-shadow`: its offset and blur, px, and its color, `None` for `currentcolor`, the color
/// of each box that inherits it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset: [f64; 2],
    pub blur: f64,
    pub color: Option<Rgba>,
}

/// A text decoration as the box that declares it gives it to the text in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    /// Underline, overline and line-through.
    pub lines: [bool; 3],
    pub color: Rgba,
    /// `text-decoration-thickness` and `text-underline-offset`, px and fractions of the font
    /// size of the decorating box, which percentages are of; `None` is `auto`.
    pub thickness: Option<Calc>,
    pub offset: Option<Calc>,
    /// The node that declares it.
    pub by: usize,
}

/// `mix-blend-mode`: how a box's colors mix with those under it, numbered as shaders take them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

/// The last is plus-lighter, which only `mix-blend-mode` takes.
const BLENDS: [(&str, Blend); 17] = [
    ("normal", Blend::Normal),
    ("multiply", Blend::Multiply),
    ("screen", Blend::Screen),
    ("overlay", Blend::Overlay),
    ("darken", Blend::Darken),
    ("lighten", Blend::Lighten),
    ("color-dodge", Blend::ColorDodge),
    ("color-burn", Blend::ColorBurn),
    ("hard-light", Blend::HardLight),
    ("soft-light", Blend::SoftLight),
    ("difference", Blend::Difference),
    ("exclusion", Blend::Exclusion),
    ("hue", Blend::Hue),
    ("saturation", Blend::Saturation),
    ("color", Blend::Color),
    ("luminosity", Blend::Luminosity),
    ("plus-lighter", Blend::PlusLighter),
];

/// A computed box shadow, px.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub offset: [f64; 2],
    pub blur: f64,
    pub spread: f64,
    pub color: Rgba,
    pub inset: bool,
}

/// The inherited text properties, computed.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// px.
    pub size: f64,
    /// 1–1000.
    pub weight: f64,
    pub family: Rc<[Family]>,
    pub line_height: LineHeight,
    pub align: TextAlign,
    /// `letter-spacing` and `word-spacing`, px.
    pub spacing: f64,
    pub word_spacing: f64,
    /// `white-space-collapse`, and `text-wrap-mode: wrap`, which `white-space` sets together.
    pub collapse: Collapse,
    pub wrap: bool,
    /// `text-wrap-style: balance`, which the text's own box takes, not a Span's.
    pub balance: bool,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub transform: TextTransform,
    /// `font-kerning` and the `font-variant-*` longhands, by [`Variant`], each as the features it
    /// sets: they inherit one by one.
    pub variants: [Rc<[Feature]>; VARIANTS],
    /// `font-feature-settings`: OpenType features, each a tag and its value, a later one over an
    /// earlier one of its tag, and over the variants'.
    pub features: Rc<[Feature]>,
    /// `font-variation-settings`: values of a variable font's axes, over the weight's.
    pub variations: Rc<[Variation]>,
    /// `-webkit-text-fill-color`, the color of glyphs; `None` is `currentcolor`, the color of
    /// each box that inherits it.
    pub fill: Option<Rgba>,
    /// `-webkit-text-stroke-width`, px, and `-webkit-text-stroke-color` (`None` is
    /// `currentcolor`): a line along the edges of glyphs, centered on them, over their fill.
    pub stroke: f64,
    pub stroke_color: Option<Rgba>,
    /// `text-shadow`, the first on top.
    pub shadows: Rc<[TextShadow]>,
    /// `text-decoration-skip-ink: auto` (or `all`): underlines and overlines leave gaps where
    /// glyphs cross them.
    pub skip_ink: bool,
    /// `text-underline-offset`, which a decoration takes from the box that declares it.
    pub underline_offset: Option<Calc>,
}

impl Font {
    /// What a decoration's `currentcolor` is, as Chrome has it, in this font of a box of color
    /// `color`: its stroke's color if it has a stroke in another color that shows, or else the
    /// fill color of its glyphs.
    pub fn decoration_color(&self, color: Rgba) -> Rgba {
        match self.stroke_color {
            Some(c) if self.stroke > 0.0 && c[3] > 0.0 => c,
            _ => self.fill.unwrap_or(color),
        }
    }

    /// What the canvas gives its children: CSS's initial values, in sans-serif.
    fn initial() -> Font {
        Font {
            size: 16.0,
            weight: 400.0,
            family: Rc::new([Family::SansSerif]),
            line_height: LineHeight::Normal,
            align: TextAlign::Start,
            spacing: 0.0,
            word_spacing: 0.0,
            collapse: Collapse::Collapse,
            wrap: true,
            balance: false,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            transform: TextTransform::None,
            variants: Default::default(),
            features: Rc::new([]),
            variations: Rc::new([]),
            fill: None,
            stroke: 0.0,
            stroke_color: None,
            shadows: Rc::new([]),
            skip_ink: true,
            underline_offset: None,
        }
    }
}

/// A `font-family` entry: a built-in generic family, or a name for a font resource.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    SansSerif,
    Serif,
    Monospace,
    Named(Rc<str>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    /// Times the font size of each element that inherits it.
    Factor(f64),
    Px(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    End,
    Left,
    Right,
    Center,
}

/// `white-space-collapse`: which white space stays as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Collapse {
    /// Runs of spaces, tabs and line breaks are one space.
    Collapse,
    /// All of it stays; line breaks break lines.
    Preserve,
    /// Line breaks stay and break lines; runs of spaces and tabs are one space.
    PreserveBreaks,
    /// All of it stays, and spaces take room at the ends of lines rather than hanging.
    BreakSpaces,
}

/// `word-break`: where lines may break besides between words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WordBreak {
    Normal,
    /// Between any two letters.
    BreakAll,
    /// Not between CJK letters either.
    KeepAll,
    /// As `normal` with `overflow-wrap: anywhere`, whatever `overflow-wrap` is.
    BreakWord,
}

/// `overflow-wrap` (and its old name `word-wrap`): whether a word too wide for its line breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverflowWrap {
    Normal,
    /// Anywhere, which the min-content width takes into account.
    Anywhere,
    /// Anywhere, which the min-content width does not take into account.
    BreakWord,
}

/// `text-transform`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    /// The first letter of each word upper case.
    Capitalize,
}

/// A variant's styles, bound once; `compute` gives each frame's.
pub struct Sheet {
    nodes: Vec<Bound>,
    /// Each node's parent, a mask's source's the mask. Parents come before their children.
    parents: Vec<Option<u32>>,
}

/// A node's declarations: those before the first one bound to an expression, applied once, and
/// from there on the rest, set every frame in order (a constant `padding` written after a
/// traced `paddingTop` overrides it).
struct Bound {
    declared: Declared,
    /// Whether it is a Text.
    text: bool,
    rest: Vec<(Field, Prop, Rc<str>)>,
    /// Where errors point.
    place: Rc<str>,
    source: Option<Source>,
}

impl Sheet {
    pub fn new(template: &Template) -> Result<Sheet, Error> {
        let n = template.nodes.len();
        let mut parents = vec![None; n];
        for (i, node) in template.nodes.iter().enumerate() {
            let source = match node.kind {
                Kind::Mask { source, .. } => Some(source),
                _ => None,
            };
            for &c in node.children.iter().chain(&source) {
                let c = c as usize;
                if c <= i || c >= n || parents[c].is_some() {
                    return Err("engine bug: template nodes are not in preorder".into());
                }
                parents[c] = Some(i as u32);
            }
        }
        let nodes = template.nodes.iter().map(bind).collect::<Result<_, _>>()?;
        Ok(Sheet { nodes, parents })
    }

    /// Every node's computed style for the program's values at one frame on a canvas of `canvas`
    /// px, in template order.
    pub fn compute(&self, values: &[V], canvas: [f32; 2]) -> Result<Vec<Style>, Error> {
        let canvas = canvas.map(f64::from);
        let mut out: Vec<Style> = Vec::with_capacity(self.nodes.len());
        let mut calcs = 0;
        for (i, (b, parent)) in self.nodes.iter().zip(&self.parents).enumerate() {
            let declared = if b.rest.is_empty() {
                Cow::Borrowed(&b.declared)
            } else {
                let mut d = b.declared.clone();
                for (field, prop, key) in &b.rest {
                    let v = match prop {
                        Prop::Const(v) => v,
                        Prop::Expr(e) => &values[*e as usize],
                    };
                    set(&mut d, *field, v).map_err(|e| format!("{}: style {key}: {e}", b.place))?;
                }
                Cow::Owned(d)
            };
            let parent = parent.map(|p| &out[p as usize]);
            let mut style = compute(&declared, parent, canvas, calcs, i, b.text)
                .map_err(|e| format!("{}: {e}", b.place))?;
            style.source = b.source;
            calcs += style.calcs.len();
            out.push(style);
        }
        Ok(out)
    }
}

fn bind(node: &Node) -> Result<Bound, Error> {
    let place = node.place();
    if !node.classes.is_empty() {
        return Err(format!("{place}: className is not supported yet"));
    }
    let mut declared = Declared::default();
    let mut rest = Vec::new();
    for (key, prop) in &node.style {
        let field = Field::by_name(key).ok_or_else(|| format!("{place}: {}", unsupported(key)))?;
        if matches!(node.kind, Kind::Span { .. }) && !field.textual() {
            return Err(format!(
                "{place}: style {key}: a <Span> is part of a text, not a box, so it takes only \
                 text properties"
            ));
        }
        let at = |e: String| format!("{place}: style {key}: {e}");
        match prop {
            Prop::Const(v) if rest.is_empty() => set(&mut declared, field, v).map_err(at)?,
            Prop::Const(v) => {
                // Checked now, so a bad constant fails on the first frame as it would up front.
                set(&mut Declared::default(), field, v).map_err(at)?;
                rest.push((field, prop.clone(), key.clone()));
            }
            Prop::Expr(_) => rest.push((field, prop.clone(), key.clone())),
        }
    }
    let source = match node.kind {
        Kind::Mask {
            source,
            luminance,
            invert,
        } => Some(Source {
            root: source as usize,
            luminance,
            invert,
        }),
        _ => None,
    };
    Ok(Bound {
        declared,
        text: matches!(node.kind, Kind::Text { .. }),
        rest,
        place,
        source,
    })
}

/// Properties Valle will never take, by camelCase name or prefix (`*`): why, and what to write
/// instead.
const NEVER: &[(&str, &str, &str)] = &[
    ("animation*", FUNCTION_OF_TIME, ANIMATE),
    ("transition*", FUNCTION_OF_TIME, ANIMATE),
    ("scroll*", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("overscroll*", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("backgroundAttachment", NO_SCROLL, NO_SCROLL_INSTEAD),
    ("cursor", NO_INTERACTION, REMOVE),
    ("pointerEvents", NO_INTERACTION, REMOVE),
    ("userSelect", NO_INTERACTION, REMOVE),
    ("touchAction", NO_INTERACTION, REMOVE),
    ("resize", NO_INTERACTION, REMOVE),
    ("caretColor", NO_INTERACTION, REMOVE),
    ("accentColor", NO_INTERACTION, REMOVE),
    ("appearance", NO_INTERACTION, REMOVE),
    ("willChange", HINT, REMOVE),
    ("contain", HINT, REMOVE),
    ("contentVisibility", HINT, REMOVE),
    ("float", NO_FLOW, USE_FLEX),
    ("clear", NO_FLOW, USE_FLEX),
    ("columns", NO_FLOW, USE_FLEX),
    ("columnCount", NO_FLOW, USE_FLEX),
    ("columnWidth", NO_FLOW, USE_FLEX),
    ("columnFill", NO_FLOW, USE_FLEX),
    ("columnSpan", NO_FLOW, USE_FLEX),
    ("columnRule*", NO_FLOW, USE_FLEX),
    ("tableLayout", NO_FLOW, USE_FLEX),
    ("borderCollapse", NO_FLOW, USE_FLEX),
    ("borderSpacing", NO_FLOW, USE_FLEX),
    ("captionSide", NO_FLOW, USE_FLEX),
    ("emptyCells", NO_FLOW, USE_FLEX),
    (
        "listStyle*",
        NO_FLOW,
        "use flex or grid layout, and make markers and numbers in JS",
    ),
    ("counter*", NO_FLOW, "make numbers in JS"),
    ("breakBefore", NO_FLOW, USE_FLEX),
    ("breakAfter", NO_FLOW, USE_FLEX),
    ("breakInside", NO_FLOW, USE_FLEX),
    ("content", NO_FLOW, "put the text in a <Text>"),
    ("verticalAlign", NO_INLINE, INLINE_INSTEAD),
    (
        "borderImage*",
        "",
        "use a border with a gradient background, or an <Image>",
    ),
    ("zoom", "", "use scale"),
];

const FUNCTION_OF_TIME: &str = "a frame is a function of time, not of earlier frames or events";
const ANIMATE: &str = "animate with interpolate(seconds, …) or spring()";
const NO_SCROLL: &str = "the canvas does not scroll";
const NO_SCROLL_INSTEAD: &str = "use overflow: hidden, and translate to move content";
const NO_INTERACTION: &str = "a video has no interaction";
const HINT: &str = "it is a browser performance hint the engine does not need";
const REMOVE: &str = "remove it";
const NO_FLOW: &str = "there is no document flow";
const USE_FLEX: &str = "use flex or grid layout";
const NO_INLINE: &str = "there is no HTML inline flow";
const INLINE_INSTEAD: &str = "use flex layout, and Span for styled runs of text";

fn unsupported(key: &str) -> String {
    let never = NEVER
        .iter()
        .find(|(name, ..)| match name.strip_suffix('*') {
            Some(prefix) => key.starts_with(prefix),
            None => key == *name,
        });
    match never {
        Some((_, "", instead)) => format!("style {key} is not supported: {instead}"),
        Some((_, why, instead)) => format!("style {key} is not supported: {why}; {instead}"),
        None => format!("style {key} is not supported"),
    }
}

/// A length as declared; `em` resolves when the style is computed.
#[derive(Clone, Debug, PartialEq)]
enum Len {
    /// The property's keyword: `auto`, or `none` and `normal` where they stand in for it.
    Auto,
    Px(f64),
    Percent(f64),
    Em(f64),
    /// Percentages of the canvas width, height, the smaller and the larger.
    Vw(f64),
    Vh(f64),
    Vmin(f64),
    Vmax(f64),
    /// A math function's sums of the lengths above.
    Calc(Rc<math::Expr>),
    /// A size from the box's content, of `width`, `height` and `flex-basis`.
    Sizing(Sizing),
}

#[derive(Clone, Debug, PartialEq)]
enum Sizing {
    MinContent,
    MaxContent,
    /// `fit-content`: the max-content size, but no wider than the box would stretch, and no
    /// narrower than the min-content size.
    FitContent,
    Stretch,
    /// `flex-basis: content`.
    Content,
}

const AUTO: Len = Len::Auto;
const ZERO: Len = Len::Px(0.0);
const CENTER: position::Offset<Len> = position::Offset {
    end: false,
    by: Len::Percent(50.0),
};
/// The initial border width.
const MEDIUM: Len = Len::Px(3.0);
const AUTO_LINES: Line<GridPlacement<String>> = Line {
    start: GridPlacement::Auto,
    end: GridPlacement::Auto,
};

/// A grid's axes, as the arrays of its fields.
const ROWS: usize = 0;
const COLUMNS: usize = 1;

/// What lengths resolve against besides their percentage base: the font size and the canvas, in
/// px.
#[derive(Clone, Copy)]
struct Units {
    em: f64,
    canvas: [f64; 2],
}

impl Len {
    /// Whether it is below 0; a math function is not, and clamps to 0 where it has to.
    fn negative(&self) -> bool {
        match *self {
            Len::Auto | Len::Calc(_) | Len::Sizing(_) => false,
            Len::Px(v) | Len::Percent(v) | Len::Em(v) => v < 0.0,
            Len::Vw(v) | Len::Vh(v) | Len::Vmin(v) | Len::Vmax(v) => v < 0.0,
        }
    }

    /// px, against `base` for percentages; the keywords are 0.
    fn px(&self, u: Units, base: f64) -> f64 {
        let [w, h] = u.canvas;
        match self {
            Len::Auto | Len::Sizing(_) => 0.0,
            Len::Px(v) => *v,
            Len::Percent(p) => p / 100.0 * base,
            Len::Em(e) => e * u.em,
            Len::Vw(v) => v / 100.0 * w,
            Len::Vh(v) => v / 100.0 * h,
            Len::Vmin(v) => v / 100.0 * w.min(h),
            Len::Vmax(v) => v / 100.0 * w.max(h),
            Len::Calc(m) => m.value(&|ls| ls.iter().map(|l| l.px(u, base)).sum()),
        }
    }

    /// px and a fraction of what percentages are of.
    fn calc(&self, u: Units) -> Calc {
        match self {
            Len::Percent(p) => Math::Sum([0.0, p / 100.0]),
            Len::Calc(m) => m.map(&|ls: &Vec<Len>| {
                ls.iter().fold([0.0, 0.0], |[px, of], l| match l {
                    Len::Percent(p) => [px, of + p / 100.0],
                    l => [px + l.px(u, 0.0), of],
                })
            }),
            l => Math::Sum([l.px(u, 0.0), 0.0]),
        }
    }

    /// The length `k` times; a math function's sums take only the units.
    fn scaled(&self, k: f64) -> Len {
        match *self {
            Len::Px(v) => Len::Px(v * k),
            Len::Percent(v) => Len::Percent(v * k),
            Len::Em(v) => Len::Em(v * k),
            Len::Vw(v) => Len::Vw(v * k),
            Len::Vh(v) => Len::Vh(v * k),
            Len::Vmin(v) => Len::Vmin(v * k),
            Len::Vmax(v) => Len::Vmax(v * k),
            Len::Auto | Len::Calc(_) | Len::Sizing(_) => self.clone(),
        }
    }
}

/// Lengths as Taffy takes them. A math function with a percentage in it is kept for layout,
/// with a handle numbered after the `first` calcs of the frame's styles before.
struct Lengths {
    u: Units,
    first: usize,
    calcs: Vec<Calc>,
}

impl Lengths {
    /// `l` made by Taffy's constructors for px, percentages and calc handles; `auto` is the
    /// caller's.
    fn taffy<T>(
        &mut self,
        l: &Len,
        length: fn(f32) -> T,
        percent: fn(f32) -> T,
        calc: fn(*const ()) -> T,
    ) -> T {
        let u = self.u;
        match l {
            Len::Percent(p) => percent((p / 100.0) as f32),
            Len::Calc(_) => {
                let c = l.calc(u);
                if !c.any(&|&[_, of]| of != 0.0) {
                    return length(c.value(&|&[px, _]| px) as f32);
                }
                self.calcs.push(c);
                calc(math::handle(self.first + self.calcs.len() - 1))
            }
            l => length(l.px(u, 0.0) as f32),
        }
    }

    fn lpa(&mut self, l: &Len) -> LengthPercentageAuto {
        type T = LengthPercentageAuto;
        match l {
            Len::Auto => T::auto(),
            l => self.taffy(l, T::length, T::percent, T::calc),
        }
    }

    fn lp(&mut self, l: &Len) -> LengthPercentage {
        type T = LengthPercentage;
        self.taffy(l, T::length, T::percent, T::calc)
    }

    fn dim(&mut self, l: &Len) -> Dimension {
        type T = Dimension;
        match l {
            Len::Auto => T::auto(),
            Len::Sizing(s) => match s {
                Sizing::MinContent => T::min_content(),
                Sizing::MaxContent => T::max_content(),
                Sizing::FitContent => T::fit_content(),
                Sizing::Stretch => T::stretch(),
                Sizing::Content => T::content(),
            },
            l => self.taffy(l, T::length, T::percent, T::calc),
        }
    }

    /// Top, right, bottom, left.
    fn rect<T>(&mut self, s: &[Len; 4], f: fn(&mut Lengths, &Len) -> T) -> taffy::Rect<T> {
        taffy::Rect {
            top: f(self, &s[0]),
            right: f(self, &s[1]),
            bottom: f(self, &s[2]),
            left: f(self, &s[3]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Block,
    /// `-webkit-box`, for a Text that `-webkit-line-clamp` clamps, a block.
    WebkitBox,
    Flex,
    None,
    InlineBlock,
    InlineFlex,
    Grid,
    InlineGrid,
    Inline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Weight {
    Absolute(f64),
    Bolder,
    Lighter,
}

#[derive(Clone, Debug, PartialEq)]
enum Leading {
    Normal,
    Factor(f64),
    Len(Len),
}

/// A node's declared values: initial values where it declares nothing, and for the inherited
/// properties `None`, which takes the parent's.
#[derive(Clone, Debug)]
struct Declared {
    display: Shown,
    position: Position,
    /// Top, right, bottom, left, as the sides below.
    inset: [Len; 4],
    size: [Len; 2],
    min_size: [Len; 2],
    /// `none` is `Auto`.
    max_size: [Len; 2],
    box_sizing: BoxSizing,
    /// Width over height; `None` is `auto`.
    aspect_ratio: Option<f64>,
    /// x, y.
    overflow: [Overflow; 2],
    margin: [Len; 4],
    padding: [Len; 4],
    flex_direction: FlexDirection,
    flex_wrap: FlexWrap,
    flex_grow: f64,
    flex_shrink: f64,
    flex_basis: Len,
    order: i32,
    z_index: Option<i32>,
    justify_content: AlignContent,
    align_items: AlignItems,
    align_self: Option<AlignItems>,
    align_content: AlignContent,
    /// Row, column; `normal` is `Auto`, 0.
    gap: [Len; 2],
    /// Rows, columns, as the grid fields below.
    grid_template: [grid::Tracks; 2],
    grid_areas: Option<grid::Areas>,
    grid_auto: [Vec<grid::Track>; 2],
    grid_auto_flow: GridAutoFlow,
    grid_lines: [Line<GridPlacement<String>>; 2],
    justify_items: AlignItems,
    justify_self: Option<AlignItems>,
    background: Rgba,
    layers: background::Declared,
    /// Horizontal and vertical, from the top left clockwise.
    radii: [[Len; 2]; 4],
    /// Top, right, bottom and left.
    border_width: [Len; 4],
    border_style: [BorderStyle; 4],
    /// `None` is `currentcolor`.
    border_color: [Option<Rgba>; 4],
    outline_width: Len,
    outline_style: BorderStyle,
    outline_color: Option<Rgba>,
    outline_offset: Len,
    shadows: Vec<Shadowed>,
    opacity: f64,
    translate: Option<transform::Function<Len>>,
    rotate: Option<transform::Function<Len>>,
    scale: Option<transform::Function<Len>>,
    transform: Vec<transform::Function<Len>>,
    transform_origin: [position::Offset<Len>; 2],
    transform_origin_z: Len,
    perspective: Option<Len>,
    perspective_origin: [position::Offset<Len>; 2],
    backface_visible: bool,
    preserve_3d: bool,
    blend: Blend,
    isolate: bool,
    filter: Vec<filter::Function<Len, Option<Rgba>>>,
    backdrop: Vec<filter::Function<Len, Option<Rgba>>>,
    clip: Option<shape::Clip<Len>>,
    masks: background::Declared,
    /// `text-decoration-*`: its lines (under, over, through), its color (`None` is
    /// `currentcolor`) and its thickness.
    decoration_lines: [bool; 3],
    decoration_color: Option<Rgba>,
    decoration_thickness: Len,
    ellipsis: bool,
    /// `-webkit-box-orient: vertical` (or `block-axis`), and `-webkit-line-clamp`.
    box_vertical: bool,
    line_clamp: Option<u32>,
    /// Inherited.
    visible: Option<bool>,
    color: Option<Rgba>,
    font_size: Option<Len>,
    font_weight: Option<Weight>,
    font_family: Option<Rc<[Family]>>,
    variants: [Option<Rc<[Feature]>>; VARIANTS],
    features: Option<Rc<[Feature]>>,
    variations: Option<Rc<[Variation]>>,
    line_height: Option<Leading>,
    text_align: Option<TextAlign>,
    /// `normal` is `Auto`, 0.
    letter_spacing: Option<Len>,
    word_spacing: Option<Len>,
    collapse: Option<Collapse>,
    wrap: Option<bool>,
    balance: Option<bool>,
    word_break: Option<WordBreak>,
    overflow_wrap: Option<OverflowWrap>,
    text_transform: Option<TextTransform>,
    /// `Some(None)` is `currentcolor`.
    fill: Option<Option<Rgba>>,
    stroke: Option<Len>,
    stroke_color: Option<Option<Rgba>>,
    text_shadows: Option<Vec<Shadowed>>,
    skip_ink: Option<bool>,
    underline_offset: Option<Len>,
}

impl Default for Declared {
    fn default() -> Declared {
        Declared {
            display: Shown::Block,
            position: Position::Static,
            inset: [AUTO; 4],
            size: [AUTO; 2],
            min_size: [AUTO; 2],
            max_size: [AUTO; 2],
            box_sizing: BoxSizing::BorderBox,
            aspect_ratio: None,
            overflow: [Overflow::Visible; 2],
            margin: [ZERO; 4],
            padding: [ZERO; 4],
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Len::Auto,
            order: 0,
            z_index: None,
            justify_content: AlignContent::NORMAL,
            align_items: AlignItems::NORMAL,
            align_self: None,
            align_content: AlignContent::NORMAL,
            gap: [AUTO; 2],
            grid_template: Default::default(),
            grid_areas: None,
            grid_auto: Default::default(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_lines: [AUTO_LINES; 2],
            justify_items: AlignItems::NORMAL,
            justify_self: None,
            background: color::TRANSPARENT,
            layers: background::Declared::default(),
            radii: [const { [ZERO; 2] }; 4],
            border_width: [MEDIUM; 4],
            border_style: [BorderStyle::None; 4],
            border_color: [None; 4],
            outline_width: MEDIUM,
            outline_style: BorderStyle::None,
            outline_color: None,
            outline_offset: ZERO,
            shadows: Vec::new(),
            opacity: 1.0,
            translate: None,
            rotate: None,
            scale: None,
            transform: Vec::new(),
            transform_origin: [CENTER; 2],
            transform_origin_z: ZERO,
            perspective: None,
            perspective_origin: [CENTER; 2],
            backface_visible: true,
            preserve_3d: false,
            blend: Blend::Normal,
            isolate: false,
            filter: Vec::new(),
            backdrop: Vec::new(),
            clip: None,
            masks: background::Declared::default(),
            decoration_lines: [false; 3],
            decoration_color: None,
            decoration_thickness: AUTO,
            ellipsis: false,
            box_vertical: false,
            line_clamp: None,
            visible: None,
            color: None,
            font_size: None,
            font_weight: None,
            font_family: None,
            variants: Default::default(),
            features: None,
            variations: None,
            line_height: None,
            text_align: None,
            letter_spacing: None,
            word_spacing: None,
            collapse: None,
            wrap: None,
            balance: None,
            word_break: None,
            overflow_wrap: None,
            text_transform: None,
            fill: None,
            stroke: None,
            stroke_color: None,
            text_shadows: None,
            skip_ink: None,
            underline_offset: None,
        }
    }
}

/// A style property the engine takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Display,
    Position,
    Inset(Sides),
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    BoxSizing,
    AspectRatio,
    /// Both axes, or x or y.
    Overflow,
    OverflowX,
    OverflowY,
    Margin(Sides),
    Padding(Sides),
    FlexDirection,
    FlexWrap,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    Order,
    ZIndex,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    Gap,
    RowGap,
    ColumnGap,
    /// Of `ROWS` or `COLUMNS`.
    GridTemplate(usize),
    GridTemplateAreas,
    /// `grid-template`.
    GridTemplateAll,
    GridAuto(usize),
    GridAutoFlow,
    Grid,
    /// `grid-row` and `grid-column`.
    GridLines(usize),
    GridStart(usize),
    GridEnd(usize),
    GridArea,
    JustifyItems,
    JustifySelf,
    PlaceItems,
    PlaceSelf,
    PlaceContent,
    Background,
    BackgroundColor,
    BackgroundImage,
    BackgroundSize,
    BackgroundPosition,
    /// x or y.
    BackgroundPositionAxis(usize),
    BackgroundRepeat,
    BackgroundOrigin,
    BackgroundClip,
    BackgroundBlendMode,
    Translate,
    Rotate,
    Scale,
    Transform,
    TransformOrigin,
    Perspective,
    PerspectiveOrigin,
    BackfaceVisibility,
    TransformStyle,
    BorderRadius,
    /// Of the corners from the top left clockwise.
    CornerRadius(usize),
    Border(Sides, BorderPart),
    Outline(BorderPart),
    OutlineOffset,
    BoxShadow,
    Opacity,
    MixBlendMode,
    Isolation,
    Filter,
    BackdropFilter,
    ClipPath,
    MaskImage,
    MaskSize,
    MaskPosition,
    MaskRepeat,
    MaskMode,
    Visibility,
    Color,
    FontSize,
    FontWeight,
    FontFamily,
    FontFeatureSettings,
    FontVariant(Option<Variant>),
    FontVariationSettings,
    LineHeight,
    TextAlign,
    LetterSpacing,
    WordSpacing,
    WhiteSpace,
    WhiteSpaceCollapse,
    TextWrapMode,
    TextWrap,
    TextWrapStyle,
    WordBreak,
    OverflowWrap,
    TextTransform,
    TextDecoration,
    TextDecorationLine,
    TextDecorationColor,
    TextDecorationStyle,
    TextDecorationThickness,
    TextDecorationSkipInk,
    TextUnderlineOffset,
    TextOverflow,
    WebkitBoxOrient,
    WebkitLineClamp,
    WebkitTextFillColor,
    WebkitTextStroke,
    WebkitTextStrokeWidth,
    WebkitTextStrokeColor,
    TextShadow,
}

/// A declared box shadow; its color `None` is `currentcolor`.
#[derive(Clone, Debug)]
struct Shadowed {
    offset: [Len; 2],
    blur: Len,
    spread: Len,
    color: Option<Rgba>,
    inset: bool,
}

/// What a border property sets on its sides: the line, its width, style and color at once, or one
/// of those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BorderPart {
    Line,
    Width,
    Style,
    Color,
}

/// A border's line style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BorderStyle {
    None,
    Solid,
}

/// The sides a box-edge property sets: the four of a shorthand, one, or logical ones, which in
/// horizontal left-to-right text are left and right (inline) and top and bottom (block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sides {
    All,
    Top,
    Right,
    Bottom,
    Left,
    Inline,
    Block,
    InlineStart,
    InlineEnd,
    BlockStart,
    BlockEnd,
}

/// The support table: the style properties the engine takes, by their React names. The case
/// runner compares each with Chrome, and makes each other property Chrome knows a case that must
/// fail.
const FIELDS: [(&str, Field); 216] = [
    ("display", Field::Display),
    ("position", Field::Position),
    ("inset", Field::Inset(Sides::All)),
    ("top", Field::Inset(Sides::Top)),
    ("right", Field::Inset(Sides::Right)),
    ("bottom", Field::Inset(Sides::Bottom)),
    ("left", Field::Inset(Sides::Left)),
    ("insetInline", Field::Inset(Sides::Inline)),
    ("insetBlock", Field::Inset(Sides::Block)),
    ("insetInlineStart", Field::Inset(Sides::InlineStart)),
    ("insetInlineEnd", Field::Inset(Sides::InlineEnd)),
    ("insetBlockStart", Field::Inset(Sides::BlockStart)),
    ("insetBlockEnd", Field::Inset(Sides::BlockEnd)),
    ("width", Field::Width),
    ("height", Field::Height),
    ("minWidth", Field::MinWidth),
    ("minHeight", Field::MinHeight),
    ("maxWidth", Field::MaxWidth),
    ("maxHeight", Field::MaxHeight),
    ("boxSizing", Field::BoxSizing),
    ("aspectRatio", Field::AspectRatio),
    ("overflow", Field::Overflow),
    ("overflowX", Field::OverflowX),
    ("overflowY", Field::OverflowY),
    ("overflowInline", Field::OverflowX),
    ("overflowBlock", Field::OverflowY),
    ("margin", Field::Margin(Sides::All)),
    ("marginTop", Field::Margin(Sides::Top)),
    ("marginRight", Field::Margin(Sides::Right)),
    ("marginBottom", Field::Margin(Sides::Bottom)),
    ("marginLeft", Field::Margin(Sides::Left)),
    ("marginInline", Field::Margin(Sides::Inline)),
    ("marginBlock", Field::Margin(Sides::Block)),
    ("marginInlineStart", Field::Margin(Sides::InlineStart)),
    ("marginInlineEnd", Field::Margin(Sides::InlineEnd)),
    ("marginBlockStart", Field::Margin(Sides::BlockStart)),
    ("marginBlockEnd", Field::Margin(Sides::BlockEnd)),
    ("padding", Field::Padding(Sides::All)),
    ("paddingTop", Field::Padding(Sides::Top)),
    ("paddingRight", Field::Padding(Sides::Right)),
    ("paddingBottom", Field::Padding(Sides::Bottom)),
    ("paddingLeft", Field::Padding(Sides::Left)),
    ("paddingInline", Field::Padding(Sides::Inline)),
    ("paddingBlock", Field::Padding(Sides::Block)),
    ("paddingInlineStart", Field::Padding(Sides::InlineStart)),
    ("paddingInlineEnd", Field::Padding(Sides::InlineEnd)),
    ("paddingBlockStart", Field::Padding(Sides::BlockStart)),
    ("paddingBlockEnd", Field::Padding(Sides::BlockEnd)),
    ("flexDirection", Field::FlexDirection),
    ("flexWrap", Field::FlexWrap),
    ("flexGrow", Field::FlexGrow),
    ("flexShrink", Field::FlexShrink),
    ("flexBasis", Field::FlexBasis),
    ("flex", Field::Flex),
    ("order", Field::Order),
    ("zIndex", Field::ZIndex),
    ("justifyContent", Field::JustifyContent),
    ("alignItems", Field::AlignItems),
    ("alignSelf", Field::AlignSelf),
    ("alignContent", Field::AlignContent),
    ("gap", Field::Gap),
    ("rowGap", Field::RowGap),
    ("columnGap", Field::ColumnGap),
    ("gridTemplateRows", Field::GridTemplate(ROWS)),
    ("gridTemplateColumns", Field::GridTemplate(COLUMNS)),
    ("gridTemplateAreas", Field::GridTemplateAreas),
    ("gridTemplate", Field::GridTemplateAll),
    ("gridAutoRows", Field::GridAuto(ROWS)),
    ("gridAutoColumns", Field::GridAuto(COLUMNS)),
    ("gridAutoFlow", Field::GridAutoFlow),
    ("grid", Field::Grid),
    ("gridRow", Field::GridLines(ROWS)),
    ("gridColumn", Field::GridLines(COLUMNS)),
    ("gridRowStart", Field::GridStart(ROWS)),
    ("gridRowEnd", Field::GridEnd(ROWS)),
    ("gridColumnStart", Field::GridStart(COLUMNS)),
    ("gridColumnEnd", Field::GridEnd(COLUMNS)),
    ("gridArea", Field::GridArea),
    ("justifyItems", Field::JustifyItems),
    ("justifySelf", Field::JustifySelf),
    ("placeItems", Field::PlaceItems),
    ("placeSelf", Field::PlaceSelf),
    ("placeContent", Field::PlaceContent),
    ("background", Field::Background),
    ("backgroundColor", Field::BackgroundColor),
    ("backgroundImage", Field::BackgroundImage),
    ("backgroundSize", Field::BackgroundSize),
    ("backgroundPosition", Field::BackgroundPosition),
    ("backgroundPositionX", Field::BackgroundPositionAxis(0)),
    ("backgroundPositionY", Field::BackgroundPositionAxis(1)),
    ("backgroundRepeat", Field::BackgroundRepeat),
    ("backgroundOrigin", Field::BackgroundOrigin),
    ("backgroundClip", Field::BackgroundClip),
    ("backgroundBlendMode", Field::BackgroundBlendMode),
    ("borderRadius", Field::BorderRadius),
    ("borderTopLeftRadius", Field::CornerRadius(0)),
    ("borderTopRightRadius", Field::CornerRadius(1)),
    ("borderBottomRightRadius", Field::CornerRadius(2)),
    ("borderBottomLeftRadius", Field::CornerRadius(3)),
    ("borderStartStartRadius", Field::CornerRadius(0)),
    ("borderStartEndRadius", Field::CornerRadius(1)),
    ("borderEndEndRadius", Field::CornerRadius(2)),
    ("borderEndStartRadius", Field::CornerRadius(3)),
    ("border", Field::Border(Sides::All, BorderPart::Line)),
    ("borderWidth", Field::Border(Sides::All, BorderPart::Width)),
    ("borderStyle", Field::Border(Sides::All, BorderPart::Style)),
    ("borderColor", Field::Border(Sides::All, BorderPart::Color)),
    ("borderTop", Field::Border(Sides::Top, BorderPart::Line)),
    (
        "borderTopWidth",
        Field::Border(Sides::Top, BorderPart::Width),
    ),
    (
        "borderTopStyle",
        Field::Border(Sides::Top, BorderPart::Style),
    ),
    (
        "borderTopColor",
        Field::Border(Sides::Top, BorderPart::Color),
    ),
    ("borderRight", Field::Border(Sides::Right, BorderPart::Line)),
    (
        "borderRightWidth",
        Field::Border(Sides::Right, BorderPart::Width),
    ),
    (
        "borderRightStyle",
        Field::Border(Sides::Right, BorderPart::Style),
    ),
    (
        "borderRightColor",
        Field::Border(Sides::Right, BorderPart::Color),
    ),
    (
        "borderBottom",
        Field::Border(Sides::Bottom, BorderPart::Line),
    ),
    (
        "borderBottomWidth",
        Field::Border(Sides::Bottom, BorderPart::Width),
    ),
    (
        "borderBottomStyle",
        Field::Border(Sides::Bottom, BorderPart::Style),
    ),
    (
        "borderBottomColor",
        Field::Border(Sides::Bottom, BorderPart::Color),
    ),
    ("borderLeft", Field::Border(Sides::Left, BorderPart::Line)),
    (
        "borderLeftWidth",
        Field::Border(Sides::Left, BorderPart::Width),
    ),
    (
        "borderLeftStyle",
        Field::Border(Sides::Left, BorderPart::Style),
    ),
    (
        "borderLeftColor",
        Field::Border(Sides::Left, BorderPart::Color),
    ),
    (
        "borderInline",
        Field::Border(Sides::Inline, BorderPart::Line),
    ),
    (
        "borderInlineWidth",
        Field::Border(Sides::Inline, BorderPart::Width),
    ),
    (
        "borderInlineStyle",
        Field::Border(Sides::Inline, BorderPart::Style),
    ),
    (
        "borderInlineColor",
        Field::Border(Sides::Inline, BorderPart::Color),
    ),
    ("borderBlock", Field::Border(Sides::Block, BorderPart::Line)),
    (
        "borderBlockWidth",
        Field::Border(Sides::Block, BorderPart::Width),
    ),
    (
        "borderBlockStyle",
        Field::Border(Sides::Block, BorderPart::Style),
    ),
    (
        "borderBlockColor",
        Field::Border(Sides::Block, BorderPart::Color),
    ),
    (
        "borderInlineStart",
        Field::Border(Sides::InlineStart, BorderPart::Line),
    ),
    (
        "borderInlineStartWidth",
        Field::Border(Sides::InlineStart, BorderPart::Width),
    ),
    (
        "borderInlineStartStyle",
        Field::Border(Sides::InlineStart, BorderPart::Style),
    ),
    (
        "borderInlineStartColor",
        Field::Border(Sides::InlineStart, BorderPart::Color),
    ),
    (
        "borderInlineEnd",
        Field::Border(Sides::InlineEnd, BorderPart::Line),
    ),
    (
        "borderInlineEndWidth",
        Field::Border(Sides::InlineEnd, BorderPart::Width),
    ),
    (
        "borderInlineEndStyle",
        Field::Border(Sides::InlineEnd, BorderPart::Style),
    ),
    (
        "borderInlineEndColor",
        Field::Border(Sides::InlineEnd, BorderPart::Color),
    ),
    (
        "borderBlockStart",
        Field::Border(Sides::BlockStart, BorderPart::Line),
    ),
    (
        "borderBlockStartWidth",
        Field::Border(Sides::BlockStart, BorderPart::Width),
    ),
    (
        "borderBlockStartStyle",
        Field::Border(Sides::BlockStart, BorderPart::Style),
    ),
    (
        "borderBlockStartColor",
        Field::Border(Sides::BlockStart, BorderPart::Color),
    ),
    (
        "borderBlockEnd",
        Field::Border(Sides::BlockEnd, BorderPart::Line),
    ),
    (
        "borderBlockEndWidth",
        Field::Border(Sides::BlockEnd, BorderPart::Width),
    ),
    (
        "borderBlockEndStyle",
        Field::Border(Sides::BlockEnd, BorderPart::Style),
    ),
    (
        "borderBlockEndColor",
        Field::Border(Sides::BlockEnd, BorderPart::Color),
    ),
    ("outline", Field::Outline(BorderPart::Line)),
    ("outlineWidth", Field::Outline(BorderPart::Width)),
    ("outlineStyle", Field::Outline(BorderPart::Style)),
    ("outlineColor", Field::Outline(BorderPart::Color)),
    ("outlineOffset", Field::OutlineOffset),
    ("boxShadow", Field::BoxShadow),
    ("opacity", Field::Opacity),
    ("mixBlendMode", Field::MixBlendMode),
    ("isolation", Field::Isolation),
    ("filter", Field::Filter),
    ("backdropFilter", Field::BackdropFilter),
    ("clipPath", Field::ClipPath),
    ("maskImage", Field::MaskImage),
    ("maskSize", Field::MaskSize),
    ("maskPosition", Field::MaskPosition),
    ("maskRepeat", Field::MaskRepeat),
    ("maskMode", Field::MaskMode),
    ("translate", Field::Translate),
    ("rotate", Field::Rotate),
    ("scale", Field::Scale),
    ("transform", Field::Transform),
    ("transformOrigin", Field::TransformOrigin),
    ("perspective", Field::Perspective),
    ("perspectiveOrigin", Field::PerspectiveOrigin),
    ("backfaceVisibility", Field::BackfaceVisibility),
    ("transformStyle", Field::TransformStyle),
    ("visibility", Field::Visibility),
    ("color", Field::Color),
    ("fontSize", Field::FontSize),
    ("fontWeight", Field::FontWeight),
    ("fontFamily", Field::FontFamily),
    ("fontFeatureSettings", Field::FontFeatureSettings),
    ("fontKerning", Field::FontVariant(Some(Variant::Kerning))),
    ("fontVariant", Field::FontVariant(None)),
    (
        "fontVariantLigatures",
        Field::FontVariant(Some(Variant::Ligatures)),
    ),
    ("fontVariantCaps", Field::FontVariant(Some(Variant::Caps))),
    (
        "fontVariantNumeric",
        Field::FontVariant(Some(Variant::Numeric)),
    ),
    (
        "fontVariantEastAsian",
        Field::FontVariant(Some(Variant::EastAsian)),
    ),
    (
        "fontVariantAlternates",
        Field::FontVariant(Some(Variant::Alternates)),
    ),
    (
        "fontVariantPosition",
        Field::FontVariant(Some(Variant::Position)),
    ),
    ("fontVariationSettings", Field::FontVariationSettings),
    ("lineHeight", Field::LineHeight),
    ("textAlign", Field::TextAlign),
    ("letterSpacing", Field::LetterSpacing),
    ("wordSpacing", Field::WordSpacing),
    ("whiteSpace", Field::WhiteSpace),
    ("whiteSpaceCollapse", Field::WhiteSpaceCollapse),
    ("textWrapMode", Field::TextWrapMode),
    ("textWrap", Field::TextWrap),
    ("textWrapStyle", Field::TextWrapStyle),
    ("wordBreak", Field::WordBreak),
    ("overflowWrap", Field::OverflowWrap),
    ("wordWrap", Field::OverflowWrap),
    ("textTransform", Field::TextTransform),
    ("textDecoration", Field::TextDecoration),
    ("textDecorationLine", Field::TextDecorationLine),
    ("textDecorationColor", Field::TextDecorationColor),
    ("textDecorationStyle", Field::TextDecorationStyle),
    ("textDecorationThickness", Field::TextDecorationThickness),
    ("textDecorationSkipInk", Field::TextDecorationSkipInk),
    ("textUnderlineOffset", Field::TextUnderlineOffset),
    ("textOverflow", Field::TextOverflow),
    ("WebkitBoxOrient", Field::WebkitBoxOrient),
    ("WebkitLineClamp", Field::WebkitLineClamp),
    ("WebkitTextFillColor", Field::WebkitTextFillColor),
    ("WebkitTextStroke", Field::WebkitTextStroke),
    ("WebkitTextStrokeWidth", Field::WebkitTextStrokeWidth),
    ("WebkitTextStrokeColor", Field::WebkitTextStrokeColor),
    ("textShadow", Field::TextShadow),
];

impl Field {
    fn by_name(key: &str) -> Option<Field> {
        FIELDS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|&(_, f)| f)
    }

    /// Whether it styles text, as a Span takes it.
    fn textual(self) -> bool {
        matches!(
            self,
            Field::Visibility
                | Field::Color
                | Field::FontSize
                | Field::FontWeight
                | Field::FontFamily
                | Field::FontFeatureSettings
                | Field::FontVariant(_)
                | Field::FontVariationSettings
                | Field::LineHeight
                | Field::TextAlign
                | Field::LetterSpacing
                | Field::WordSpacing
                | Field::WhiteSpace
                | Field::WhiteSpaceCollapse
                | Field::TextWrapMode
                | Field::TextWrap
                | Field::TextWrapStyle
                | Field::WordBreak
                | Field::OverflowWrap
                | Field::TextTransform
                | Field::TextDecoration
                | Field::TextDecorationLine
                | Field::TextDecorationColor
                | Field::TextDecorationStyle
                | Field::TextDecorationThickness
                | Field::TextDecorationSkipInk
                | Field::TextUnderlineOffset
                | Field::WebkitTextFillColor
                | Field::WebkitTextStroke
                | Field::WebkitTextStrokeWidth
                | Field::WebkitTextStrokeColor
                | Field::TextShadow
        )
    }
}

/// The style properties the engine takes.
pub fn properties() -> impl Iterator<Item = &'static str> {
    FIELDS.iter().map(|&(name, _)| name)
}

/// What a length may be.
#[derive(Clone, Copy)]
struct Accept {
    /// The keyword that is `Len::Auto`.
    keyword: Option<&'static str>,
    negative: bool,
    percent: bool,
    /// The sizing keywords it takes.
    sizing: &'static [&'static str],
}

/// The sizing keywords of `width` and `height`.
const SIZING: &[&str] = &["min-content", "max-content", "fit-content", "stretch"];

const INSET: Accept = Accept {
    keyword: Some("auto"),
    negative: true,
    percent: true,
    sizing: &[],
};
const SIZE: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
    sizing: SIZING,
};
/// Taffy's minimum and maximum sizes are lengths only.
const MIN_SIZE: Accept = Accept {
    sizing: &[],
    ..SIZE
};
const BASIS: Accept = Accept {
    sizing: &[
        "min-content",
        "max-content",
        "fit-content",
        "stretch",
        "content",
    ],
    ..SIZE
};
const MAX_SIZE: Accept = Accept {
    keyword: Some("none"),
    ..MIN_SIZE
};
const PADDING: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};
const GAP: Accept = Accept {
    keyword: Some("normal"),
    negative: false,
    percent: true,
    sizing: &[],
};
const RADIUS: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};
/// `border-width`.
const WIDTH: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
    sizing: &[],
};
/// `box-shadow` blurs.
const BLUR: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
    sizing: &[],
};
/// `outline-offset`, and `box-shadow` offsets and spreads.
const OFFSET: Accept = Accept {
    keyword: None,
    negative: true,
    percent: false,
    sizing: &[],
};
const SPACING: Accept = Accept {
    keyword: Some("normal"),
    negative: true,
    percent: false,
    sizing: &[],
};
const THICKNESS: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
    sizing: &[],
};
const UNDERLINE_OFFSET: Accept = Accept {
    keyword: Some("auto"),
    negative: true,
    percent: true,
    sizing: &[],
};

/// A value as written in a type error.
fn shown(v: &V) -> String {
    match v {
        V::Num(x) => x.to_string(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => format!("{s:?}"),
    }
}

/// CSS text, trimmed and lowercased: keywords, units and functions are ASCII case-insensitive.
fn text(v: &V) -> Result<String, String> {
    match v {
        V::Str(s) => Ok(s.trim().to_ascii_lowercase()),
        other => Err(format!("{} is not a string", shown(other))),
    }
}

/// CSS text, or a number as itself.
fn number_text(v: &V) -> Result<String, String> {
    match v {
        V::Num(x) if x.is_finite() => Ok(x.to_string()),
        V::Num(x) => Err(format!("{x} is not a finite number")),
        _ => text(v),
    }
}

/// CSS text, or a number as a length in px.
fn px_text(v: &V) -> Result<String, String> {
    match v {
        V::Num(x) if x.is_finite() => Ok(format!("{x}px")),
        V::Num(x) => Err(format!("{x} is not a finite length")),
        _ => text(v),
    }
}

fn length_text(s: &str, a: Accept) -> Result<Len, String> {
    if Some(s) == a.keyword {
        return Ok(Len::Auto);
    }
    if let Some(sizing) = sizing(s, a)? {
        return Ok(Len::Sizing(sizing));
    }
    if math::is_math(s) {
        let m = math::length(s, |n| unit(n, a))?;
        // Out of the property's range, a math function clamps to it.
        let m = match a.negative {
            true => m,
            false => Math::Max(vec![m, Math::Sum(vec![ZERO])]),
        };
        return Ok(Len::Calc(Rc::new(m)));
    }
    let n = css::num(s).ok_or_else(|| format!("{s} is not a length"))?;
    let l = match n.unit {
        "" if n.value == 0.0 => Len::Px(0.0),
        "" => return Err(format!("{s} needs a unit: {s}px")),
        _ => unit(n, a).map_err(|e| format!("{s}: {e}"))?,
    };
    if !a.negative && l.negative() {
        return Err(format!("{s} is negative"));
    }
    Ok(l)
}

/// A sizing keyword, if `s` is one.
fn sizing(s: &str, a: Accept) -> Result<Option<Sizing>, String> {
    if css::function(s).is_some_and(|(name, _)| name == "fit-content") {
        return Err(format!(
            "{s}: browsers take fit-content() only for grid tracks; use fit-content and max-width"
        ));
    }
    if !SIZING.contains(&s) && !a.sizing.contains(&s) {
        return Ok(None);
    }
    if !a.sizing.contains(&s) {
        return Err(format!(
            "{s}: only width, height and flex-basis take sizing keywords so far"
        ));
    }
    Ok(Some(match s {
        "min-content" => Sizing::MinContent,
        "max-content" => Sizing::MaxContent,
        "fit-content" => Sizing::FitContent,
        "stretch" => Sizing::Stretch,
        _ => Sizing::Content,
    }))
}

/// A number with a unit as a length.
fn unit(n: css::Num, a: Accept) -> Result<Len, String> {
    Ok(match n.unit {
        "px" => Len::Px(n.value),
        "em" => Len::Em(n.value),
        "rem" => Len::Px(n.value * REM),
        "%" if a.percent => Len::Percent(n.value),
        "%" => return Err("percentages are not supported here".into()),
        u => match viewport(u) {
            Some(unit) => unit(n.value),
            None => return Err(format!("{u} is not supported; use {UNITS}")),
        },
    })
}

const UNITS: &str = "px, em, rem, %, vw, vh, vmin or vmax";

/// A viewport unit. The canvas is the viewport, small, large and dynamic alike, and the inline
/// and block axes are horizontal and vertical.
fn viewport(unit: &str) -> Option<fn(f64) -> Len> {
    let base = (unit.strip_prefix(['s', 'l', 'd'])).filter(|b| b.starts_with('v'));
    match base.unwrap_or(unit) {
        "vw" | "vi" => Some(Len::Vw),
        "vh" | "vb" => Some(Len::Vh),
        "vmin" => Some(Len::Vmin),
        "vmax" => Some(Len::Vmax),
        _ => None,
    }
}

/// A length; a number is px.
fn length(v: &V, a: Accept) -> Result<Len, String> {
    match v {
        V::Num(x) if !x.is_finite() => Err(format!("{x} is not a finite length")),
        V::Num(x) if !a.negative && *x < 0.0 => Err(format!("{x} is negative")),
        V::Num(x) => Ok(Len::Px(*x)),
        V::Bool(_) => Err(format!("{} is not a length", shown(v))),
        V::Str(_) => length_text(&text(v)?, a),
    }
}

/// The values of a shorthand, `what` they are: one to `most`, each a word or the one number.
fn values<T>(
    v: &V,
    most: usize,
    what: &str,
    one: impl Fn(&V) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let ts = match v {
        V::Str(_) => (css::words(&text(v)?).into_iter())
            .map(|w| one(&V::Str(w.into())))
            .collect::<Result<Vec<_>, _>>()?,
        _ => vec![one(v)?],
    };
    if ts.is_empty() || ts.len() > most {
        let many = if most == 2 {
            "one or two"
        } else {
            "one to four"
        };
        return Err(format!("{} takes {many} {what}", shown(v)));
    }
    Ok(ts)
}

/// The lengths of a shorthand: one to four for the sides of a box, one or two for gaps.
fn lengths(v: &V, a: Accept, most: usize) -> Result<Vec<Len>, String> {
    values(v, most, "lengths", |v| length(v, a))
}

/// Which of one to four values each side of a box takes, top, right, bottom and left, as CSS
/// shorthands spread them.
const SPREAD: [[usize; 4]; 4] = [[0, 0, 0, 0], [0, 1, 0, 1], [0, 1, 2, 1], [0, 1, 2, 3]];

/// The four sides of a box from one to four values.
fn sides<T: Clone>(
    v: &V,
    what: &str,
    one: impl Fn(&V) -> Result<T, String>,
) -> Result<[T; 4], String> {
    let ts = values(v, 4, what, one)?;
    Ok(SPREAD[ts.len() - 1].map(|i| ts[i].clone()))
}

/// The sides `which` names, of top, right, bottom and left; a logical pair's start, then its end.
fn named(which: Sides) -> &'static [usize] {
    use Sides as S;
    match which {
        S::All => &[0, 1, 2, 3],
        S::Top | S::BlockStart => &[0],
        S::Right | S::InlineEnd => &[1],
        S::Bottom | S::BlockEnd => &[2],
        S::Left | S::InlineStart => &[3],
        S::Inline => &[3, 1],
        S::Block => &[0, 2],
    }
}

/// Sets the sides `which` names of `sides`, which are top, right, bottom and left: all four from
/// one to four values, a logical pair from one for both or the start's and the end's, a side from
/// one.
fn set_sides<T: Clone>(
    sides: &mut [T; 4],
    which: Sides,
    v: &V,
    what: &str,
    one: impl Fn(&V) -> Result<T, String>,
) -> Result<(), String> {
    let at = named(which);
    let ts = match at.len() {
        1 => vec![one(v)?],
        n => values(v, n, what, one)?,
    };
    for (k, &side) in at.iter().enumerate() {
        let i = match at.len() {
            4 => SPREAD[ts.len() - 1][k],
            _ => k.min(ts.len() - 1),
        };
        sides[side] = ts[i].clone();
    }
    Ok(())
}

/// `border-radius`: the corners' horizontal radii, from the top left clockwise as the sides of a
/// box take lengths, then after a slash their vertical radii, which are the same without one.
fn radii(v: &V) -> Result<[[Len; 2]; 4], String> {
    let halves = match v {
        V::Str(_) => text(v)?,
        _ => return Ok(sides(v, "lengths", radius)?.map(|r| [r.clone(), r])),
    };
    let half = |h: &str| sides(&V::Str(h.into()), "lengths", radius);
    let [x, y] = match css::slashes(&halves)[..] {
        [both] => half(both).map(|x| [x.clone(), x])?,
        [x, y] => [half(x)?, half(y)?],
        _ => return Err(format!("{} takes one slash at most", shown(v))),
    };
    Ok(std::array::from_fn(|c| [x[c].clone(), y[c].clone()]))
}

fn radius(v: &V) -> Result<Len, String> {
    length(v, RADIUS)
}

/// A corner's radius: horizontal, then vertical if it differs.
fn corner(v: &V) -> Result<[Len; 2], String> {
    Ok(match &lengths(v, RADIUS, 2)?[..] {
        [x, y, ..] => [x, y].map(Len::clone),
        [both] => [both; 2].map(Len::clone),
        [] => [ZERO; 2],
    })
}

/// A border or outline shorthand's line: a width, a style (as `styled` takes it) and a color in
/// any order, each at most once, and the initial ones for those left out.
fn line(
    v: &V,
    styled: fn(&V) -> Result<BorderStyle, String>,
) -> Result<(Len, BorderStyle, Option<Rgba>), String> {
    if let V::Num(_) = v {
        return Ok((border_width(v)?, BorderStyle::None, None));
    }
    let s = text(v)?;
    let words = css::words(&s);
    if words.is_empty() {
        return Err(format!("{} is not a line", shown(v)));
    }
    let (mut width, mut style, mut color) = (None, None, None);
    for w in words {
        let word = &V::Str(w.into());
        let twice = if LINE_STYLES.contains(&w) || w == "auto" {
            style.replace(styled(word)?).is_some()
        } else if WIDTHS.contains(&w) || css::num(w).is_some() || math::is_math(w) {
            width.replace(border_width(word)?).is_some()
        } else {
            color.replace(current_color(word)?).is_some()
        };
        if twice {
            return Err(format!("{s} sets a width, a style or a color twice"));
        }
    }
    let style = style.unwrap_or(BorderStyle::None);
    Ok((width.unwrap_or(MEDIUM), style, color.flatten()))
}

const WIDTHS: [&str; 3] = ["thin", "medium", "thick"];

/// `border-width`: a length, or `thin`, `medium` or `thick`.
fn border_width(v: &V) -> Result<Len, String> {
    match text(v).as_deref() {
        Ok("thin") => Ok(Len::Px(1.0)),
        Ok("medium") => Ok(MEDIUM),
        Ok("thick") => Ok(Len::Px(5.0)),
        _ => length(v, WIDTH),
    }
}

const LINE_STYLES: [&str; 10] = [
    "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset",
];

/// `border-style`: so far `solid`, and `none` or `hidden`, which are the same without tables.
fn border_style(v: &V) -> Result<BorderStyle, String> {
    let s = text(v)?;
    match s.as_str() {
        "none" | "hidden" => Ok(BorderStyle::None),
        "solid" => Ok(BorderStyle::Solid),
        s if LINE_STYLES.contains(&s) => Err(format!("{s} lines are not supported yet; use solid")),
        s => Err(format!("{s} is not one of {}", LINE_STYLES.join(", "))),
    }
}

/// `outline-style`: so far `solid` and `none`.
fn outline_style(v: &V) -> Result<BorderStyle, String> {
    match text(v)?.as_str() {
        "auto" => Err("auto is each browser's own focus ring; use solid".into()),
        "hidden" => Err("hidden is not an outline style; use none".into()),
        _ => border_style(v),
    }
}

/// `box-shadow`: `none`, or shadows separated by commas, each two offsets, a blur and a spread
/// (the last two may be left out), a color and `inset` in any order around them. A `text-shadow`
/// has no spread and no `inset`.
fn shadows(v: &V, text_shadow: bool) -> Result<Vec<Shadowed>, String> {
    let s = text(v)?;
    match s.as_str() {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a shadow; use none".into()),
        _ => (css::commas(&s).into_iter())
            .map(|s| shadow(s, text_shadow))
            .collect(),
    }
}

fn shadow(s: &str, text_shadow: bool) -> Result<Shadowed, String> {
    let (mut lengths, mut color, mut inset) = (Vec::new(), None, false);
    let mut words = css::words(s).into_iter().peekable();
    let length_like = |w: &str| css::num(w).is_some() || math::is_math(w);
    while let Some(w) = words.next() {
        if w == "inset" && !inset {
            inset = true;
        } else if length_like(w) && lengths.is_empty() {
            lengths.push(w);
            while let Some(w) = words.next_if(|w| length_like(w)) {
                lengths.push(w);
            }
        } else if color.is_none() && !length_like(w) && w != "inset" {
            color = Some(current_color(&V::Str(w.into()))?);
        } else {
            return Err(format!(
                "{s:?} is not a shadow: two to four lengths, a color and inset"
            ));
        }
    }
    let [x, y, blur, spread] = match lengths[..] {
        _ if text_shadow && (inset || lengths.len() == 4) => {
            return Err(format!(
                "{s:?} is not a text shadow: two or three lengths (x, y and blur) and a color"
            ));
        }
        [x, y] => [x, y, "0", "0"],
        [x, y, blur] => [x, y, blur, "0"],
        [x, y, blur, spread] => [x, y, blur, spread],
        _ => {
            return Err(format!(
                "{s:?} needs two to four lengths: x, y, blur and spread"
            ));
        }
    };
    let at = |w: &str, a| length_text(w, a);
    Ok(Shadowed {
        offset: [at(x, OFFSET)?, at(y, OFFSET)?],
        blur: at(blur, BLUR)?,
        spread: at(spread, OFFSET)?,
        color: color.flatten(),
        inset,
    })
}

/// A border width as browsers compute it: whole px, and 1 px for one between 0 and 1.
fn snapped(width: f64) -> f64 {
    if width > 0.0 && width < 1.0 {
        1.0
    } else {
        width.floor()
    }
}

/// A color, or `currentcolor`, which is `None`.
fn current_color(v: &V) -> Result<Option<Rgba>, String> {
    match text(v)?.as_str() {
        "currentcolor" => Ok(None),
        s => color::parse(s).map(Some),
    }
}

fn number(v: &V) -> Result<f64, String> {
    match v {
        V::Num(x) if x.is_finite() => Ok(*x),
        V::Num(x) => Err(format!("{x} is not a finite number")),
        V::Bool(_) => Err(format!("{} is not a number", shown(v))),
        V::Str(_) => {
            let s = text(v)?;
            css::num(&s)
                .filter(|n| n.unit.is_empty())
                .map(|n| n.value)
                .ok_or_else(|| format!("{s} is not a number"))
        }
    }
}

fn factor(v: &V) -> Result<f64, String> {
    let x = number(v)?;
    if x < 0.0 {
        return Err(format!("{x} is negative"));
    }
    Ok(x)
}

fn integer(v: &V) -> Result<i32, String> {
    let x = number(v)?;
    if x.fract() != 0.0 || x.abs() > f64::from(i32::MAX) {
        return Err(format!("{x} is not an integer"));
    }
    Ok(x as i32)
}

/// `aspect-ratio`: `auto`, a ratio `w / h` or `w`, or both, which for a box without a natural
/// ratio is the ratio. A ratio with a 0 in it is degenerate, and `auto`.
fn ratio(v: &V) -> Result<Option<f64>, String> {
    let r = match v {
        V::Str(_) => {
            let s = text(v)?;
            let spaced = s.replace('/', " / ");
            let mut words = css::words(&spaced);
            let auto = (words.first() == Some(&"auto")) || (words.last() == Some(&"auto"));
            if words.first() == Some(&"auto") {
                words.remove(0);
            } else if words.last() == Some(&"auto") {
                words.pop();
            }
            let part = |w: &str| {
                css::num(w)
                    .filter(|n| n.unit.is_empty() && n.value >= 0.0)
                    .map(|n| n.value)
            };
            let bad = || format!("{s} is not an aspect ratio");
            match words[..] {
                [] if auto => return Ok(None),
                [w] => part(w).ok_or_else(bad)?,
                [w, "/", h] => match (part(w), part(h)) {
                    (Some(_), Some(0.0)) => 0.0,
                    (Some(w), Some(h)) => w / h,
                    _ => return Err(bad()),
                },
                _ => return Err(bad()),
            }
        }
        _ => factor(v)?,
    };
    Ok((r > 0.0 && r.is_finite()).then_some(r))
}

fn keyword<T: Copy>(v: &V, table: &[(&str, T)]) -> Result<T, String> {
    let s = text(v)?;
    match table.iter().find(|(k, _)| *k == s) {
        Some((_, t)) => Ok(*t),
        None => {
            let names: Vec<&str> = table.iter().map(|(k, _)| *k).collect();
            Err(format!("{s} is not one of {}", names.join(", ")))
        }
    }
}

fn display(v: &V) -> Result<Shown, String> {
    let s = text(v)?;
    if s == "table" || s == "inline-table" || s.starts_with("table-") || s == "list-item" {
        return Err(format!("{s} is not supported: {NO_FLOW}; {USE_FLEX}"));
    }
    keyword(
        v,
        &[
            ("block", Shown::Block),
            ("flex", Shown::Flex),
            ("none", Shown::None),
            ("inline-block", Shown::InlineBlock),
            ("inline-flex", Shown::InlineFlex),
            ("grid", Shown::Grid),
            ("inline-grid", Shown::InlineGrid),
            ("inline", Shown::Inline),
            ("-webkit-box", Shown::WebkitBox),
        ],
    )
}

fn position(v: &V) -> Result<Position, String> {
    if text(v)? == "sticky" {
        return Err(format!(
            "sticky is not supported: {NO_SCROLL}; use relative or absolute"
        ));
    }
    keyword(
        v,
        &[
            ("static", Position::Static),
            ("relative", Position::Relative),
            ("absolute", Position::Absolute),
            ("fixed", Position::Fixed),
        ],
    )
}

const SELF_ALIGN: [(&str, AlignItems); 10] = [
    ("normal", AlignItems::NORMAL),
    ("stretch", AlignItems::STRETCH),
    ("center", AlignItems::CENTER),
    ("start", AlignItems::START),
    ("end", AlignItems::END),
    ("flex-start", AlignItems::FLEX_START),
    ("flex-end", AlignItems::FLEX_END),
    ("self-start", AlignItems::SELF_START),
    ("self-end", AlignItems::SELF_END),
    ("baseline", AlignItems::BASELINE),
];

const CONTENT_ALIGN: [(&str, AlignContent); 10] = [
    ("normal", AlignContent::NORMAL),
    ("stretch", AlignContent::STRETCH),
    ("center", AlignContent::CENTER),
    ("start", AlignContent::START),
    ("end", AlignContent::END),
    ("flex-start", AlignContent::FLEX_START),
    ("flex-end", AlignContent::FLEX_END),
    ("space-between", AlignContent::SPACE_BETWEEN),
    ("space-around", AlignContent::SPACE_AROUND),
    ("space-evenly", AlignContent::SPACE_EVENLY),
];

/// `overflow-x` and the like: `visible`, `hidden` or `clip`.
fn overflow(v: &V) -> Result<Overflow, String> {
    match text(v)?.as_str() {
        s @ ("auto" | "scroll") => Err(format!("{s} is not supported: {NO_SCROLL}; use hidden")),
        _ => keyword(
            v,
            &[
                ("visible", Overflow::Visible),
                ("hidden", Overflow::Hidden),
                ("clip", Overflow::Clip),
            ],
        ),
    }
}

/// `align-self` and `justify-self`: `auto` takes the container's `-items`.
fn self_align(v: &V) -> Result<Option<AlignItems>, String> {
    Ok(match text(v)?.as_str() {
        "auto" => None,
        _ => Some(keyword(v, &SELF_ALIGN)?),
    })
}

/// A shorthand of two values, such as `place-items`' alignment and justification, where one value
/// is both.
fn pair<T: Clone>(v: &V, one: fn(&V) -> Result<T, String>) -> Result<(T, T), String> {
    let s = text(v)?;
    let word = |w: &str| one(&V::Str(w.into()));
    match css::words(&s)[..] {
        [both] => word(both).map(|t| (t.clone(), t)),
        [align, justify] => Ok((word(align)?, word(justify)?)),
        _ => Err(format!("{} takes one or two values", shown(v))),
    }
}

/// `flex`: grow, shrink and basis. A lone factor makes the basis `0%`, as browsers compute it.
fn flex(v: &V) -> Result<(f64, f64, Len), String> {
    if let V::Num(_) = v {
        return Ok((factor(v)?, 1.0, Len::Percent(0.0)));
    }
    let s = text(v)?;
    match s.as_str() {
        "none" => return Ok((0.0, 0.0, Len::Auto)),
        "auto" => return Ok((1.0, 1.0, Len::Auto)),
        _ => {}
    }
    // `[<grow> <shrink>?] || <basis>`: a unitless number is a factor unless two came before it.
    let mut factors = Vec::new();
    let mut basis = None;
    let mut order = String::new();
    for w in css::words(&s) {
        match css::num(w) {
            Some(n) if n.unit.is_empty() && factors.len() < 2 => {
                factors.push(factor(&V::Num(n.value))?);
                order.push('f');
            }
            _ if basis.is_none() => {
                basis = Some(length_text(w, BASIS)?);
                order.push('b');
            }
            _ => return Err(format!("{s} is not a flex value")),
        }
    }
    if !["f", "ff", "b", "fb", "ffb", "bf", "bff"].contains(&order.as_str()) {
        return Err(format!("{s} is not a flex value"));
    }
    let basis = match basis {
        Some(b) => b,
        None => Len::Percent(0.0),
    };
    Ok((
        factors.first().copied().unwrap_or(1.0),
        factors.get(1).copied().unwrap_or(1.0),
        basis,
    ))
}

fn weight(v: &V) -> Result<Weight, String> {
    if let V::Str(_) = v {
        match text(v)?.as_str() {
            "normal" => return Ok(Weight::Absolute(400.0)),
            "bold" => return Ok(Weight::Absolute(700.0)),
            "bolder" => return Ok(Weight::Bolder),
            "lighter" => return Ok(Weight::Lighter),
            _ => {}
        }
    }
    let w = number(v)?;
    if !(1.0..=1000.0).contains(&w) {
        return Err(format!("{w} is not a weight from 1 to 1000"));
    }
    Ok(Weight::Absolute(w))
}

fn leading(v: &V) -> Result<Leading, String> {
    if let V::Num(_) = v {
        return Ok(Leading::Factor(factor(v)?));
    }
    let s = text(v)?;
    if s == "normal" {
        return Ok(Leading::Normal);
    }
    match css::num(&s) {
        Some(n) if n.unit.is_empty() => Ok(Leading::Factor(factor(&V::Num(n.value))?)),
        _ => Ok(Leading::Len(length_text(&s, PADDING)?)),
    }
}

/// CSS's generic families other than the three built in.
const OTHER_GENERICS: [&str; 10] = [
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "emoji",
    "math",
    "fangsong",
];

/// The keywords of each `font-variant-*` longhand, or `font-kerning`, in groups, each keyword
/// with the features it sets; a value takes at most one of a group.
type Groups = &'static [&'static [(&'static str, &'static [(&'static [u8; 4], u16)])]];

const KERNING: Groups = &[&[("auto", &[]), ("none", &[(b"kern", 0)])]];
const LIGATURES: Groups = &[
    &[
        ("common-ligatures", &[(b"liga", 1), (b"clig", 1)]),
        ("no-common-ligatures", &[(b"liga", 0), (b"clig", 0)]),
    ],
    &[
        ("discretionary-ligatures", &[(b"dlig", 1)]),
        ("no-discretionary-ligatures", &[(b"dlig", 0)]),
    ],
    &[
        ("historical-ligatures", &[(b"hlig", 1)]),
        ("no-historical-ligatures", &[(b"hlig", 0)]),
    ],
    &[
        ("contextual", &[(b"calt", 1)]),
        ("no-contextual", &[(b"calt", 0)]),
    ],
];
const CAPS: Groups = &[&[
    ("small-caps", &[(b"smcp", 1)]),
    ("all-small-caps", &[(b"c2sc", 1), (b"smcp", 1)]),
    ("titling-caps", &[(b"titl", 1)]),
]];
const NUMERIC: Groups = &[
    &[
        ("lining-nums", &[(b"lnum", 1)]),
        ("oldstyle-nums", &[(b"onum", 1)]),
    ],
    &[
        ("proportional-nums", &[(b"pnum", 1)]),
        ("tabular-nums", &[(b"tnum", 1)]),
    ],
    &[
        ("diagonal-fractions", &[(b"frac", 1)]),
        ("stacked-fractions", &[(b"afrc", 1)]),
    ],
    &[("ordinal", &[(b"ordn", 1)])],
    &[("slashed-zero", &[(b"zero", 1)])],
];
const EAST_ASIAN: Groups = &[
    &[
        ("jis78", &[(b"jp78", 1)]),
        ("jis83", &[(b"jp83", 1)]),
        ("jis90", &[(b"jp90", 1)]),
        ("jis04", &[(b"jp04", 1)]),
        ("simplified", &[(b"smpl", 1)]),
        ("traditional", &[(b"trad", 1)]),
    ],
    &[
        ("full-width", &[(b"fwid", 1)]),
        ("proportional-width", &[(b"pwid", 1)]),
    ],
    &[("ruby", &[(b"ruby", 1)])],
];
const ALTERNATES: Groups = &[&[("historical-forms", &[(b"hist", 1)])]];
const POSITION: Groups = &[&[("sub", &[(b"subs", 1)]), ("super", &[(b"sups", 1)])]];
const NO_LIGATURES: [(&[u8; 4], u16); 5] = [
    (b"liga", 0),
    (b"clig", 0),
    (b"dlig", 0),
    (b"hlig", 0),
    (b"calt", 0),
];

impl Variant {
    const ALL: [Variant; VARIANTS] = [
        Variant::Kerning,
        Variant::Ligatures,
        Variant::Caps,
        Variant::Numeric,
        Variant::EastAsian,
        Variant::Alternates,
        Variant::Position,
    ];

    fn groups(self) -> Groups {
        match self {
            Variant::Kerning => KERNING,
            Variant::Ligatures => LIGATURES,
            Variant::Caps => CAPS,
            Variant::Numeric => NUMERIC,
            Variant::EastAsian => EAST_ASIAN,
            Variant::Alternates => ALTERNATES,
            Variant::Position => POSITION,
        }
    }
}

/// `font-kerning`, a `font-variant-*` longhand, or with no `variant` the `font-variant`
/// shorthand, which sets all of those but kerning: `normal`, `none` for ligatures, or keywords,
/// at most one of each group, as the features each longhand sets.
fn variants(v: &V, variant: Option<Variant>, d: &mut Declared) -> Result<(), String> {
    let s = text(v)?;
    let longhands = match variant {
        Some(variant) => vec![variant],
        None => Variant::ALL[1..].to_vec(),
    };
    let mut set: Vec<(Variant, Vec<Feature>)> = longhands.iter().map(|&l| (l, vec![])).collect();
    let feature = |&(tag, value): &(&[u8; 4], u16)| Feature { tag: *tag, value };
    match s.as_str() {
        "normal" => {}
        "none" if longhands.contains(&Variant::Ligatures) => {
            let ligatures = set.iter_mut().find(|(l, _)| *l == Variant::Ligatures);
            ligatures.unwrap().1 = NO_LIGATURES.iter().map(feature).collect();
        }
        _ => {
            let mut taken: Vec<(Variant, usize, &str)> = Vec::new();
            for word in css::words(&s) {
                let found = longhands.iter().find_map(|&l| {
                    let mut entries = l.groups().iter().enumerate();
                    entries.find_map(|(g, group)| {
                        let entry = group.iter().find(|(k, _)| *k == word)?;
                        Some((l, g, entry))
                    })
                });
                let Some((l, g, (_, features))) = found else {
                    return Err(match (word, variant) {
                        ("petite-caps" | "all-petite-caps" | "unicase", _) => format!(
                            "{word} is not supported yet: Chrome falls back to small capitals in \
                             fonts without its own; use small-caps or all-small-caps"
                        ),
                        (w, _) if css::function(w).is_some() => format!(
                            "{w} needs @font-feature-values, which a style cannot declare; use \
                             font-feature-settings"
                        ),
                        (_, Some(l)) => {
                            let names = l.groups().iter().flat_map(|g| g.iter().map(|(k, _)| *k));
                            let names: Vec<&str> = names.collect();
                            format!("{word} is not one of normal, {}", names.join(", "))
                        }
                        (_, None) => format!("{word} is not a keyword of a font-variant longhand"),
                    });
                };
                if let Some((_, _, other)) = taken.iter().find(|t| (t.0, t.1) == (l, g)) {
                    return Err(format!("{other} and {word} set the same: give one"));
                }
                taken.push((l, g, word));
                let into = set.iter_mut().find(|(k, _)| *k == l).unwrap();
                into.1.extend(features.iter().map(feature));
            }
        }
    }
    for (l, features) in set {
        d.variants[l as usize] = Some(features.into());
    }
    Ok(())
}

/// `font-feature-settings`: `normal`, or OpenType features, each `on` (as it is left out), `off`
/// or a value.
fn features(v: &V) -> Result<Rc<[Feature]>, String> {
    let value = |word: Option<&str>| match word.map(str::to_ascii_lowercase).as_deref() {
        None | Some("on") => Ok(1),
        Some("off") => Ok(0),
        Some(n) => match css::num(n) {
            Some(css::Num { value, unit: "" }) if value.fract() == 0.0 && value >= 0.0 => {
                u16::try_from(value as i64)
                    .map_err(|_| format!("{n} is over 65535, the largest feature value"))
            }
            _ => Err(format!("{n} is not a feature value: on, off or 0 or more")),
        },
    };
    let settings = settings(v, value)?;
    Ok((settings.into_iter())
        .map(|(tag, value)| Feature { tag, value })
        .collect())
}

/// `font-variation-settings`: `normal`, or axes of variable fonts, each with a number.
fn variations(v: &V) -> Result<Rc<[Variation]>, String> {
    let value = |word: Option<&str>| match word.map(css::num) {
        // Without -0, which would hash apart from 0.
        Some(Some(css::Num { value, unit: "" })) => Ok(value as f32 + 0.0),
        Some(_) => Err(format!(
            "{} is not an axis value: a number",
            word.unwrap_or("")
        )),
        None => Err("an axis takes a number after its tag".into()),
    };
    let settings = settings(v, value)?;
    Ok((settings.into_iter())
        .map(|(tag, value)| Variation { tag, value })
        .collect())
}

/// `normal`, or settings of OpenType tags: each a tag of 4 characters, quoted, then what `value`
/// reads from the word after it, if there is one.
fn settings<T>(
    v: &V,
    value: impl Fn(Option<&str>) -> Result<T, String>,
) -> Result<Vec<([u8; 4], T)>, String> {
    let V::Str(s) = v else {
        return Err(format!("{} is not a string", shown(v)));
    };
    if s.trim().eq_ignore_ascii_case("normal") {
        return Ok(Vec::new());
    }
    let setting = |part: &str| {
        let words = css::words(part);
        let (tag, word) = match words[..] {
            [tag] => (tag, None),
            [tag, word] => (tag, Some(word)),
            _ => return Err(format!("{part:?} is not a quoted tag and its value")),
        };
        let tag = ['"', '\''].iter().find_map(|&q| {
            (tag.strip_prefix(q))
                .and_then(|t| t.strip_suffix(q))
                .and_then(|t| <[u8; 4]>::try_from(t.as_bytes()).ok())
                .filter(|t| t.iter().all(|b| (b' '..=b'~').contains(b)))
        });
        let Some(tag) = tag else {
            return Err(format!(
                "{part:?} does not start with a tag of 4 characters, quoted"
            ));
        };
        Ok((tag, value(word)?))
    };
    css::commas(s).into_iter().map(setting).collect()
}

fn families(v: &V) -> Result<Rc<[Family]>, String> {
    let V::Str(s) = v else {
        return Err(format!("{} is not a string", shown(v)));
    };
    let mut out = Vec::new();
    for part in css::commas(s.trim()) {
        let quoted = ['"', '\''].iter().find_map(|&q| {
            part.strip_prefix(q)
                .and_then(|p| p.strip_suffix(q))
                .filter(|p| !p.contains(q))
        });
        let family = match quoted {
            Some(name) if !name.trim().is_empty() => Family::Named(name.into()),
            Some(_) => return Err(format!("{s:?} names an empty family")),
            None => {
                let words = part.split_whitespace().collect::<Vec<_>>();
                if words.is_empty() {
                    return Err(format!("{s:?} names an empty family"));
                }
                if !words.iter().all(|w| {
                    w.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                        && !w.starts_with(|c: char| c.is_ascii_digit())
                }) {
                    return Err(format!("{part} is not a family name: quote it"));
                }
                let lower = part.to_ascii_lowercase();
                match lower.as_str() {
                    "sans-serif" => Family::SansSerif,
                    "serif" => Family::Serif,
                    "monospace" => Family::Monospace,
                    g if OTHER_GENERICS.contains(&g) => {
                        return Err(format!(
                            "{g} is not built in: the built-in families are sans-serif, serif and \
                             monospace, and others come from font resources"
                        ));
                    }
                    _ => Family::Named(words.join(" ").into()),
                }
            }
        };
        out.push(family);
    }
    if out.is_empty() {
        return Err(format!("{s:?} names no family"));
    }
    Ok(out.into())
}

fn color_of(v: &V) -> Result<Rgba, String> {
    color::parse(&text(v)?)
}

fn set(d: &mut Declared, field: Field, v: &V) -> Result<(), String> {
    use Field as F;
    match field {
        F::Display => d.display = display(v)?,
        F::Position => d.position = position(v)?,
        F::Inset(s) => set_sides(&mut d.inset, s, v, "lengths", |v| length(v, INSET))?,
        F::Width => d.size[0] = length(v, SIZE)?,
        F::Height => d.size[1] = length(v, SIZE)?,
        F::MinWidth => d.min_size[0] = length(v, MIN_SIZE)?,
        F::MinHeight => d.min_size[1] = length(v, MIN_SIZE)?,
        F::MaxWidth => d.max_size[0] = length(v, MAX_SIZE)?,
        F::MaxHeight => d.max_size[1] = length(v, MAX_SIZE)?,
        F::Margin(s) => set_sides(&mut d.margin, s, v, "lengths", |v| length(v, INSET))?,
        F::Padding(s) => set_sides(&mut d.padding, s, v, "lengths", |v| length(v, PADDING))?,
        F::FlexDirection => {
            d.flex_direction = keyword(
                v,
                &[
                    ("row", FlexDirection::Row),
                    ("column", FlexDirection::Column),
                    ("row-reverse", FlexDirection::RowReverse),
                    ("column-reverse", FlexDirection::ColumnReverse),
                ],
            )?
        }
        F::FlexWrap => {
            d.flex_wrap = keyword(
                v,
                &[
                    ("nowrap", FlexWrap::NoWrap),
                    ("wrap", FlexWrap::Wrap),
                    ("wrap-reverse", FlexWrap::WrapReverse),
                ],
            )?
        }
        F::FlexGrow => d.flex_grow = factor(v)?,
        F::FlexShrink => d.flex_shrink = factor(v)?,
        F::FlexBasis => d.flex_basis = length(v, BASIS)?,
        F::Flex => (d.flex_grow, d.flex_shrink, d.flex_basis) = flex(v)?,
        F::Order => d.order = integer(v)?,
        F::ZIndex => {
            d.z_index = match v {
                V::Str(_) if text(v)? == "auto" => None,
                _ => Some(integer(v)?),
            }
        }
        F::BoxSizing => {
            d.box_sizing = keyword(
                v,
                &[
                    ("border-box", BoxSizing::BorderBox),
                    ("content-box", BoxSizing::ContentBox),
                ],
            )?
        }
        F::AspectRatio => d.aspect_ratio = ratio(v)?,
        F::Overflow => d.overflow = pair(v, overflow)?.into(),
        F::OverflowX => d.overflow[0] = overflow(v)?,
        F::OverflowY => d.overflow[1] = overflow(v)?,
        F::JustifyContent => d.justify_content = keyword(v, &CONTENT_ALIGN)?,
        F::AlignItems => d.align_items = keyword(v, &SELF_ALIGN)?,
        F::AlignSelf => d.align_self = self_align(v)?,
        F::AlignContent => d.align_content = keyword(v, &CONTENT_ALIGN)?,
        F::Gap => {
            d.gap = match &lengths(v, GAP, 2)?[..] {
                [row, column, ..] => [row, column].map(Len::clone),
                [g] => [g; 2].map(Len::clone),
                [] => [AUTO; 2],
            }
        }
        F::RowGap => d.gap[0] = length(v, GAP)?,
        F::ColumnGap => d.gap[1] = length(v, GAP)?,
        F::GridTemplate(axis) => d.grid_template[axis] = grid::template(v)?,
        F::GridTemplateAreas => d.grid_areas = grid::areas(v)?,
        F::GridTemplateAll => (d.grid_template, d.grid_areas) = grid::template_all(v)?,
        F::GridAuto(axis) => d.grid_auto[axis] = grid::auto(v)?,
        F::GridAutoFlow => d.grid_auto_flow = grid::flow(v)?,
        F::Grid => {
            let g = grid::grid(v)?;
            (d.grid_template, d.grid_areas) = (g.template, g.areas);
            (d.grid_auto, d.grid_auto_flow) = (g.auto, g.flow);
        }
        F::GridLines(axis) => d.grid_lines[axis] = grid::lines(v)?,
        F::GridStart(axis) => d.grid_lines[axis].start = grid::line(v)?,
        F::GridEnd(axis) => d.grid_lines[axis].end = grid::line(v)?,
        F::GridArea => d.grid_lines = grid::area(v)?,
        F::JustifyItems => d.justify_items = keyword(v, &SELF_ALIGN)?,
        F::JustifySelf => d.justify_self = self_align(v)?,
        F::PlaceItems => (d.align_items, d.justify_items) = pair(v, |v| keyword(v, &SELF_ALIGN))?,
        F::PlaceSelf => (d.align_self, d.justify_self) = pair(v, self_align)?,
        F::PlaceContent => {
            (d.align_content, d.justify_content) = pair(v, |v| keyword(v, &CONTENT_ALIGN))?
        }
        F::Background => {
            // The shorthand leaves the blend modes as they are.
            let blends = std::mem::take(&mut d.layers.blends);
            (d.layers, d.background) = background::background(&text(v)?)?;
            d.layers.blends = blends;
        }
        F::BackgroundColor => d.background = color_of(v)?,
        F::BackgroundImage => d.layers.images = background::images(&text(v)?)?,
        F::BackgroundSize => d.layers.sizes = background::sizes(&px_text(v)?)?,
        F::BackgroundPosition => {
            d.layers.positions = background::positions(&px_text(v)?, true)?;
        }
        F::BackgroundPositionAxis(axis) => {
            d.layers.positions[axis] = background::axis_positions(&px_text(v)?, axis)?
        }
        F::BackgroundRepeat => d.layers.repeats = background::repeats(&text(v)?)?,
        F::BackgroundOrigin => d.layers.origins = background::areas(&text(v)?, false)?,
        F::BackgroundClip => d.layers.clips = background::areas(&text(v)?, true)?,
        F::BackgroundBlendMode => d.layers.blends = background::blends(&text(v)?)?,
        F::BorderRadius => d.radii = radii(v)?,
        F::CornerRadius(c) => d.radii[c] = corner(v)?,
        F::Border(which, BorderPart::Line) => {
            let (width, style, color) = line(v, border_style)?;
            for &i in named(which) {
                d.border_width[i] = width.clone();
                d.border_style[i] = style;
                d.border_color[i] = color;
            }
        }
        F::Border(which, BorderPart::Width) => {
            set_sides(&mut d.border_width, which, v, "widths", border_width)?
        }
        F::Border(which, BorderPart::Style) => {
            set_sides(&mut d.border_style, which, v, "styles", border_style)?
        }
        F::Border(which, BorderPart::Color) => {
            set_sides(&mut d.border_color, which, v, "colors", current_color)?
        }
        F::Outline(BorderPart::Line) => {
            (d.outline_width, d.outline_style, d.outline_color) = line(v, outline_style)?
        }
        F::Outline(BorderPart::Width) => d.outline_width = border_width(v)?,
        F::Outline(BorderPart::Style) => d.outline_style = outline_style(v)?,
        F::Outline(BorderPart::Color) => d.outline_color = current_color(v)?,
        F::OutlineOffset => d.outline_offset = length(v, OFFSET)?,
        F::BoxShadow => d.shadows = shadows(v, false)?,
        F::Translate => d.translate = transform::translate(&px_text(v)?)?,
        F::Rotate => d.rotate = transform::rotate(&text(v)?)?,
        F::Scale => d.scale = transform::scale(&number_text(v)?)?,
        F::Transform => d.transform = transform::transform(&text(v)?)?,
        F::TransformOrigin => {
            (d.transform_origin, d.transform_origin_z) = transform::origin(&px_text(v)?)?;
        }
        F::Perspective => d.perspective = transform::perspective(&px_text(v)?)?,
        F::PerspectiveOrigin => {
            d.perspective_origin = transform::perspective_origin(&px_text(v)?)?;
        }
        F::BackfaceVisibility => {
            d.backface_visible = keyword(v, &[("visible", true), ("hidden", false)])?;
        }
        F::TransformStyle => {
            d.preserve_3d = keyword(v, &[("flat", false), ("preserve-3d", true)])?;
        }
        F::Opacity => {
            let o = match v {
                V::Str(_) => {
                    let s = text(v)?;
                    match css::num(&s) {
                        Some(n) if n.unit == "%" => n.value / 100.0,
                        _ => number(v)?,
                    }
                }
                _ => number(v)?,
            };
            d.opacity = o.clamp(0.0, 1.0);
        }
        F::MixBlendMode => d.blend = keyword(v, &BLENDS)?,
        F::Isolation => d.isolate = keyword(v, &[("auto", false), ("isolate", true)])?,
        F::Filter => d.filter = filter::filter(&text(v)?)?,
        F::BackdropFilter => d.backdrop = filter::filter(&text(v)?)?,
        F::ClipPath => match v {
            V::Str(s) => d.clip = shape::clip(s)?,
            other => return Err(format!("{} is not a string", shown(other))),
        },
        F::MaskImage => d.masks.images = background::images(&text(v)?)?,
        F::MaskSize => d.masks.sizes = background::sizes(&px_text(v)?)?,
        F::MaskPosition => d.masks.positions = background::positions(&px_text(v)?, false)?,
        F::MaskRepeat => d.masks.repeats = background::repeats(&text(v)?)?,
        F::MaskMode => d.masks.modes = background::modes(&text(v)?)?,
        F::Visibility => {
            // Collapse is hidden but in tables, which there are none of.
            let shown = [("visible", true), ("hidden", false), ("collapse", false)];
            d.visible = Some(keyword(v, &shown)?);
        }
        F::Color => d.color = Some(color_of(v)?),
        F::FontSize => d.font_size = Some(length(v, PADDING)?),
        F::FontWeight => d.font_weight = Some(weight(v)?),
        F::FontFamily => d.font_family = Some(families(v)?),
        F::FontFeatureSettings => d.features = Some(features(v)?),
        F::FontVariant(variant) => variants(v, variant, d)?,
        F::FontVariationSettings => d.variations = Some(variations(v)?),
        F::LineHeight => d.line_height = Some(leading(v)?),
        F::TextAlign => {
            d.text_align = Some(keyword(
                v,
                &[
                    ("start", TextAlign::Start),
                    ("end", TextAlign::End),
                    ("left", TextAlign::Left),
                    ("right", TextAlign::Right),
                    ("center", TextAlign::Center),
                ],
            )?)
        }
        F::LetterSpacing => d.letter_spacing = Some(length(v, SPACING)?),
        F::WordSpacing => d.word_spacing = Some(length(v, SPACING)?),
        F::WhiteSpace => {
            (d.collapse, d.wrap) = white_space(&text(v)?).map(|(c, w)| (Some(c), Some(w)))?
        }
        F::WhiteSpaceCollapse => d.collapse = Some(keyword(v, COLLAPSE)?),
        F::TextWrapMode => d.wrap = Some(keyword(v, WRAP)?),
        F::TextWrap => {
            (d.wrap, d.balance) = text_wrap(&text(v)?).map(|(w, b)| (Some(w), Some(b)))?
        }
        F::TextWrapStyle => d.balance = Some(wrap_style(&text(v)?)?),
        F::WordBreak => {
            d.word_break = Some(keyword(
                v,
                &[
                    ("normal", WordBreak::Normal),
                    ("break-all", WordBreak::BreakAll),
                    ("keep-all", WordBreak::KeepAll),
                    ("break-word", WordBreak::BreakWord),
                ],
            )?)
        }
        F::OverflowWrap => {
            d.overflow_wrap = Some(keyword(
                v,
                &[
                    ("normal", OverflowWrap::Normal),
                    ("anywhere", OverflowWrap::Anywhere),
                    ("break-word", OverflowWrap::BreakWord),
                ],
            )?)
        }
        F::TextTransform => {
            d.text_transform = Some(keyword(
                v,
                &[
                    ("none", TextTransform::None),
                    ("uppercase", TextTransform::Uppercase),
                    ("lowercase", TextTransform::Lowercase),
                    ("capitalize", TextTransform::Capitalize),
                ],
            )?)
        }
        F::TextDecoration => {
            (
                d.decoration_lines,
                d.decoration_color,
                d.decoration_thickness,
            ) = decoration(v)?
        }
        F::TextDecorationLine => d.decoration_lines = decoration_lines(&text(v)?)?,
        F::TextDecorationColor => d.decoration_color = current_color(v)?,
        F::TextDecorationStyle => decoration_style(&text(v)?)?,
        F::TextDecorationThickness => d.decoration_thickness = thickness_of(v)?,
        F::TextDecorationSkipInk => {
            let skips = [("auto", true), ("all", true), ("none", false)];
            d.skip_ink = Some(keyword(v, &skips)?);
        }
        F::TextUnderlineOffset => d.underline_offset = Some(length(v, UNDERLINE_OFFSET)?),
        F::WebkitTextFillColor => d.fill = Some(current_color(v)?),
        F::WebkitTextStroke => {
            let (width, color) = stroke(v)?;
            (d.stroke, d.stroke_color) = (Some(width), Some(color));
        }
        F::WebkitTextStrokeWidth => d.stroke = Some(stroke_width(v)?),
        F::WebkitTextStrokeColor => d.stroke_color = Some(current_color(v)?),
        F::TextShadow => d.text_shadows = Some(shadows(v, true)?),
        F::TextOverflow => d.ellipsis = keyword(v, &[("clip", false), ("ellipsis", true)])?,
        F::WebkitBoxOrient => {
            let orients = [
                ("horizontal", false),
                ("vertical", true),
                ("inline-axis", false),
                ("block-axis", true),
            ];
            d.box_vertical = keyword(v, &orients)?;
        }
        F::WebkitLineClamp => {
            d.line_clamp = match v {
                V::Str(_) if text(v)? == "none" => None,
                _ => match integer(v)? {
                    n if n >= 1 => Some(n as u32),
                    n => return Err(format!("{n} is not a number of lines: none, or 1 or more")),
                },
            }
        }
    }
    Ok(())
}

const DECORATION_LINES: [&str; 3] = ["underline", "overline", "line-through"];
const DECORATION_STYLES: [&str; 5] = ["solid", "double", "dotted", "dashed", "wavy"];

/// `text-decoration`: `none`, or lines, a style, a color and a thickness in any order, each at
/// most once, and the initial ones for those left out.
fn decoration(v: &V) -> Result<([bool; 3], Option<Rgba>, Len), String> {
    let s = text(v)?;
    let words = css::words(&s);
    if words.is_empty() {
        return Err(format!("{} is not a text decoration", shown(v)));
    }
    let (mut lines, mut style, mut color, mut thickness) = (Vec::new(), None, None, None);
    let mut after_line = false;
    for w in words {
        let word = &V::Str(w.into());
        let line = w == "none" || DECORATION_LINES.contains(&w);
        // The lines are one value: their words go together.
        let twice = if line {
            lines.push(w);
            lines.len() > 1 && !after_line
        } else if DECORATION_STYLES.contains(&w) {
            style.replace(decoration_style(w)?).is_some()
        } else if matches!(w, "auto" | "from-font") || css::num(w).is_some() || math::is_math(w) {
            thickness.replace(thickness_of(word)?).is_some()
        } else {
            color.replace(current_color(word)?).is_some()
        };
        if twice {
            return Err(format!(
                "{s} sets its lines, style, color or thickness twice"
            ));
        }
        after_line = line;
    }
    let lines = match lines.is_empty() {
        true => [false; 3],
        false => decoration_lines(&lines.join(" "))?,
    };
    Ok((lines, color.flatten(), thickness.unwrap_or(AUTO)))
}

/// `text-decoration-line`: `none`, or any of `underline`, `overline` and `line-through`.
fn decoration_lines(s: &str) -> Result<[bool; 3], String> {
    if s == "none" {
        return Ok([false; 3]);
    }
    let mut lines = [false; 3];
    for w in s.split_ascii_whitespace() {
        match DECORATION_LINES.iter().position(|l| *l == w) {
            Some(k) if !lines[k] => lines[k] = true,
            _ => {
                return Err(format!(
                    "{s:?} is not a text-decoration-line: none, or underline, overline and \
                     line-through, each at most once"
                ));
            }
        }
    }
    match lines.contains(&true) {
        true => Ok(lines),
        false => Err(format!("{s:?} is not a text-decoration-line")),
    }
}

/// `text-decoration-style`: so far `solid`.
fn decoration_style(s: &str) -> Result<(), String> {
    match s {
        "solid" => Ok(()),
        s if DECORATION_STYLES.contains(&s) => {
            Err(format!("{s} decorations are not supported yet; use solid"))
        }
        s => Err(format!(
            "{s:?} is not one of {}",
            DECORATION_STYLES.join(", ")
        )),
    }
}

/// `-webkit-text-stroke`: a width and a color in either order, each at most once, and the
/// initial ones for those left out.
fn stroke(v: &V) -> Result<(Len, Option<Rgba>), String> {
    if let V::Num(_) = v {
        return Ok((stroke_width(v)?, None));
    }
    let s = text(v)?;
    let words = css::words(&s);
    if words.is_empty() {
        return Err(format!("{} is not a text stroke", shown(v)));
    }
    let (mut width, mut color) = (None, None);
    for w in words {
        let word = &V::Str(w.into());
        let twice = if WIDTHS.contains(&w) || css::num(w).is_some() || math::is_math(w) {
            width.replace(stroke_width(word)?).is_some()
        } else {
            color.replace(current_color(word)?).is_some()
        };
        if twice {
            return Err(format!("{s} sets a width or a color twice"));
        }
    }
    Ok((width.unwrap_or(ZERO), color.flatten()))
}

/// `-webkit-text-stroke-width`: a length, or `thin`, `medium` or `thick`, which Chrome takes as
/// 1, 3 and 5 px of a font size of 48 px.
fn stroke_width(v: &V) -> Result<Len, String> {
    match text(v).as_deref() {
        Ok("thin") => Ok(Len::Em(1.0 / 48.0)),
        Ok("medium") => Ok(Len::Em(3.0 / 48.0)),
        Ok("thick") => Ok(Len::Em(5.0 / 48.0)),
        _ => length(v, WIDTH),
    }
}

/// `text-decoration-thickness`: `auto`, or a length or a percentage of the font size.
fn thickness_of(v: &V) -> Result<Len, String> {
    match text(v).as_deref() {
        Ok("from-font") => Err("from-font is not supported yet; use auto or a length".into()),
        _ => length(v, THICKNESS),
    }
}

const COLLAPSE: &[(&str, Collapse)] = &[
    ("collapse", Collapse::Collapse),
    ("preserve", Collapse::Preserve),
    ("preserve-breaks", Collapse::PreserveBreaks),
    ("break-spaces", Collapse::BreakSpaces),
];

const WRAP: &[(&str, bool)] = &[("wrap", true), ("nowrap", false)];

/// `text-wrap-style`: whether lines balance; `auto` and `stable`, which is `auto` without
/// editing, do not.
fn wrap_style(s: &str) -> Result<bool, String> {
    match s {
        "auto" | "stable" => Ok(false),
        "balance" => Ok(true),
        "pretty" => Err("text-wrap pretty is not supported yet; use balance or auto".into()),
        s => Err(format!("{s:?} is not one of auto, balance, stable, pretty")),
    }
}

/// `text-wrap`: a `text-wrap-mode` and a `text-wrap-style` in either order, each at most once,
/// and the initial ones for those left out.
fn text_wrap(s: &str) -> Result<(bool, bool), String> {
    let (mut wrap, mut balance) = (None, None);
    for word in s.split_ascii_whitespace() {
        let twice = match WRAP.iter().find(|(k, _)| *k == word) {
            Some(&(_, w)) => wrap.replace(w).is_some(),
            None => balance.replace(wrap_style(word)?).is_some(),
        };
        if twice {
            return Err(format!(
                "{s:?} sets a text-wrap-mode or a text-wrap-style twice"
            ));
        }
    }
    match (wrap, balance) {
        (None, None) => Err(format!("{s:?} is not a text-wrap")),
        _ => Ok((wrap.unwrap_or(true), balance.unwrap_or(false))),
    }
}

/// `white-space`: a keyword of CSS 2, or a `white-space-collapse` and a `text-wrap-mode`, either
/// left out for its initial value.
fn white_space(s: &str) -> Result<(Collapse, bool), String> {
    let legacy = match s {
        "normal" => Some((Collapse::Collapse, true)),
        "nowrap" => Some((Collapse::Collapse, false)),
        "pre" => Some((Collapse::Preserve, false)),
        "pre-wrap" => Some((Collapse::Preserve, true)),
        "pre-line" => Some((Collapse::PreserveBreaks, true)),
        _ => None,
    };
    if let Some(both) = legacy {
        return Ok(both);
    }
    let (mut collapse, mut wrap) = (None, None);
    for word in s.split_ascii_whitespace() {
        let c = COLLAPSE.iter().find(|(k, _)| *k == word);
        let w = WRAP.iter().find(|(k, _)| *k == word);
        match (c, w) {
            (Some(&(_, c)), _) if collapse.is_none() => collapse = Some(c),
            (_, Some(&(_, w))) if wrap.is_none() => wrap = Some(w),
            _ => {
                return Err(format!(
                    "{s:?} is not a white-space: normal, nowrap, pre, pre-wrap, pre-line, or a \
                     white-space-collapse and a text-wrap-mode"
                ));
            }
        }
    }
    match (collapse, wrap) {
        (None, None) => Err(format!("{s:?} is not a white-space")),
        _ => Ok((collapse.unwrap_or(Collapse::Collapse), wrap.unwrap_or(true))),
    }
}

/// CSS Fonts' relative weights.
fn bolder(w: f64) -> f64 {
    match w {
        w if w < 350.0 => 400.0,
        w if w < 550.0 => 700.0,
        w if w < 900.0 => 900.0,
        w => w,
    }
}

fn lighter(w: f64) -> f64 {
    match w {
        w if w < 100.0 => w,
        w if w < 550.0 => 100.0,
        w if w < 750.0 => 400.0,
        _ => 700.0,
    }
}

/// The computed style of a node with these declared values under `parent` (`None`: the canvas,
/// a block container with CSS's initial text properties), after `first` calcs in the frame.
fn compute(
    d: &Declared,
    parent: Option<&Style>,
    canvas: [f64; 2],
    first: usize,
    node: usize,
    text: bool,
) -> Result<Style, String> {
    let initial;
    let (inherited, inherited_color, inherited_visible, container) = match parent {
        Some(p) => (&p.font, p.color, p.visible, p.layout.display),
        None => {
            initial = Font::initial();
            (&initial, color::BLACK, true, Display::Block)
        }
    };
    let units = |em| Units { em, canvas };
    let size = match &d.font_size {
        None => inherited.size,
        Some(l) => l.px(units(inherited.size), inherited.size),
    };
    let font = Font {
        size,
        weight: match d.font_weight {
            None => inherited.weight,
            Some(Weight::Absolute(w)) => w,
            Some(Weight::Bolder) => bolder(inherited.weight),
            Some(Weight::Lighter) => lighter(inherited.weight),
        },
        family: match &d.font_family {
            None => inherited.family.clone(),
            Some(f) => f.clone(),
        },
        line_height: match &d.line_height {
            None => inherited.line_height,
            Some(Leading::Normal) => LineHeight::Normal,
            Some(Leading::Factor(f)) => LineHeight::Factor(*f),
            Some(Leading::Len(l)) => LineHeight::Px(l.px(units(size), size)),
        },
        align: d.text_align.unwrap_or(inherited.align),
        spacing: match &d.letter_spacing {
            None => inherited.spacing,
            Some(l) => l.px(units(size), 0.0),
        },
        word_spacing: match &d.word_spacing {
            None => inherited.word_spacing,
            Some(l) => l.px(units(size), 0.0),
        },
        collapse: d.collapse.unwrap_or(inherited.collapse),
        wrap: d.wrap.unwrap_or(inherited.wrap),
        balance: d.balance.unwrap_or(inherited.balance),
        word_break: d.word_break.unwrap_or(inherited.word_break),
        overflow_wrap: d.overflow_wrap.unwrap_or(inherited.overflow_wrap),
        transform: d.text_transform.unwrap_or(inherited.transform),
        variants: std::array::from_fn(|i| {
            (d.variants[i].clone()).unwrap_or_else(|| inherited.variants[i].clone())
        }),
        features: (d.features.clone()).unwrap_or_else(|| inherited.features.clone()),
        variations: (d.variations.clone()).unwrap_or_else(|| inherited.variations.clone()),
        fill: d.fill.unwrap_or(inherited.fill),
        stroke: match &d.stroke {
            None => inherited.stroke,
            Some(l) => l.px(units(size), 0.0),
        },
        stroke_color: d.stroke_color.unwrap_or(inherited.stroke_color),
        shadows: match &d.text_shadows {
            None => inherited.shadows.clone(),
            Some(list) => (list.iter())
                .map(|s| TextShadow {
                    offset: s.offset.each_ref().map(|l| l.px(units(size), 0.0)),
                    blur: s.blur.px(units(size), 0.0),
                    color: s.color,
                })
                .collect(),
        },
        skip_ink: d.skip_ink.unwrap_or(inherited.skip_ink),
        underline_offset: match &d.underline_offset {
            None => inherited.underline_offset.clone(),
            Some(Len::Auto) => None,
            Some(l) => Some(l.calc(units(size))),
        },
    };
    let display = match d.display {
        Shown::Block => Display::Block,
        Shown::WebkitBox if text => Display::Block,
        Shown::WebkitBox => {
            return Err(format!(
                "display: -webkit-box is only for a Text, which -webkit-line-clamp clamps: other \
                 boxes would lay out their children in a row, as old flexbox did; {USE_FLEX}"
            ));
        }
        Shown::Flex => Display::Flex,
        Shown::Grid => Display::Grid,
        Shown::None => Display::None,
        inline => {
            let out_of_flow = matches!(d.position, Position::Absolute | Position::Fixed);
            if !out_of_flow && !matches!(container, Display::Flex | Display::Grid) {
                let name = match inline {
                    Shown::InlineFlex => "inline-flex",
                    Shown::InlineGrid => "inline-grid",
                    Shown::InlineBlock => "inline-block",
                    _ => "inline",
                };
                return Err(format!(
                    "display: {name} needs a flex or grid parent: {NO_INLINE}; {INLINE_INSTEAD}"
                ));
            }
            // Blockified, as CSS does to flex and grid items and out-of-flow boxes.
            match inline {
                Shown::InlineFlex => Display::Flex,
                Shown::InlineGrid => Display::Grid,
                _ => Display::Block,
            }
        }
    };
    let u = units(size);
    let color = d.color.unwrap_or(inherited_color);
    let out_of_flow = matches!(d.position, Position::Absolute | Position::Fixed);
    let mut decorations = match parent {
        Some(p) if !out_of_flow => p.decorations.clone(),
        _ => Vec::new(),
    };
    if d.decoration_lines.contains(&true) {
        decorations.push(Decoration {
            lines: d.decoration_lines,
            color: d.decoration_color.unwrap_or(font.decoration_color(color)),
            thickness: match &d.decoration_thickness {
                Len::Auto => None,
                l => Some(l.calc(u)),
            },
            offset: font.underline_offset.clone(),
            by: node,
        });
    }
    let width = |style, width: &Len| match style {
        BorderStyle::None => 0.0,
        BorderStyle::Solid => snapped(width.px(u, 0.0)),
    };
    let border: [f64; 4] = std::array::from_fn(|i| width(d.border_style[i], &d.border_width[i]));
    let mut l = Lengths {
        u,
        first,
        calcs: Vec::new(),
    };
    let (grid_template_rows, grid_template_row_names) = l.template(&d.grid_template[ROWS]);
    let (grid_template_columns, grid_template_column_names) = l.template(&d.grid_template[COLUMNS]);
    let [grid_auto_rows, grid_auto_columns]: [Vec<_>; 2] =
        [ROWS, COLUMNS].map(|axis| d.grid_auto[axis].iter().map(|t| l.track(t)).collect());
    // Hidden on one axis makes the other, visible or clip, hidden too: CSS computes it to auto or
    // hidden, which clip alike when nothing scrolls.
    let [x, y] = match d.overflow {
        o if o.contains(&Overflow::Hidden) => [Overflow::Hidden; 2],
        o => o,
    };
    let layout: taffy::Style = taffy::Style {
        display,
        position: d.position,
        overflow: taffy::Point { x, y },
        inset: l.rect(&d.inset, Lengths::lpa),
        size: taffy::Size {
            width: l.dim(&d.size[0]),
            height: l.dim(&d.size[1]),
        },
        min_size: taffy::Size {
            width: l.lpa(&d.min_size[0]),
            height: l.lpa(&d.min_size[1]),
        },
        max_size: taffy::Size {
            width: l.lpa(&d.max_size[0]),
            height: l.lpa(&d.max_size[1]),
        },
        box_sizing: d.box_sizing,
        aspect_ratio: d.aspect_ratio.map(|r| r as f32),
        margin: l.rect(&d.margin, Lengths::lpa),
        padding: l.rect(&d.padding, Lengths::lp),
        border: l.rect(&border.map(Len::Px), Lengths::lp),
        flex_direction: d.flex_direction,
        flex_wrap: d.flex_wrap,
        flex_grow: d.flex_grow as f32,
        flex_shrink: d.flex_shrink as f32,
        flex_basis: l.dim(&d.flex_basis),
        justify_content: d.justify_content,
        align_items: d.align_items,
        align_self: d.align_self,
        align_content: d.align_content,
        gap: taffy::Size {
            width: l.lp(&d.gap[1]),
            height: l.lp(&d.gap[0]),
        },
        grid_template_rows,
        grid_template_columns,
        grid_template_row_names,
        grid_template_column_names,
        grid_template_areas: d.grid_areas.clone(),
        grid_auto_rows,
        grid_auto_columns,
        grid_auto_flow: d.grid_auto_flow,
        grid_row: d.grid_lines[ROWS].clone(),
        grid_column: d.grid_lines[COLUMNS].clone(),
        justify_items: d.justify_items,
        justify_self: d.justify_self,
        ..taffy::Style::DEFAULT
    };
    Ok(Style {
        layout,
        order: d.order,
        z_index: d.z_index,
        background: d.background,
        layers: d.layers.computed(u, color, background::Area::PaddingBox),
        radii: d.radii.each_ref().map(|r| r.each_ref().map(|l| l.calc(u))),
        border,
        border_colors: d.border_color.map(|c| c.unwrap_or(color)),
        outline: width(d.outline_style, &d.outline_width),
        outline_color: d.outline_color.unwrap_or(color),
        outline_offset: d.outline_offset.px(u, 0.0).trunc(),
        shadows: (d.shadows.iter())
            .map(|s| Shadow {
                offset: s.offset.each_ref().map(|l| l.px(u, 0.0)),
                blur: s.blur.px(u, 0.0),
                spread: s.spread.px(u, 0.0),
                color: s.color.unwrap_or(color),
                inset: s.inset,
            })
            .collect(),
        opacity: d.opacity,
        transform: [&d.translate, &d.rotate, &d.scale]
            .into_iter()
            .flatten()
            .chain(&d.transform)
            .map(|f| f.computed(u))
            .collect(),
        transform_origin: d.transform_origin.each_ref().map(|o| o.computed(u)),
        transform_origin_z: d.transform_origin_z.px(u, 0.0),
        perspective: d.perspective.as_ref().map(|l| l.px(u, 0.0)),
        perspective_origin: d.perspective_origin.each_ref().map(|o| o.computed(u)),
        backface_visible: d.backface_visible,
        preserve_3d: d.preserve_3d,
        blend: d.blend,
        isolate: d.isolate,
        filter: d.filter.iter().map(|f| f.computed(u, color)).collect(),
        backdrop: d.backdrop.iter().map(|f| f.computed(u, color)).collect(),
        clip: d.clip.as_ref().map(|c| c.computed(u)),
        masks: match d
            .masks
            .images
            .iter()
            .all(|i| matches!(i, gradient::Image::None))
        {
            true => Vec::new(),
            false => d.masks.computed(u, color, background::Area::BorderBox),
        },
        source: None,
        visible: d.visible.unwrap_or(inherited_visible),
        color,
        font,
        decorations,
        ellipsis: d.ellipsis,
        clamp: match (d.display, d.box_vertical) {
            (Shown::WebkitBox, true) => d.line_clamp,
            _ => None,
        },
        calcs: l.calcs,
    })
}

#[cfg(test)]
mod tests;
