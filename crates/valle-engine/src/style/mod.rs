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
pub mod gradient;
mod grid;
pub mod math;
pub mod position;

use std::borrow::Cow;
use std::rc::Rc;

use taffy::{
    AlignContent, AlignItems, BoxSizing, Dimension, Display, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, LengthPercentage, LengthPercentageAuto, Line, Overflow, Position,
};

use color::Rgba;
use math::{Calc, Math};

use crate::template::{Error, Node, Prop, Template, V};

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
    /// `visibility: visible`; hidden boxes paint nothing of their own.
    pub visible: bool,
    pub color: Rgba,
    pub font: Font,
    /// The math functions with percentages in them that `layout` holds handles to.
    pub calcs: Vec<Calc>,
}

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
    /// `letter-spacing`, px.
    pub spacing: f64,
}

impl Font {
    /// What the canvas gives its children: CSS's initial values, in sans-serif.
    fn initial() -> Font {
        Font {
            size: 16.0,
            weight: 400.0,
            family: Rc::new([Family::SansSerif]),
            line_height: LineHeight::Normal,
            align: TextAlign::Start,
            spacing: 0.0,
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

/// A variant's styles, bound once; `compute` gives each frame's.
pub struct Sheet {
    nodes: Vec<Bound>,
    /// Each node's parent. Parents come before their children.
    parents: Vec<Option<u32>>,
}

/// A node's declarations: those before the first one bound to an expression, applied once, and
/// from there on the rest, set every frame in order (a constant `padding` written after a
/// traced `paddingTop` overrides it).
struct Bound {
    declared: Declared,
    rest: Vec<(Field, Prop, Rc<str>)>,
    /// Where errors point.
    place: Rc<str>,
}

impl Sheet {
    pub fn new(template: &Template) -> Result<Sheet, Error> {
        let n = template.nodes.len();
        let mut parents = vec![None; n];
        for (i, node) in template.nodes.iter().enumerate() {
            for &c in &node.children {
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
        for (b, parent) in self.nodes.iter().zip(&self.parents) {
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
            let style = compute(&declared, parent, canvas, calcs)
                .map_err(|e| format!("{}: {e}", b.place))?;
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
    Ok(Bound {
        declared,
        rest,
        place,
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
    /// Inherited.
    visible: Option<bool>,
    color: Option<Rgba>,
    font_size: Option<Len>,
    font_weight: Option<Weight>,
    font_family: Option<Rc<[Family]>>,
    line_height: Option<Leading>,
    text_align: Option<TextAlign>,
    /// `normal` is `Auto`, 0.
    letter_spacing: Option<Len>,
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
            visible: None,
            color: None,
            font_size: None,
            font_weight: None,
            font_family: None,
            line_height: None,
            text_align: None,
            letter_spacing: None,
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
    BorderRadius,
    /// Of the corners from the top left clockwise.
    CornerRadius(usize),
    Border(Sides, BorderPart),
    Outline(BorderPart),
    OutlineOffset,
    BoxShadow,
    Opacity,
    Visibility,
    Color,
    FontSize,
    FontWeight,
    FontFamily,
    LineHeight,
    TextAlign,
    LetterSpacing,
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
const FIELDS: [(&str, Field); 159] = [
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
    ("visibility", Field::Visibility),
    ("color", Field::Color),
    ("fontSize", Field::FontSize),
    ("fontWeight", Field::FontWeight),
    ("fontFamily", Field::FontFamily),
    ("lineHeight", Field::LineHeight),
    ("textAlign", Field::TextAlign),
    ("letterSpacing", Field::LetterSpacing),
];

impl Field {
    fn by_name(key: &str) -> Option<Field> {
        FIELDS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|&(_, f)| f)
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
/// (the last two may be left out), a color and `inset` in any order around them.
fn shadows(v: &V) -> Result<Vec<Shadowed>, String> {
    let s = text(v)?;
    match s.as_str() {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a shadow; use none".into()),
        _ => css::commas(&s).into_iter().map(shadow).collect(),
    }
}

fn shadow(s: &str) -> Result<Shadowed, String> {
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
        F::Background => (d.layers, d.background) = background::background(&text(v)?)?,
        F::BackgroundColor => d.background = color_of(v)?,
        F::BackgroundImage => d.layers.images = background::images(&text(v)?)?,
        F::BackgroundSize => d.layers.sizes = background::sizes(&px_text(v)?)?,
        F::BackgroundPosition => d.layers.positions = background::positions(&px_text(v)?)?,
        F::BackgroundPositionAxis(axis) => {
            d.layers.positions[axis] = background::axis_positions(&px_text(v)?, axis)?
        }
        F::BackgroundRepeat => d.layers.repeats = background::repeats(&text(v)?)?,
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
        F::BoxShadow => d.shadows = shadows(v)?,
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
        F::Visibility => {
            // Collapse is hidden but in tables, which there are none of.
            let shown = [("visible", true), ("hidden", false), ("collapse", false)];
            d.visible = Some(keyword(v, &shown)?);
        }
        F::Color => d.color = Some(color_of(v)?),
        F::FontSize => d.font_size = Some(length(v, PADDING)?),
        F::FontWeight => d.font_weight = Some(weight(v)?),
        F::FontFamily => d.font_family = Some(families(v)?),
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
    }
    Ok(())
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
    };
    let display = match d.display {
        Shown::Block => Display::Block,
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
        layers: d.layers.computed(u, color),
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
        visible: d.visible.unwrap_or(inherited_visible),
        color,
        font,
        calcs: l.calcs,
    })
}

#[cfg(test)]
mod tests;
