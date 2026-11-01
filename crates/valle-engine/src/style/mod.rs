//! The style layer. A variant's inline styles parse once into declared values; each frame sets
//! the declarations bound to expressions and computes every node top down, as CSS computes
//! styles: text properties inherit, `em` resolves against the font size, and `display`
//! blockifies in flex containers. Layout reads a Taffy style; paint reads the rest.
//!
//! What the engine does not take is an error, never ignored. Properties Valle will never take say
//! why and what to write instead.

pub mod color;
mod css;

use std::borrow::Cow;
use std::rc::Rc;

use taffy::{
    AlignContent, AlignItems, Dimension, Display, FlexDirection, FlexWrap, LengthPercentage,
    LengthPercentageAuto, Position,
};

use color::Rgba;

use crate::template::{Error, Node, Prop, Template, V};

/// The root font size: `rem`.
const REM: f64 = 16.0;

/// A node's computed style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub layout: taffy::Style,
    pub background: Rgba,
    /// `border-radius`, px.
    pub radius: f64,
    /// Within [0, 1].
    pub opacity: f64,
    pub color: Rgba,
    pub font: Font,
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

    /// Every node's computed style for the program's values at one frame, in template order.
    pub fn compute(&self, values: &[V]) -> Result<Vec<Style>, Error> {
        let mut out: Vec<Style> = Vec::with_capacity(self.nodes.len());
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
            let style = compute(&declared, parent).map_err(|e| format!("{}: {e}", b.place))?;
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
#[derive(Clone, Copy, Debug, PartialEq)]
enum Len {
    /// The property's keyword: `auto`, or `none` and `normal` where they stand in for it.
    Auto,
    Px(f64),
    Percent(f64),
    Em(f64),
}

impl Len {
    fn negative(self) -> bool {
        match self {
            Len::Auto => false,
            Len::Px(v) | Len::Percent(v) | Len::Em(v) => v < 0.0,
        }
    }

    /// px, against a font size for `em` and a base for percentages; the keyword is 0.
    fn px(self, em: f64, base: f64) -> f64 {
        match self {
            Len::Auto => 0.0,
            Len::Px(v) => v,
            Len::Percent(p) => p / 100.0 * base,
            Len::Em(e) => e * em,
        }
    }
}

fn lpa(l: Len, em: f64) -> LengthPercentageAuto {
    match l {
        Len::Auto => LengthPercentageAuto::auto(),
        Len::Percent(p) => LengthPercentageAuto::percent((p / 100.0) as f32),
        l => LengthPercentageAuto::length(l.px(em, 0.0) as f32),
    }
}

fn lp(l: Len, em: f64) -> LengthPercentage {
    match l {
        Len::Percent(p) => LengthPercentage::percent((p / 100.0) as f32),
        l => LengthPercentage::length(l.px(em, 0.0) as f32),
    }
}

fn dim(l: Len, em: f64) -> Dimension {
    match l {
        Len::Auto => Dimension::auto(),
        Len::Percent(p) => Dimension::percent((p / 100.0) as f32),
        l => Dimension::length(l.px(em, 0.0) as f32),
    }
}

/// Top, right, bottom, left.
fn rect<T>(s: [Len; 4], em: f64, f: fn(Len, f64) -> T) -> taffy::Rect<T> {
    taffy::Rect {
        top: f(s[0], em),
        right: f(s[1], em),
        bottom: f(s[2], em),
        left: f(s[3], em),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Block,
    Flex,
    None,
    InlineBlock,
    InlineFlex,
    Inline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Weight {
    Absolute(f64),
    Bolder,
    Lighter,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
    margin: [Len; 4],
    padding: [Len; 4],
    flex_direction: FlexDirection,
    flex_wrap: FlexWrap,
    flex_grow: f64,
    flex_shrink: f64,
    flex_basis: Len,
    justify_content: AlignContent,
    align_items: AlignItems,
    align_self: Option<AlignItems>,
    align_content: AlignContent,
    /// Row, column; `normal` is `Auto`, 0.
    gap: [Len; 2],
    background: Rgba,
    radius: Len,
    opacity: f64,
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
        let zero = Len::Px(0.0);
        Declared {
            display: Shown::Block,
            position: Position::Static,
            inset: [Len::Auto; 4],
            size: [Len::Auto; 2],
            min_size: [Len::Auto; 2],
            max_size: [Len::Auto; 2],
            margin: [zero; 4],
            padding: [zero; 4],
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Len::Auto,
            justify_content: AlignContent::NORMAL,
            align_items: AlignItems::NORMAL,
            align_self: None,
            align_content: AlignContent::NORMAL,
            gap: [Len::Auto; 2],
            background: color::TRANSPARENT,
            radius: zero,
            opacity: 1.0,
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
    Top,
    Right,
    Bottom,
    Left,
    Inset,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    FlexDirection,
    FlexWrap,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    Gap,
    RowGap,
    ColumnGap,
    Background,
    BackgroundColor,
    BorderRadius,
    Opacity,
    Color,
    FontSize,
    FontWeight,
    FontFamily,
    LineHeight,
    TextAlign,
    LetterSpacing,
}

impl Field {
    fn by_name(key: &str) -> Option<Field> {
        use Field as F;
        Some(match key {
            "display" => F::Display,
            "position" => F::Position,
            "top" => F::Top,
            "right" => F::Right,
            "bottom" => F::Bottom,
            "left" => F::Left,
            "inset" => F::Inset,
            "width" => F::Width,
            "height" => F::Height,
            "minWidth" => F::MinWidth,
            "minHeight" => F::MinHeight,
            "maxWidth" => F::MaxWidth,
            "maxHeight" => F::MaxHeight,
            "margin" => F::Margin,
            "marginTop" => F::MarginTop,
            "marginRight" => F::MarginRight,
            "marginBottom" => F::MarginBottom,
            "marginLeft" => F::MarginLeft,
            "padding" => F::Padding,
            "paddingTop" => F::PaddingTop,
            "paddingRight" => F::PaddingRight,
            "paddingBottom" => F::PaddingBottom,
            "paddingLeft" => F::PaddingLeft,
            "flexDirection" => F::FlexDirection,
            "flexWrap" => F::FlexWrap,
            "flexGrow" => F::FlexGrow,
            "flexShrink" => F::FlexShrink,
            "flexBasis" => F::FlexBasis,
            "flex" => F::Flex,
            "justifyContent" => F::JustifyContent,
            "alignItems" => F::AlignItems,
            "alignSelf" => F::AlignSelf,
            "alignContent" => F::AlignContent,
            "gap" => F::Gap,
            "rowGap" => F::RowGap,
            "columnGap" => F::ColumnGap,
            "background" => F::Background,
            "backgroundColor" => F::BackgroundColor,
            "borderRadius" => F::BorderRadius,
            "opacity" => F::Opacity,
            "color" => F::Color,
            "fontSize" => F::FontSize,
            "fontWeight" => F::FontWeight,
            "fontFamily" => F::FontFamily,
            "lineHeight" => F::LineHeight,
            "textAlign" => F::TextAlign,
            "letterSpacing" => F::LetterSpacing,
            _ => return None,
        })
    }
}

/// What a length may be.
#[derive(Clone, Copy)]
struct Accept {
    /// The keyword that is `Len::Auto`.
    keyword: Option<&'static str>,
    negative: bool,
    percent: bool,
}

const INSET: Accept = Accept {
    keyword: Some("auto"),
    negative: true,
    percent: true,
};
const SIZE: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
};
const MAX_SIZE: Accept = Accept {
    keyword: Some("none"),
    negative: false,
    percent: true,
};
const PADDING: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
};
const GAP: Accept = Accept {
    keyword: Some("normal"),
    negative: false,
    percent: true,
};
const RADIUS: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
};
const SPACING: Accept = Accept {
    keyword: Some("normal"),
    negative: true,
    percent: false,
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

fn length_text(s: &str, a: Accept) -> Result<Len, String> {
    if Some(s) == a.keyword {
        return Ok(Len::Auto);
    }
    let n = css::num(s).ok_or_else(|| format!("{s} is not a length"))?;
    let l = match n.unit {
        "px" => Len::Px(n.value),
        "em" => Len::Em(n.value),
        "rem" => Len::Px(n.value * REM),
        "%" if a.percent => Len::Percent(n.value),
        "%" => return Err(format!("{s}: percentages are not supported here")),
        "" if n.value == 0.0 => Len::Px(0.0),
        "" => return Err(format!("{s} needs a unit: {s}px")),
        "vw" | "vh" | "vmin" | "vmax" => {
            return Err(format!("{s}: {} is not supported yet", n.unit));
        }
        u => return Err(format!("{s}: {u} is not supported; use px, em, rem or %")),
    };
    if !a.negative && l.negative() {
        return Err(format!("{s} is negative"));
    }
    Ok(l)
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

/// The lengths of a shorthand: one to four for the sides of a box, one or two for gaps.
fn lengths(v: &V, a: Accept, most: usize) -> Result<Vec<Len>, String> {
    let ls = match v {
        V::Str(_) => {
            let s = text(v)?;
            css::words(&s)
                .into_iter()
                .map(|w| length_text(w, a))
                .collect::<Result<Vec<_>, _>>()?
        }
        _ => vec![length(v, a)?],
    };
    if ls.is_empty() || ls.len() > most {
        let many = if most == 2 {
            "one or two"
        } else {
            "one to four"
        };
        return Err(format!("{} takes {many} lengths", shown(v)));
    }
    Ok(ls)
}

/// The four sides of a box as CSS shorthands give them.
fn sides(v: &V, a: Accept) -> Result<[Len; 4], String> {
    Ok(match lengths(v, a, 4)?[..] {
        [x] => [x; 4],
        [y, x] => [y, x, y, x],
        [t, x, b] => [t, x, b, x],
        [t, r, b, l, ..] => [t, r, b, l],
        [] => [Len::Px(0.0); 4],
    })
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
    if s == "grid" || s == "inline-grid" {
        return Err(format!("{s} is not supported yet"));
    }
    keyword(
        v,
        &[
            ("block", Shown::Block),
            ("flex", Shown::Flex),
            ("none", Shown::None),
            ("inline-block", Shown::InlineBlock),
            ("inline-flex", Shown::InlineFlex),
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
                basis = Some(length_text(w, SIZE)?);
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
        F::Top => d.inset[0] = length(v, INSET)?,
        F::Right => d.inset[1] = length(v, INSET)?,
        F::Bottom => d.inset[2] = length(v, INSET)?,
        F::Left => d.inset[3] = length(v, INSET)?,
        F::Inset => d.inset = sides(v, INSET)?,
        F::Width => d.size[0] = length(v, SIZE)?,
        F::Height => d.size[1] = length(v, SIZE)?,
        F::MinWidth => d.min_size[0] = length(v, SIZE)?,
        F::MinHeight => d.min_size[1] = length(v, SIZE)?,
        F::MaxWidth => d.max_size[0] = length(v, MAX_SIZE)?,
        F::MaxHeight => d.max_size[1] = length(v, MAX_SIZE)?,
        F::Margin => d.margin = sides(v, INSET)?,
        F::MarginTop => d.margin[0] = length(v, INSET)?,
        F::MarginRight => d.margin[1] = length(v, INSET)?,
        F::MarginBottom => d.margin[2] = length(v, INSET)?,
        F::MarginLeft => d.margin[3] = length(v, INSET)?,
        F::Padding => d.padding = sides(v, PADDING)?,
        F::PaddingTop => d.padding[0] = length(v, PADDING)?,
        F::PaddingRight => d.padding[1] = length(v, PADDING)?,
        F::PaddingBottom => d.padding[2] = length(v, PADDING)?,
        F::PaddingLeft => d.padding[3] = length(v, PADDING)?,
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
        F::FlexBasis => d.flex_basis = length(v, SIZE)?,
        F::Flex => (d.flex_grow, d.flex_shrink, d.flex_basis) = flex(v)?,
        F::JustifyContent => d.justify_content = keyword(v, &CONTENT_ALIGN)?,
        F::AlignItems => d.align_items = keyword(v, &SELF_ALIGN)?,
        F::AlignSelf => {
            d.align_self = match text(v)?.as_str() {
                "auto" => None,
                _ => Some(keyword(v, &SELF_ALIGN)?),
            }
        }
        F::AlignContent => d.align_content = keyword(v, &CONTENT_ALIGN)?,
        F::Gap => {
            d.gap = match lengths(v, GAP, 2)?[..] {
                [row, column, ..] => [row, column],
                [g] => [g, g],
                [] => [Len::Auto; 2],
            }
        }
        F::RowGap => d.gap[0] = length(v, GAP)?,
        F::ColumnGap => d.gap[1] = length(v, GAP)?,
        F::Background => {
            let s = text(v)?;
            d.background = match s.as_str() {
                "none" => color::TRANSPARENT,
                _ => color::parse(&s).map_err(|e| {
                    format!(
                        "{e}: background takes only a color so far; images are not supported yet"
                    )
                })?,
            }
        }
        F::BackgroundColor => d.background = color_of(v)?,
        F::BorderRadius => {
            if matches!(v, V::Str(s) if css::words(s).len() > 1) {
                return Err(format!(
                    "{}: one radius for every corner is supported so far",
                    shown(v)
                ));
            }
            d.radius = length(v, RADIUS)?
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
/// a block container with CSS's initial text properties).
fn compute(d: &Declared, parent: Option<&Style>) -> Result<Style, String> {
    let canvas;
    let (inherited, inherited_color, container) = match parent {
        Some(p) => (&p.font, p.color, p.layout.display),
        None => {
            canvas = Font::initial();
            (&canvas, color::BLACK, Display::Block)
        }
    };
    let size = match d.font_size {
        None => inherited.size,
        Some(l) => l.px(inherited.size, inherited.size),
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
        line_height: match d.line_height {
            None => inherited.line_height,
            Some(Leading::Normal) => LineHeight::Normal,
            Some(Leading::Factor(f)) => LineHeight::Factor(f),
            Some(Leading::Len(l)) => LineHeight::Px(l.px(size, size)),
        },
        align: d.text_align.unwrap_or(inherited.align),
        spacing: match d.letter_spacing {
            None => inherited.spacing,
            Some(l) => l.px(size, 0.0),
        },
    };
    let display = match d.display {
        Shown::Block => Display::Block,
        Shown::Flex => Display::Flex,
        Shown::None => Display::None,
        inline => {
            let out_of_flow = matches!(d.position, Position::Absolute | Position::Fixed);
            if !out_of_flow && container != Display::Flex {
                let name = match inline {
                    Shown::InlineFlex => "inline-flex",
                    Shown::InlineBlock => "inline-block",
                    _ => "inline",
                };
                return Err(format!(
                    "display: {name} needs a flex parent: {NO_INLINE}; {INLINE_INSTEAD}"
                ));
            }
            // Blockified, as CSS does to flex items and out-of-flow boxes.
            match inline {
                Shown::InlineFlex => Display::Flex,
                _ => Display::Block,
            }
        }
    };
    let em = size;
    let layout: taffy::Style = taffy::Style {
        display,
        position: d.position,
        inset: rect(d.inset, em, lpa),
        size: taffy::Size {
            width: dim(d.size[0], em),
            height: dim(d.size[1], em),
        },
        min_size: taffy::Size {
            width: lpa(d.min_size[0], em),
            height: lpa(d.min_size[1], em),
        },
        max_size: taffy::Size {
            width: lpa(d.max_size[0], em),
            height: lpa(d.max_size[1], em),
        },
        margin: rect(d.margin, em, lpa),
        padding: rect(d.padding, em, lp),
        flex_direction: d.flex_direction,
        flex_wrap: d.flex_wrap,
        flex_grow: d.flex_grow as f32,
        flex_shrink: d.flex_shrink as f32,
        flex_basis: dim(d.flex_basis, em),
        justify_content: d.justify_content,
        align_items: d.align_items,
        align_self: d.align_self,
        align_content: d.align_content,
        gap: taffy::Size {
            width: lp(d.gap[1], em),
            height: lp(d.gap[0], em),
        },
        ..taffy::Style::DEFAULT
    };
    Ok(Style {
        layout,
        background: d.background,
        radius: d.radius.px(em, 0.0),
        opacity: d.opacity,
        color: d.color.unwrap_or(inherited_color),
        font,
    })
}

#[cfg(test)]
mod tests;
