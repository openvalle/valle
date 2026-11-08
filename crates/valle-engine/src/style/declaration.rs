//! Declared property values and their assignment.

use std::borrow::Cow;
use std::rc::Rc;

use taffy::{
    AlignContent, AlignItems, AlignmentSafety, BoxSizing, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, Line, Overflow, Position,
};

use super::color::Rgba;
use super::computed::{BLENDS, Blend, BorderStyle, Cap, Fit, Join, PAINT_ORDER, Paint, PaintPart};
use super::font::{
    Collapse, DecorationStyle, Family, Feature, FontStyle, Leading, OverflowWrap, TextAlign,
    TextTransform, VARIANTS, Variation, Weight, WordBreak, families, features, font_style, leading,
    stretch, variants, variations, weight,
};
use super::property::{BorderPart, Field, NO_FLOW, NO_SCROLL, Sides, USE_FLEX};
use super::units::{
    Accept, BLUR, GAP, INDENT, INSET, Len, MAX_SIZE, OFFSET, PADDING, RADIUS, SIZE, SPACING,
    THICKNESS, UNDERLINE_OFFSET, WIDTH, length, length_text, lengths, svg_length,
};
use super::{background, color, css, filter, grid, math, offset, position, shape, transform};

use crate::template::V;

const AUTO: Len = Len::Auto;

pub(super) const ZERO: Len = Len::Px(0.0);

pub(super) const CENTER: position::Offset<Len> = position::Offset {
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
pub(super) const ROWS: usize = 0;

pub(super) const COLUMNS: usize = 1;

/// The largest length Chrome lays out, px: its `LayoutUnit`'s.
pub(super) const LARGEST: f64 = 33_554_431.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shown {
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

/// A node's declared values: initial values where it declares nothing, and for the inherited
/// properties `None`, which takes the parent's.
#[derive(Clone, Debug)]
pub(super) struct Declared {
    pub(super) display: Shown,
    pub(super) position: Position,
    /// Top, right, bottom, left, as the sides below.
    pub(super) inset: [Len; 4],
    pub(super) size: [Len; 2],
    pub(super) min_size: [Len; 2],
    /// `none` is `Auto`.
    pub(super) max_size: [Len; 2],
    pub(super) box_sizing: BoxSizing,
    /// Width over height; `None` is `auto`.
    pub(super) aspect_ratio: Option<f64>,
    /// x, y.
    pub(super) overflow: [Overflow; 2],
    pub(super) margin: [Len; 4],
    pub(super) padding: [Len; 4],
    pub(super) flex_direction: FlexDirection,
    pub(super) flex_wrap: FlexWrap,
    pub(super) flex_grow: f64,
    pub(super) flex_shrink: f64,
    pub(super) flex_basis: Len,
    pub(super) order: i32,
    pub(super) z_index: Option<i32>,
    pub(super) justify_content: AlignContent,
    pub(super) align_items: AlignItems,
    pub(super) align_self: Option<AlignItems>,
    pub(super) align_content: AlignContent,
    /// Row, column; `normal` is `Auto`, 0.
    pub(super) gap: [Len; 2],
    /// Rows, columns, as the grid fields below.
    pub(super) grid_template: [grid::Tracks; 2],
    pub(super) grid_areas: Option<grid::Areas>,
    pub(super) grid_auto: [Vec<grid::Track>; 2],
    pub(super) grid_auto_flow: GridAutoFlow,
    pub(super) grid_lines: [Line<GridPlacement<String>>; 2],
    pub(super) justify_items: AlignItems,
    pub(super) justify_self: Option<AlignItems>,
    /// `None` is `currentcolor`.
    pub(super) background: Option<Rgba>,
    pub(super) layers: background::Declared,
    /// Horizontal and vertical, from the top left clockwise.
    pub(super) radii: [[Len; 2]; 4],
    /// Top, right, bottom and left.
    pub(super) border_width: [Len; 4],
    pub(super) border_style: [BorderStyle; 4],
    /// `None` is `currentcolor`.
    pub(super) border_color: [Option<Rgba>; 4],
    pub(super) outline_width: Len,
    pub(super) outline_style: BorderStyle,
    pub(super) outline_color: Option<Rgba>,
    pub(super) outline_offset: Len,
    pub(super) shadows: Vec<Shadowed>,
    pub(super) opacity: f64,
    pub(super) translate: Option<transform::Function<Len>>,
    pub(super) rotate: Option<transform::Function<Len>>,
    pub(super) scale: Option<transform::Function<Len>>,
    pub(super) transform: Vec<transform::Function<Len>>,
    pub(super) transform_origin: [position::Offset<Len>; 2],
    pub(super) transform_origin_z: Len,
    pub(super) offset_path: Option<(offset::Route<Len>, background::Area)>,
    pub(super) offset_distance: Len,
    pub(super) offset_rotate: (bool, f64),
    pub(super) offset_anchor: Option<[position::Offset<Len>; 2]>,
    pub(super) offset_position: offset::Start<Len>,
    pub(super) perspective: Option<Len>,
    pub(super) perspective_origin: [position::Offset<Len>; 2],
    pub(super) object_fit: Fit,
    pub(super) object_position: [position::Offset<Len>; 2],
    pub(super) backface_visible: bool,
    pub(super) preserve_3d: bool,
    pub(super) blend: Blend,
    pub(super) isolate: bool,
    pub(super) filter: Vec<filter::Function<Len, Option<Rgba>>>,
    /// `motionBlur` as the frontend gives it, if declared: the trail in the box's own px, or
    /// `None` for `auto`.
    pub(super) motion_blur: Option<Option<[f64; 2]>>,
    pub(super) backdrop: Vec<filter::Function<Len, Option<Rgba>>>,
    pub(super) clip: Option<shape::Clip<Len>>,
    pub(super) masks: background::Declared,
    /// `text-decoration-*`: its lines (under, over, through), its style, its color (`None` is
    /// `currentcolor`) and its thickness.
    pub(super) decoration_lines: [bool; 3],
    pub(super) decoration_style: DecorationStyle,
    pub(super) decoration_color: Option<Rgba>,
    pub(super) decoration_thickness: Len,
    pub(super) ellipsis: bool,
    /// `-webkit-box-orient: vertical` (or `block-axis`), and `-webkit-line-clamp`.
    pub(super) box_vertical: bool,
    pub(super) line_clamp: Option<u32>,
    /// Inherited.
    pub(super) visible: Option<bool>,
    pub(super) pixelated: Option<bool>,
    pub(super) svg_fill: Option<Paint>,
    pub(super) fill_opacity: Option<f64>,
    pub(super) even_odd: Option<bool>,
    pub(super) svg_stroke: Option<Paint>,
    pub(super) stroke_opacity: Option<f64>,
    pub(super) stroke_width: Option<Len>,
    pub(super) cap: Option<Cap>,
    pub(super) join: Option<Join>,
    pub(super) miter_limit: Option<f64>,
    pub(super) dashes: Option<Rc<[Len]>>,
    pub(super) dash_offset: Option<Len>,
    pub(super) stop_color: Option<Paint>,
    pub(super) stop_opacity: Option<f64>,
    pub(super) clip_ref: Option<Rc<str>>,
    pub(super) mask_ref: Option<Rc<str>>,
    pub(super) clip_even_odd: Option<bool>,
    pub(super) mask_alpha: Option<bool>,
    /// Each declared or not, `none` or an id.
    pub(super) markers: [Option<Option<Rc<str>>>; 3],
    pub(super) paint_order: Option<[PaintPart; 3]>,
    pub(super) color: Option<Rgba>,
    pub(super) font_size: Option<Len>,
    pub(super) font_weight: Option<Weight>,
    pub(super) stretch: Option<f64>,
    pub(super) font_style: Option<FontStyle>,
    pub(super) font_family: Option<Rc<[Family]>>,
    pub(super) variants: [Option<Rc<[Feature]>>; VARIANTS],
    pub(super) features: Option<Rc<[Feature]>>,
    pub(super) variations: Option<Rc<[Variation]>>,
    pub(super) line_height: Option<Leading>,
    pub(super) text_align: Option<TextAlign>,
    /// `normal` is `Auto`, 0.
    pub(super) text_indent: Option<Len>,
    pub(super) letter_spacing: Option<Len>,
    pub(super) word_spacing: Option<Len>,
    pub(super) collapse: Option<Collapse>,
    pub(super) wrap: Option<bool>,
    pub(super) balance: Option<bool>,
    pub(super) word_break: Option<WordBreak>,
    pub(super) overflow_wrap: Option<OverflowWrap>,
    pub(super) text_transform: Option<TextTransform>,
    /// `Some(None)` is `currentcolor`.
    pub(super) fill: Option<Option<Rgba>>,
    pub(super) stroke: Option<Len>,
    pub(super) stroke_color: Option<Option<Rgba>>,
    pub(super) text_shadows: Option<Vec<Shadowed>>,
    pub(super) skip_ink: Option<bool>,
    pub(super) underline_offset: Option<Len>,
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
            background: Some(color::TRANSPARENT),
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
            offset_path: None,
            offset_distance: ZERO,
            offset_rotate: (true, 0.0),
            offset_anchor: None,
            offset_position: offset::Start::Normal,
            perspective: None,
            perspective_origin: [CENTER; 2],
            object_fit: Fit::Fill,
            object_position: [CENTER; 2],
            backface_visible: true,
            preserve_3d: false,
            blend: Blend::Normal,
            isolate: false,
            filter: Vec::new(),
            motion_blur: None,
            backdrop: Vec::new(),
            clip: None,
            masks: background::Declared::default(),
            decoration_lines: [false; 3],
            decoration_style: DecorationStyle::Solid,
            decoration_color: None,
            decoration_thickness: AUTO,
            ellipsis: false,
            box_vertical: false,
            line_clamp: None,
            visible: None,
            pixelated: None,
            svg_fill: None,
            fill_opacity: None,
            even_odd: None,
            svg_stroke: None,
            stroke_opacity: None,
            stroke_width: None,
            cap: None,
            join: None,
            miter_limit: None,
            dashes: None,
            dash_offset: None,
            stop_color: None,
            stop_opacity: None,
            clip_ref: None,
            mask_ref: None,
            clip_even_odd: None,
            mask_alpha: None,
            markers: [None, None, None],
            paint_order: None,
            color: None,
            font_size: None,
            font_weight: None,
            stretch: None,
            font_style: None,
            font_family: None,
            variants: Default::default(),
            features: None,
            variations: None,
            line_height: None,
            text_align: None,
            text_indent: None,
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

/// A declared box shadow; its color `None` is `currentcolor`.
#[derive(Clone, Debug)]
pub(super) struct Shadowed {
    pub(super) offset: [Len; 2],
    pub(super) blur: Len,
    pub(super) spread: Len,
    pub(super) color: Option<Rgba>,
    pub(super) inset: bool,
}

/// Taffy's minimum and maximum sizes are lengths only.
pub(super) const MIN_SIZE: Accept = Accept {
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

/// A value as written in a type error.
pub(super) fn shown(v: &V) -> String {
    match v {
        V::Num(x) => x.to_string(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => format!("{s:?}"),
    }
}

/// A presentation attribute's value as style takes it: a number written as a string is a number,
/// so that `width="100"` is 100 px.
pub(super) fn presented(v: &V, attribute: bool) -> Cow<'_, V> {
    match v {
        V::Str(s) if attribute => match s.trim().parse::<f64>() {
            Ok(x) if x.is_finite() => Cow::Owned(V::Num(x)),
            _ => Cow::Borrowed(v),
        },
        _ => Cow::Borrowed(v),
    }
}

/// What fills or strokes an SVG shape: `none`, `currentcolor`, a color, or `url(#id)`, its id
/// as written.
pub(super) fn paint(v: &V) -> Result<Paint, String> {
    let s = text(v)?;
    if let Some(id) = reference(&v.text()) {
        return id.map(Paint::Url);
    }
    Ok(match s.as_str() {
        "none" => Paint::None,
        "currentcolor" => Paint::CurrentColor,
        s => Paint::Color(color::parse(s)?),
    })
}

/// The id `url(#id)` names, as written, if `s` is a `url()`.
fn reference(s: &str) -> Option<Result<Rc<str>, String>> {
    let s = s.trim();
    if !s.get(..4)?.eq_ignore_ascii_case("url(") {
        return None;
    }
    let Some(inner) = s[4..].strip_suffix(')') else {
        return Some(Err(format!("{s}: what follows url() is not supported yet")));
    };
    let quoted = inner.trim().trim_matches(|c| c == '"' || c == '\'');
    Some(match quoted.strip_prefix('#') {
        Some(id) if !id.is_empty() => Ok(id.into()),
        _ => Err(format!("{s}: url() names an SVG element, as url(#id)")),
    })
}

/// `stroke-dasharray`: `none`, or SVG lengths, apart by commas or spaces.
pub(super) fn dashes(v: &V) -> Result<Rc<[Len]>, String> {
    if let V::Str(_) = v
        && text(v)? == "none"
    {
        return Ok(Rc::new([]));
    }
    let list: Vec<Len> = match v {
        V::Str(s) => (css::commas(s).into_iter())
            .flat_map(css::words)
            .map(|w| svg_length(&V::Str(w.into()), false))
            .collect::<Result<_, _>>()?,
        _ => vec![svg_length(v, false)?],
    };
    if list.is_empty() {
        return Err("there are no dashes: write none".into());
    }
    Ok(list.into())
}

/// An opacity: a number or a percentage, clamped to [0, 1].
pub(super) fn alpha(v: &V) -> Result<f64, String> {
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
    Ok(o.clamp(0.0, 1.0))
}

/// CSS text, trimmed and lowercased: keywords, units and functions are ASCII case-insensitive.
pub(super) fn text(v: &V) -> Result<String, String> {
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

/// The values of a shorthand, `what` they are: one to `most`, each a word or the one number.
pub(super) fn values<T>(
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

/// Which of one to four values each side of a box takes, top, right, bottom and left, as CSS
/// shorthands spread them.
pub(super) const SPREAD: [[usize; 4]; 4] = [[0, 0, 0, 0], [0, 1, 0, 1], [0, 1, 2, 1], [0, 1, 2, 3]];

/// The four sides of a box from one to four values.
pub(super) fn sides<T: Clone>(
    v: &V,
    what: &str,
    one: impl Fn(&V) -> Result<T, String>,
) -> Result<[T; 4], String> {
    let ts = values(v, 4, what, one)?;
    Ok(SPREAD[ts.len() - 1].map(|i| ts[i].clone()))
}

/// The sides `which` names, of top, right, bottom and left; a logical pair's start, then its end.
pub(super) fn named(which: Sides) -> &'static [usize] {
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
pub(super) fn radii(v: &V) -> Result<[[Len; 2]; 4], String> {
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

pub(super) fn radius(v: &V) -> Result<Len, String> {
    length(v, RADIUS)
}

/// A corner's radius: horizontal, then vertical if it differs.
pub(super) fn corner(v: &V) -> Result<[Len; 2], String> {
    Ok(match &lengths(v, RADIUS, 2)?[..] {
        [x, y, ..] => [x, y].map(Len::clone),
        [both] => [both; 2].map(Len::clone),
        [] => [ZERO; 2],
    })
}

/// A border or outline shorthand's line: a width, a style (as `styled` takes it) and a color in
/// any order, each at most once, and the initial ones for those left out.
pub(super) fn line(
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
pub(super) fn border_width(v: &V) -> Result<Len, String> {
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

/// `border-style`: any, with `none` and `hidden` the same without tables.
pub(super) fn border_style(v: &V) -> Result<BorderStyle, String> {
    let s = text(v)?;
    match s.as_str() {
        "none" | "hidden" => Ok(BorderStyle::None),
        "solid" => Ok(BorderStyle::Solid),
        "double" => Ok(BorderStyle::Double),
        "groove" => Ok(BorderStyle::Groove),
        "ridge" => Ok(BorderStyle::Ridge),
        "inset" => Ok(BorderStyle::Inset),
        "outset" => Ok(BorderStyle::Outset),
        "dotted" => Ok(BorderStyle::Dotted),
        "dashed" => Ok(BorderStyle::Dashed),
        s => Err(format!("{s} is not one of {}", LINE_STYLES.join(", "))),
    }
}

/// `outline-style`: those of `border-style` but `hidden`, and not `auto`.
pub(super) fn outline_style(v: &V) -> Result<BorderStyle, String> {
    match text(v)?.as_str() {
        "auto" => Err("auto is each browser's own focus ring; use solid".into()),
        "hidden" => Err("hidden is not an outline style; use none".into()),
        _ => border_style(v),
    }
}

/// `box-shadow`: `none`, or shadows separated by commas, each two offsets, a blur and a spread
/// (the last two may be left out), a color and `inset` in any order around them. A `text-shadow`
/// has no spread and no `inset`.
pub(super) fn shadows(v: &V, text_shadow: bool) -> Result<Vec<Shadowed>, String> {
    let s = text(v)?;
    match s.as_str() {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a shadow; use none".into()),
        _ => (css::commas(&s).into_iter())
            .map(|s| shadow(s, text_shadow))
            .collect(),
    }
}

pub(super) fn shadow(s: &str, text_shadow: bool) -> Result<Shadowed, String> {
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

/// A color, or `currentcolor`, which is `None`.
fn current_color(v: &V) -> Result<Option<Rgba>, String> {
    match text(v)?.as_str() {
        "currentcolor" => Ok(None),
        s => color::parse(s).map(Some),
    }
}

pub(super) fn number(v: &V) -> Result<f64, String> {
    match v {
        V::Num(x) if x.is_finite() => Ok(*x),
        V::Num(x) => Err(format!("{x} is not a finite number")),
        V::Bool(_) => Err(format!("{} is not a number", shown(v))),
        V::Str(_) => {
            let s = text(v)?;
            if math::is_math(&s) {
                return math::number(&s);
            }
            css::num(&s)
                .filter(|n| n.unit.is_empty())
                .map(|n| n.value)
                .ok_or_else(|| format!("{s} is not a number"))
        }
    }
}

pub(super) fn factor(v: &V) -> Result<f64, String> {
    let x = number(v)?;
    if x < 0.0 {
        return Err(format!("{x} is negative"));
    }
    Ok(x)
}

pub(super) fn integer(v: &V) -> Result<i32, String> {
    let mut x = number(v)?;
    // CSS rounds a math function to the nearest integer, halves up.
    if matches!(v, V::Str(s) if math::is_math(s.trim())) {
        x = (x + 0.5).floor();
    }
    if x.fract() != 0.0 || x.abs() > f64::from(i32::MAX) {
        return Err(format!("{x} is not an integer"));
    }
    Ok(x as i32)
}

/// `aspect-ratio`: `auto`, a ratio `w / h` or `w`, or both, which for a box without a natural
/// ratio is the ratio. A ratio with a 0 in it is degenerate, and `auto`.
pub(super) fn ratio(v: &V) -> Result<Option<f64>, String> {
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

pub(super) fn keyword<T: Copy>(v: &V, table: &[(&str, T)]) -> Result<T, String> {
    let s = text(v)?;
    match table.iter().find(|(k, _)| *k == s) {
        Some((_, t)) => Ok(*t),
        None => {
            let names: Vec<&str> = table.iter().map(|(k, _)| *k).collect();
            Err(format!("{s} is not one of {}", names.join(", ")))
        }
    }
}

pub(super) fn display(v: &V) -> Result<Shown, String> {
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

pub(super) fn position(v: &V) -> Result<Position, String> {
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
pub(super) fn overflow(v: &V) -> Result<Overflow, String> {
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
        _ => Some(items_align(v)?),
    })
}

/// The `safe` or `unsafe` before an alignment, and the alignment; only those that place a box
/// at one point take it.
fn overflow_position(v: &V) -> Result<(AlignmentSafety, V), String> {
    let s = text(v)?;
    let words = css::words(&s);
    let safety = match words[..] {
        ["safe", _] => AlignmentSafety::Safe,
        ["unsafe", _] => AlignmentSafety::Unsafe,
        _ => return Ok((AlignmentSafety::Default, v.clone())),
    };
    match POSITIONAL.contains(&words[1]) {
        true => Ok((safety, V::Str(words[1].into()))),
        false => Err(format!("{} takes no {}", words[1], words[0])),
    }
}

const POSITIONAL: [&str; 7] = [
    "center",
    "start",
    "end",
    "flex-start",
    "flex-end",
    "self-start",
    "self-end",
];

fn items_align(v: &V) -> Result<AlignItems, String> {
    let (safety, v) = overflow_position(v)?;
    Ok(keyword(&v, &SELF_ALIGN)?.with_safety(safety))
}

fn content_align(v: &V) -> Result<AlignContent, String> {
    let (safety, v) = overflow_position(v)?;
    Ok(keyword(&v, &CONTENT_ALIGN)?.with_safety(safety))
}

/// A shorthand of two values, such as `place-items`' alignment and justification, where one value
/// is both.
pub(super) fn pair<T: Clone>(v: &V, one: fn(&V) -> Result<T, String>) -> Result<(T, T), String> {
    let s = text(v)?;
    let word = |w: &str| one(&V::Str(w.into()));
    // `safe` and `unsafe` go with the word after them.
    let mut values: Vec<String> = Vec::new();
    for w in css::words(&s) {
        match values.last_mut() {
            Some(last) if last == "safe" || last == "unsafe" => *last = format!("{last} {w}"),
            _ => values.push(w.into()),
        }
    }
    match &values[..] {
        [both] => word(both).map(|t| (t.clone(), t)),
        [align, justify] => Ok((word(align)?, word(justify)?)),
        _ => Err(format!("{} takes one or two values", shown(v))),
    }
}

/// `flex`: grow, shrink and basis. A lone factor makes the basis `0%`, as browsers compute it.
pub(super) fn flex(v: &V) -> Result<(f64, f64, Len), String> {
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

fn color_of(v: &V) -> Result<Rgba, String> {
    color::parse(&text(v)?)
}

fn is_current_color(v: &V) -> bool {
    matches!(v, V::Str(s) if s.trim().eq_ignore_ascii_case("currentcolor"))
}

/// Whether `v` is `inherit`, of a property that inherits: as if it were not declared, over what
/// an earlier declaration set.
pub(super) fn inherits(v: &V) -> bool {
    matches!(v, V::Str(s) if s.trim().eq_ignore_ascii_case("inherit"))
}

pub(super) fn set(d: &mut Declared, field: Field, v: &V) -> Result<(), String> {
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
        F::JustifyContent => d.justify_content = content_align(v)?,
        F::AlignItems => d.align_items = items_align(v)?,
        F::AlignSelf => d.align_self = self_align(v)?,
        F::AlignContent => d.align_content = content_align(v)?,
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
        F::JustifyItems => d.justify_items = items_align(v)?,
        F::JustifySelf => d.justify_self = self_align(v)?,
        F::PlaceItems => (d.align_items, d.justify_items) = pair(v, items_align)?,
        F::PlaceSelf => (d.align_self, d.justify_self) = pair(v, self_align)?,
        F::PlaceContent => (d.align_content, d.justify_content) = pair(v, content_align)?,
        F::Background => {
            // The shorthand leaves the blend modes as they are.
            let blends = std::mem::take(&mut d.layers.blends);
            let (layers, color) = background::background(&text(v)?)?;
            (d.layers, d.background) = (layers, Some(color));
            d.layers.blends = blends;
        }
        F::BackgroundColor => d.background = current_color(v)?,
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
        F::OffsetPath => match v {
            V::Str(s) => d.offset_path = offset::path(s)?,
            other => return Err(format!("{} is not a string", shown(other))),
        },
        F::OffsetDistance => d.offset_distance = offset::distance(&px_text(v)?)?,
        F::OffsetRotate => d.offset_rotate = offset::rotate(&text(v)?)?,
        F::OffsetAnchor => d.offset_anchor = offset::anchor(&px_text(v)?)?,
        F::OffsetPosition => d.offset_position = offset::start(&px_text(v)?)?,
        F::ObjectFit => {
            d.object_fit = keyword(
                v,
                &[
                    ("fill", Fit::Fill),
                    ("contain", Fit::Contain),
                    ("cover", Fit::Cover),
                    ("none", Fit::None),
                    ("scale-down", Fit::ScaleDown),
                ],
            )?;
        }
        F::ObjectPosition => d.object_position = position::object(&px_text(v)?)?,
        F::ImageRendering => {
            d.pixelated = Some(keyword(v, &[("auto", false), ("pixelated", true)])?)
        }
        F::Opacity => d.opacity = alpha(v)?,
        F::MixBlendMode => d.blend = keyword(v, &BLENDS)?,
        F::Isolation => d.isolate = keyword(v, &[("auto", false), ("isolate", true)])?,
        F::Filter => d.filter = filter::filter(&text(v)?)?,
        F::MotionBlur => {
            let trail = text(v)?;
            let xy: Vec<f64> = trail.split(' ').filter_map(|n| n.parse().ok()).collect();
            d.motion_blur = Some(match xy[..] {
                _ if trail == "auto" => None,
                [x, y] if x.is_finite() && y.is_finite() => Some([x, y]),
                _ => return Err(format!("motionBlur trail {trail:?}")),
            });
        }
        F::BackdropFilter => d.backdrop = filter::filter(&text(v)?)?,
        F::ClipPath => match v {
            V::Str(s) => match reference(s) {
                Some(id) => (d.clip, d.clip_ref) = (None, Some(id?)),
                None => (d.clip, d.clip_ref) = (shape::clip(s)?, None),
            },
            other => return Err(format!("{} is not a string", shown(other))),
        },
        F::MaskImage => d.masks.images = background::images(&text(v)?)?,
        F::MaskSize => d.masks.sizes = background::sizes(&px_text(v)?)?,
        F::MaskPosition => d.masks.positions = background::positions(&px_text(v)?, false)?,
        F::MaskRepeat => d.masks.repeats = background::repeats(&text(v)?)?,
        F::MaskMode => d.masks.modes = background::modes(&text(v)?)?,
        F::MaskComposite => d.masks.composites = background::composites(&text(v)?)?,
        F::MaskClip => d.masks.clips = background::areas(&text(v)?, false)?,
        F::MaskOrigin => d.masks.origins = background::areas(&text(v)?, false)?,
        F::Visibility => {
            // Collapse is hidden but in tables, which there are none of.
            let shown = [("visible", true), ("hidden", false), ("collapse", false)];
            d.visible = Some(keyword(v, &shown)?);
        }
        F::Fill if inherits(v) => d.svg_fill = None,
        F::Fill => d.svg_fill = Some(paint(v)?),
        F::FillOpacity => d.fill_opacity = Some(alpha(v)?),
        F::FillRule => d.even_odd = Some(keyword(v, &[("nonzero", false), ("evenodd", true)])?),
        F::PaintOrder => d.paint_order = paint_order(v)?,
        F::Stroke if inherits(v) => d.svg_stroke = None,
        F::Stroke => d.svg_stroke = Some(paint(v)?),
        F::StrokeOpacity => d.stroke_opacity = Some(alpha(v)?),
        F::StrokeWidth => d.stroke_width = Some(svg_length(v, false)?),
        F::StrokeLinecap => {
            let caps = [
                ("butt", Cap::Butt),
                ("round", Cap::Round),
                ("square", Cap::Square),
            ];
            d.cap = Some(keyword(v, &caps)?);
        }
        F::StrokeLinejoin => {
            let joins = [
                ("miter", Join::Miter),
                ("round", Join::Round),
                ("bevel", Join::Bevel),
            ];
            d.join = Some(keyword(v, &joins)?);
        }
        F::StrokeMiterlimit => match number(v)? {
            x if x < 1.0 => return Err(format!("{x} is below 1")),
            x => d.miter_limit = Some(x),
        },
        F::StrokeDasharray => d.dashes = Some(dashes(v)?),
        F::StrokeDashoffset => d.dash_offset = Some(svg_length(v, true)?),
        F::StopColor => match paint(v)? {
            p @ (Paint::Color(_) | Paint::CurrentColor) => d.stop_color = Some(p),
            _ => return Err("a stop's color is a color or currentColor".into()),
        },
        F::StopOpacity => d.stop_opacity = Some(alpha(v)?),
        F::ClipRule => {
            d.clip_even_odd = Some(keyword(v, &[("nonzero", false), ("evenodd", true)])?);
        }
        F::Mask => {
            let s = text(v)?;
            d.mask_ref = match reference(&v.text()) {
                Some(id) => Some(id?),
                None if s == "none" => None,
                None => {
                    return Err(format!(
                        "{s}: a mask is url(#id) of a <mask> in an <svg>, or none; a box takes maskImage"
                    ));
                }
            };
        }
        F::MaskType => {
            d.mask_alpha = Some(keyword(v, &[("luminance", false), ("alpha", true)])?);
        }
        F::Marker(which) => {
            let s = text(v)?;
            let marker = match reference(&v.text()) {
                Some(id) => Some(id?),
                None if s == "none" => None,
                None => return Err(format!("{s}: a marker is url(#id) of a <marker>, or none")),
            };
            match which {
                Some(k) => d.markers[k] = Some(marker),
                None => d.markers = [(); 3].map(|_| Some(marker.clone())),
            }
        }
        // `currentcolor` is the color the element would otherwise inherit.
        F::Color if inherits(v) || is_current_color(v) => d.color = None,
        F::Color => d.color = Some(color_of(v)?),
        F::FontSize => d.font_size = Some(length(v, PADDING)?),
        F::FontWeight => d.font_weight = Some(weight(v)?),
        F::FontStretch => d.stretch = Some(stretch(v)?),
        F::FontStyle => d.font_style = Some(font_style(v)?),
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
                    ("justify", TextAlign::Justify),
                ],
            )?)
        }
        F::TextIndent => d.text_indent = Some(length(v, INDENT)?),
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
                d.decoration_style,
                d.decoration_color,
                d.decoration_thickness,
            ) = decoration(v)?
        }
        F::TextDecorationLine => d.decoration_lines = decoration_lines(&text(v)?)?,
        F::TextDecorationColor => d.decoration_color = current_color(v)?,
        F::TextDecorationStyle => d.decoration_style = decoration_style(&text(v)?)?,
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
                V::Str(_) if matches!(&*text(v)?, "none" | "unset") => None,
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
pub(super) fn decoration(v: &V) -> Result<([bool; 3], DecorationStyle, Option<Rgba>, Len), String> {
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
    let style = style.unwrap_or(DecorationStyle::Solid);
    Ok((lines, style, color.flatten(), thickness.unwrap_or(AUTO)))
}

/// `text-decoration-line`: `none`, or any of `underline`, `overline` and `line-through`.
pub(super) fn decoration_lines(s: &str) -> Result<[bool; 3], String> {
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

/// `text-decoration-style`.
pub(super) fn decoration_style(s: &str) -> Result<DecorationStyle, String> {
    match s {
        "solid" => Ok(DecorationStyle::Solid),
        "double" => Ok(DecorationStyle::Double),
        "dotted" => Ok(DecorationStyle::Dotted),
        "dashed" => Ok(DecorationStyle::Dashed),
        "wavy" => Ok(DecorationStyle::Wavy),
        s => Err(format!(
            "{s:?} is not one of {}",
            DECORATION_STYLES.join(", ")
        )),
    }
}

/// Operations not listed in `paint-order` follow in their normal order.
fn paint_order(v: &V) -> Result<Option<[PaintPart; 3]>, String> {
    let s = text(v)?;
    match s.as_str() {
        "inherit" | "unset" => return Ok(None),
        "normal" | "initial" => return Ok(Some(PAINT_ORDER)),
        _ => {}
    }
    let words = css::words(&s);
    if words.is_empty() || words.len() > 3 {
        return Err(format!("{s:?} is not a paint order"));
    }
    let mut order = PAINT_ORDER;
    let mut n = 0;
    for word in words {
        let part = match word {
            "fill" => PaintPart::Fill,
            "stroke" => PaintPart::Stroke,
            "markers" => PaintPart::Markers,
            _ => return Err(format!("{word} is not fill, stroke or markers")),
        };
        if order[..n].contains(&part) {
            return Err(format!("{s} repeats {word}"));
        }
        order[n] = part;
        n += 1;
    }
    for part in PAINT_ORDER {
        if !order[..n].contains(&part) {
            order[n] = part;
            n += 1;
        }
    }
    Ok(Some(order))
}

/// `-webkit-text-stroke`: a width and a color in either order, each at most once, and the
/// initial ones for those left out.
pub(super) fn stroke(v: &V) -> Result<(Len, Option<Rgba>), String> {
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
pub(super) fn stroke_width(v: &V) -> Result<Len, String> {
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

#[cfg(test)]
mod tests {

    use taffy::{
        AlignContent, AlignItems, BoxSizing, Dimension, Display, GridPlacement,
        LengthPercentageAuto, Line,
    };

    use crate::style::math::Math;

    use crate::template::Prop;

    use super::*;
    use crate::style::*;
    use crate::testing::styles::{auto_px, error, num, one, px, text};

    /// The images of a style's background layers.
    fn images(style: &[(&str, Prop)]) -> Vec<gradient::Image> {
        one(style).layers.into_iter().map(|l| l.image).collect()
    }

    #[test]
    fn logical_sides_are_the_physical_ones_in_declaration_order() {
        let l = one(&[
            ("padding", num(1.0)),
            ("paddingInline", text("2px 3px")),
            ("paddingTop", num(4.0)),
            ("paddingBlock", num(5.0)),
            ("marginInlineEnd", num(6.0)),
            ("marginRight", num(7.0)),
            ("insetBlockEnd", text("10%")),
        ])
        .layout;
        let padding = taffy::Rect {
            top: px(5.0),
            right: px(3.0),
            bottom: px(5.0),
            left: px(2.0),
        };
        assert_eq!(l.padding, padding);
        assert_eq!(l.margin.right, auto_px(7.0));
        assert_eq!(l.inset.bottom, LengthPercentageAuto::percent(0.1));
        assert!(
            error(&[("marginBlock", text("1px 2px 3px"))]).ends_with("takes one or two lengths")
        );
    }

    #[test]
    fn box_sizing_aspect_ratio_and_order() {
        let initial = one(&[]);
        assert_eq!(initial.layout.box_sizing, BoxSizing::BorderBox);
        assert_eq!((initial.layout.aspect_ratio, initial.order), (None, 0));
        let content = one(&[("boxSizing", text(" Content-Box "))]);
        assert_eq!(content.layout.box_sizing, BoxSizing::ContentBox);
        let ratio = |p: Prop| one(&[("aspectRatio", p)]).layout.aspect_ratio;
        assert_eq!(ratio(num(2.0)), Some(2.0));
        assert_eq!(ratio(text("16/10")), Some(1.6));
        assert_eq!(ratio(text("auto 3 / 2")), Some(1.5));
        assert_eq!(ratio(text("1 auto")), Some(1.0));
        for none in [text("auto"), text("0"), text("2 / 0"), num(0.0)] {
            assert_eq!(ratio(none), None);
        }
        for bad in ["16:9", "1 / -2", "1 2", "auto auto", "1px", ""] {
            let e = error(&[("aspectRatio", text(bad))]);
            assert!(
                e.starts_with("s.tsx:1:1: style aspectRatio: "),
                "{bad}: {e}"
            );
        }
        assert!(error(&[("aspectRatio", num(-1.0))]).ends_with("-1 is negative"));
        assert_eq!(one(&[("order", text("-3"))]).order, -3);
        assert!(error(&[("order", num(1.5))]).ends_with("1.5 is not an integer"));
    }

    /// Sets of line names.
    fn names(ns: &[&[&str]]) -> Vec<Vec<String>> {
        ns.iter()
            .map(|n| n.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn z_index_is_auto_or_an_integer() {
        assert_eq!(one(&[]).z_index, None);
        assert_eq!(one(&[("zIndex", num(-3.0))]).z_index, Some(-3));
        assert_eq!(one(&[("zIndex", text(" 2 "))]).z_index, Some(2));
        assert_eq!(one(&[("zIndex", text("AUTO"))]).z_index, None);
        assert!(error(&[("zIndex", num(1.5))]).contains("1.5 is not an integer"));
    }

    #[test]
    fn overflow_hidden_on_one_axis_hides_both() {
        use taffy::Overflow::{Clip, Hidden, Visible};
        let overflow = |style: &[(&str, Prop)]| {
            let o = one(style).layout.overflow;
            [o.x, o.y]
        };
        assert_eq!(overflow(&[]), [Visible, Visible]);
        assert_eq!(overflow(&[("overflow", text("clip"))]), [Clip, Clip]);
        assert_eq!(overflow(&[("overflowX", text("clip"))]), [Clip, Visible]);
        assert_eq!(
            overflow(&[("overflowBlock", text("HIDDEN"))]),
            [Hidden, Hidden]
        );
        assert_eq!(
            overflow(&[("overflow", text("clip hidden"))]),
            [Hidden, Hidden]
        );
        assert!(error(&[("overflow", text("auto"))]).contains("the canvas does not scroll"));
    }

    #[test]
    fn grid_tracks_and_lines_go_to_taffy() {
        use taffy::{
            GridTemplateComponent as C, GridTemplateRepetition, MaxTrackSizingFunction as Max,
            MinMax, MinTrackSizingFunction as Min, RepetitionCount,
        };
        let s = one(&[
            ("display", text("grid")),
            ("fontSize", num(10.0)),
            (
                "gridTemplateColumns",
                text(
                    "[Start] 2em repeat(2, [a] minmax(10%, 1FR)) fit-content(calc(50% - 5px)) [End]",
                ),
            ),
            ("gridTemplateRows", text("repeat(auto-fill, 20px)")),
            ("gridAutoRows", text("min-content 3fr")),
            ("gridAutoFlow", text("column dense")),
            ("gridRow", text("span 2 / Main")),
            ("gridColumn", text("Main")),
            ("gridColumnEnd", num(-1.0)),
            ("justifyItems", text("center")),
            ("justifySelf", text("auto")),
        ]);
        let l = &s.layout;
        let both = |min, max| MinMax { min, max };
        assert_eq!(l.display, Display::Grid);
        assert_eq!(
            l.grid_template_columns,
            [
                C::Single(both(Min::length(20.0), Max::length(20.0))),
                C::Repeat(GridTemplateRepetition {
                    count: RepetitionCount::Count(2),
                    tracks: vec![both(Min::percent(0.1), Max::fr(1.0))],
                    line_names: names(&[&["a"], &[]]),
                }),
                C::Single(both(Min::auto(), Max::fit_content_calc(math::handle(0)))),
            ]
        );
        // Names are as written; a set for each line around the parts.
        assert_eq!(
            l.grid_template_column_names,
            names(&[&["Start"], &[], &[], &["End"]])
        );
        assert_eq!(s.calcs.len(), 1);
        assert_eq!(
            l.grid_template_rows,
            [C::Repeat(GridTemplateRepetition {
                count: RepetitionCount::AutoFill,
                tracks: vec![both(Min::length(20.0), Max::length(20.0))],
                line_names: names(&[&[], &[]]),
            })]
        );
        assert_eq!(
            l.grid_auto_rows,
            [
                both(Min::min_content(), Max::min_content()),
                both(Min::auto(), Max::fr(3.0)),
            ]
        );
        assert_eq!(l.grid_auto_flow, taffy::GridAutoFlow::ColumnDense);
        let main = || GridPlacement::NamedLine("Main".to_string(), 0);
        assert_eq!(l.grid_row.start, GridPlacement::Span(2));
        assert_eq!(l.grid_row.end, main());
        // A name alone ends where it starts, unless something sets the end.
        assert_eq!(l.grid_column.start, main());
        assert_eq!(l.grid_column.end, GridPlacement::Line((-1i16).into()));
        assert_eq!(l.justify_items, AlignItems::CENTER);
        assert_eq!(l.justify_self, None);

        let line = |v: &str| one(&[("gridRowStart", text(v))]).layout.grid_row.start;
        assert_eq!(line("2 a"), GridPlacement::NamedLine("a".into(), 2));
        assert_eq!(line("a -1"), GridPlacement::NamedLine("a".into(), -1));
        assert_eq!(line("span B 2"), GridPlacement::NamedSpan("B".into(), 2));
        assert_eq!(line("SPAN b"), GridPlacement::NamedSpan("b".into(), 1));
        assert_eq!(line("Auto"), GridPlacement::Auto);
        let flow = |v: &str| one(&[("gridAutoFlow", text(v))]).layout.grid_auto_flow;
        assert_eq!(flow("dense"), taffy::GridAutoFlow::RowDense);
        assert_eq!(flow("COLUMN"), taffy::GridAutoFlow::Column);
        let none = one(&[("gridTemplateColumns", text("none"))]);
        assert!(none.layout.grid_template_columns.is_empty());
        assert!(none.layout.grid_template_column_names.is_empty());
    }

    #[test]
    fn grid_areas_and_shorthands_go_to_taffy() {
        use taffy::{GridAutoFlow, GridTemplateArea};
        let area = |name: &str, rows: [u16; 2], columns: [u16; 2]| GridTemplateArea {
            name: name.into(),
            row_start: rows[0],
            row_end: rows[1],
            column_start: columns[0],
            column_end: columns[1],
        };
        let s = one(&[(
            "gridTemplateAreas",
            text("'Head Head' \"side .\" 'side main'"),
        )]);
        let a = s.layout.grid_template_areas.unwrap();
        assert_eq!(
            a.areas,
            [
                area("Head", [1, 2], [1, 3]),
                area("side", [2, 4], [1, 2]),
                area("main", [3, 4], [2, 3]),
            ]
        );
        assert_eq!((a.row_count, a.column_count), (3, 2));

        // Rows of names, a string and a size; columns after the `/`.
        let s = one(&[(
            "gridTemplate",
            text("[top] \"a a\" 20px [mid] \"b c\" / [l] 1fr 2fr"),
        )]);
        let l = &s.layout;
        assert_eq!(l.grid_template_rows.len(), 2);
        assert_eq!(l.grid_template_row_names, names(&[&["top"], &["mid"], &[]]));
        assert_eq!(l.grid_template_column_names, names(&[&["l"], &[], &[]]));
        assert_eq!(l.grid_template_areas.as_ref().unwrap().areas.len(), 3);

        // `grid` sets what items flow into, and resets the rest.
        let s = one(&[
            ("gridAutoRows", text("5px")),
            ("grid", text("auto-flow dense 10px / 1fr 1fr")),
        ]);
        assert_eq!(s.layout.grid_auto_flow, GridAutoFlow::RowDense);
        assert_eq!(s.layout.grid_auto_rows.len(), 1);
        assert_eq!(s.layout.grid_template_columns.len(), 2);
        assert!(s.layout.grid_template_rows.is_empty());
        let s = one(&[
            ("gridAutoRows", text("5px")),
            ("grid", text("20px / auto-flow")),
        ]);
        assert_eq!(s.layout.grid_auto_flow, GridAutoFlow::Column);
        assert!(s.layout.grid_auto_rows.is_empty());

        // Lines `grid-area` leaves out take the name of their start.
        let s = one(&[("gridArea", text("main"))]);
        let main = || GridPlacement::NamedLine("main".to_string(), 0);
        let line = |start, end| Line { start, end };
        assert_eq!(s.layout.grid_row, line(main(), main()));
        assert_eq!(s.layout.grid_column, line(main(), main()));
        let s = one(&[("gridArea", text("2 / a / span 2"))]);
        let a = || GridPlacement::NamedLine("a".to_string(), 0);
        let row = line(GridPlacement::Line(2i16.into()), GridPlacement::Span(2));
        assert_eq!(s.layout.grid_row, row);
        assert_eq!(s.layout.grid_column, line(a(), a()));

        // One value of a `place-` shorthand is both.
        let s = one(&[
            ("placeItems", text("center")),
            ("placeSelf", text("end start")),
            ("placeContent", text("space-between center")),
        ]);
        let l = &s.layout;
        assert_eq!(
            (l.align_items, l.justify_items),
            (AlignItems::CENTER, AlignItems::CENTER)
        );
        assert_eq!(
            (l.align_self, l.justify_self),
            (Some(AlignItems::END), Some(AlignItems::START))
        );
        assert_eq!(
            (l.align_content, l.justify_content),
            (AlignContent::SPACE_BETWEEN, AlignContent::CENTER)
        );
    }

    #[test]
    fn grid_values_out_of_css_are_errors() {
        let cases: &[(&str, &str, &str)] = &[
            ("gridTemplateColumns", "1fr 2", "2 needs a unit: 2px"),
            ("gridTemplateColumns", "", "\"\" has no tracks"),
            (
                "gridTemplateColumns",
                "repeat(auto-fill, 1fr)",
                "beside auto-fill or auto-fit, every track needs a length",
            ),
            (
                "gridTemplateColumns",
                "repeat(auto-fit, 10px) auto",
                "beside auto-fill or auto-fit",
            ),
            (
                "gridTemplateColumns",
                "repeat(auto-fill, 10px) repeat(auto-fit, 10px)",
                "only one repeat() may be auto-fill or auto-fit",
            ),
            ("gridTemplateColumns", "repeat(0, 10px)", "0 is not a count"),
            (
                "gridTemplateColumns",
                "repeat(2, repeat(2, 10px))",
                "repeat() does not nest",
            ),
            (
                "gridTemplateColumns",
                "minmax(1fr, 10px)",
                "the minimum is not flexible",
            ),
            (
                "gridTemplateColumns",
                "fit-content",
                "tracks take fit-content(limit)",
            ),
            (
                "gridTemplateColumns",
                "[a] [b] 10px",
                "put names of one line in one [ ]",
            ),
            (
                "gridTemplateColumns",
                "[span] 10px",
                "span is a keyword, not a line name",
            ),
            ("gridTemplateColumns", "[1a] 10px", "1a is not a line name"),
            ("gridTemplateColumns", "[a 10px", "a [ is not closed"),
            ("gridTemplateColumns", "-1fr", "-1fr is negative"),
            ("gridTemplateRows", "subgrid", "subgrid is not supported"),
            (
                "gridAutoRows",
                "repeat(2, 10px)",
                "implicit tracks take sizes only",
            ),
            ("gridAutoFlow", "row column", "is not row or column"),
            ("gridRow", "0", "there is no line 0"),
            ("gridRow", "1 / 2 / 3", "takes at most 2 lines"),
            ("gridRow", "span 0", "a span is at least 1"),
            ("gridRow", "span", "is not a grid line"),
            ("gridRowStart", "1.5", "1.5 is not a whole number"),
            ("gridColumnStart", "a b", "is not a grid line"),
            ("gridColumnEnd", "auto 2", "auto is a keyword"),
            ("justifyItems", "left", "left is not one of normal, stretch"),
            (
                "gridTemplateColumns",
                "'a' 10px",
                "strings of areas go in grid-template-areas",
            ),
            (
                "gridTemplateAreas",
                "'a b' 'a'",
                "every row needs as many cells as the others",
            ),
            (
                "gridTemplateAreas",
                "'a b' 'b a'",
                "area a is not a rectangle",
            ),
            ("gridTemplateAreas", "'a #'", "# is not in a name"),
            ("gridTemplateAreas", "a b", "each row is a string"),
            ("gridTemplateAreas", "'a", "a string is not closed"),
            (
                "gridTemplate",
                "'a' / repeat(2, 1fr)",
                "columns with areas take no repeat()",
            ),
            (
                "gridTemplate",
                "1fr",
                "takes rows / columns, or rows with areas",
            ),
            (
                "gridTemplate",
                "'a' [x] [y] [z] 'b'",
                "each row is names, a string of areas",
            ),
            (
                "grid",
                "10px auto-flow / 1fr",
                "auto-flow and dense come before the sizes",
            ),
            ("gridArea", "1 / 2 / 3 / 4 / 5", "takes at most 4 lines"),
            ("placeItems", "center end start", "takes one or two values"),
            ("placeContent", "baseline", "baseline is not one of"),
        ];
        for (key, value, message) in cases {
            let e = error(&[(*key, text(value))]);
            assert!(
                e.starts_with(&format!("s.tsx:1:1: style {key}: ")) && e.contains(message),
                "{key}: {value}: {e}"
            );
        }
    }

    #[test]
    fn the_flex_shorthand_follows_css() {
        let flex = |p: Prop| {
            let l = one(&[("flex", p)]).layout;
            (l.flex_grow, l.flex_shrink, l.flex_basis)
        };
        let zero = Dimension::percent(0.0);
        assert_eq!(flex(num(2.0)), (2.0, 1.0, zero));
        assert_eq!(flex(text("2 3")), (2.0, 3.0, zero));
        assert_eq!(flex(text("1 1 0")), (1.0, 1.0, Dimension::length(0.0)));
        assert_eq!(flex(text("10px")), (1.0, 1.0, Dimension::length(10.0)));
        assert_eq!(flex(text("2 30%")), (2.0, 1.0, Dimension::percent(0.3)));
        assert_eq!(flex(text("30% 2 0")), (2.0, 0.0, Dimension::percent(0.3)));
        assert_eq!(flex(text("auto")), (1.0, 1.0, Dimension::auto()));
        assert_eq!(flex(text("none")), (0.0, 0.0, Dimension::auto()));
        for bad in ["1 10px 2", "1 2 3 4", "-1", "1px 2px", ""] {
            assert!(
                error(&[("flex", text(bad))]).starts_with("s.tsx:1:1: style flex: "),
                "{bad}"
            );
        }
    }

    #[test]
    fn borders_take_lines_widths_styles_and_colors() {
        let c = |name| color::parse(name).unwrap();
        let border = |style: &[(&str, Prop)]| {
            let s = one(style);
            let b = s.layout.border;
            assert_eq!(
                [b.top, b.right, b.bottom, b.left],
                s.border.map(|w| px(w as f32))
            );
            (s.border, s.border_colors)
        };
        // No line until a style gives one; then `medium`, in `currentcolor`.
        assert_eq!(border(&[]), ([0.0; 4], [color::BLACK; 4]));
        assert_eq!(
            border(&[("color", text("blue")), ("border", text("solid"))]),
            ([3.0; 4], [c("blue"); 4])
        );
        assert_eq!(
            border(&[("border", text("red 2px solid"))]),
            ([2.0; 4], [c("red"); 4])
        );
        assert_eq!(
            border(&[
                ("borderStyle", text("solid none")),
                ("borderWidth", text("thin medium thick")),
                ("borderLeftStyle", text("solid")),
                ("borderLeftWidth", text("2.75px")),
                ("borderColor", text("red green")),
                ("borderBlockEndColor", text("currentcolor")),
            ]),
            (
                [1.0, 0.0, 5.0, 2.0],
                [c("red"), c("green"), color::BLACK, c("green")]
            )
        );
        // Logical sides, and widths between 0 and 1 px.
        assert_eq!(
            border(&[
                ("borderInline", text("0.25px solid blue")),
                ("borderBlockStyle", text("hidden solid")),
                ("borderBlockWidth", num(4.0)),
            ]),
            (
                [0.0, 1.0, 4.0, 1.0],
                [color::BLACK, c("blue"), color::BLACK, c("blue")]
            )
        );
        // Their styles, for paint.
        use BorderStyle::{Dashed, Dotted, Double, Groove, Inset, Outset, Ridge, Solid};
        let styles = |style: &[(&str, Prop)]| (one(style).border_styles, one(style).outline_style);
        assert_eq!(
            styles(&[
                ("borderStyle", text("double solid")),
                ("borderLeftStyle", text("hidden")),
                ("outline", text("double")),
            ]),
            ([Double, Solid, Double, BorderStyle::None], Double)
        );
        assert_eq!(
            styles(&[
                ("borderStyle", text("groove ridge inset outset")),
                ("outlineStyle", text("ridge")),
            ]),
            ([Groove, Ridge, Inset, Outset], Ridge)
        );
        assert_eq!(
            styles(&[
                ("borderStyle", text("dotted dashed")),
                ("outline", text("2px dashed"))
            ]),
            ([Dotted, Dashed, Dotted, Dashed], Dashed)
        );
    }

    #[test]
    fn outlines_take_a_line_and_an_offset() {
        let c = |name| color::parse(name).unwrap();
        let outline = |style: &[(&str, Prop)]| {
            let s = one(style);
            (s.outline, s.outline_color, s.outline_offset)
        };
        assert_eq!(outline(&[]), (0.0, color::BLACK, 0.0));
        assert_eq!(
            outline(&[("color", text("red")), ("outline", text("solid"))]),
            (3.0, c("red"), 0.0)
        );
        assert_eq!(
            outline(&[
                ("outline", text("thick blue solid")),
                ("outlineOffset", text("-0.5em")),
            ]),
            (5.0, c("blue"), -8.0)
        );
        assert_eq!(
            outline(&[
                ("outlineStyle", text("solid")),
                ("outlineWidth", text("2.5px")),
                ("outlineColor", text("green")),
                ("outlineColor", text("currentcolor")),
            ]),
            (2.0, color::BLACK, 0.0)
        );
        let offset = |px: &str| one(&[("outlineOffset", text(px))]).outline_offset;
        assert_eq!([offset("2.5px"), offset("-1.5px")], [2.0, -1.0]);
    }

    #[test]
    fn box_shadows_take_lengths_a_color_and_inset() {
        let c = |name| color::parse(name).unwrap();
        let shadows = |v: &str| one(&[("color", text("blue")), ("boxShadow", text(v))]).shadows;
        let shadow = |offset, blur, spread, color, inset| Shadow {
            offset,
            blur,
            spread,
            color,
            inset,
        };
        assert_eq!(shadows("none"), []);
        assert_eq!(
            shadows("1px 2px"),
            [shadow([1.0, 2.0], 0.0, 0.0, c("blue"), false)]
        );
        assert_eq!(
            shadows("inset -1px 0 3px -2px red, 0.5em 0 GREEN"),
            [
                shadow([-1.0, 0.0], 3.0, -2.0, c("red"), true),
                shadow([8.0, 0.0], 0.0, 0.0, c("green"), false),
            ]
        );
        assert_eq!(
            shadows("rgb(0 0 0 / 50%) 1px 1px calc(1px + 1px) inset"),
            [shadow([1.0, 1.0], 2.0, 0.0, [0.0, 0.0, 0.0, 0.5], true)]
        );
    }

    #[test]
    fn backgrounds_take_gradients_over_a_color() {
        use gradient::{Hue, Image, Line, Shape, Space, Stop};
        let c = |name| color::parse(name).unwrap();
        // Colors, `None` for a hint, at px of a 100 px line.
        let stops = |image: &Image| match image {
            Image::Gradient(g) => (g.stops.iter())
                .map(|s| match s {
                    Stop::Color(c, at) => (Some(*c), at.as_ref().map(|a| a.resolve(100.0))),
                    Stop::Hint(at) => (None, Some(at.resolve(100.0))),
                })
                .collect(),
            Image::None | Image::Url(_) => Vec::new(),
        };
        let s = one(&[
            ("color", text("blue")),
            (
                "background",
                text("linear-gradient(red, 10px, currentColor 20% 2em), none, Green"),
            ),
        ]);
        assert_eq!(s.background, c("green"));
        assert_eq!(s.layers[1].image, Image::None);
        assert_eq!(
            stops(&s.layers[0].image),
            [
                (Some(c("red")), None),
                (None, Some(10.0)),
                (Some(c("blue")), Some(20.0)),
                (Some(c("blue")), Some(32.0)),
            ]
        );
        let gradient = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
            [Image::Gradient(g)] => match g.shape {
                Shape::Linear(line) => (line, g.repeating, g.space),
                _ => panic!("{g:?}"),
            },
            other => panic!("{other:?}"),
        };
        assert_eq!(
            gradient("linear-gradient(red, blue)"),
            (Line::Angle(180.0), false, Space::Srgb)
        );
        assert_eq!(
            gradient("linear-gradient(to left, red)").0,
            Line::Angle(270.0)
        );
        assert_eq!(gradient("linear-gradient(0, red)").0, Line::Angle(0.0));
        assert_eq!(
            gradient("repeating-linear-gradient(0.25turn in oklab, red, blue)"),
            (Line::Angle(90.0), true, Space::Oklab)
        );
        assert_eq!(
            gradient("linear-gradient(in oklch longer hue to top left, red, blue)"),
            (Line::Corner([-1.0, -1.0]), false, Space::Oklch(Hue::Longer))
        );
        assert_eq!(
            gradient("linear-gradient(to bottom right in srgb-linear, red, blue)"),
            (Line::Corner([1.0, 1.0]), false, Space::SrgbLinear)
        );
        assert_eq!(
            gradient("linear-gradient(in oklch, red, blue)").2,
            Space::Oklch(Hue::Shorter)
        );
        // Colors past legacy sRGB mix in Oklab unless the gradient says.
        assert_eq!(
            gradient("linear-gradient(oklch(70% 0.2 30), red)").2,
            Space::Oklab
        );
        assert_eq!(
            gradient("linear-gradient(hsl(0 50% 50%), currentColor, #fff, rgb(0 0 0))").2,
            Space::Srgb
        );
        assert_eq!(
            gradient("linear-gradient(calc(30deg * -1), red, blue)").0,
            Line::Angle(-30.0)
        );
        assert_eq!(
            gradient("linear-gradient(to right in lab, red, blue)").2,
            Space::Lab
        );
        assert_eq!(
            gradient("linear-gradient(in lch increasing hue, red, blue)").2,
            Space::Lch(Hue::Increasing)
        );
        assert_eq!(images(&[("background", text("none"))]), [Image::None]);
    }

    #[test]
    fn backgrounds_and_masks_take_images_of_resources() {
        use background::ImageSize;
        use gradient::Image;
        assert_eq!(
            images(&[(
                "backgroundImage",
                text("url(resource://tile), url(\"resource://photo\")")
            )]),
            [Image::Url(1), Image::Url(0)]
        );
        let s = one(&[
            (
                "background",
                text("url('resource://photo') center / cover no-repeat"),
            ),
            ("maskImage", text("url(resource://tile)")),
        ]);
        assert_eq!(s.layers[0].image, Image::Url(0));
        assert_eq!(s.layers[0].size, ImageSize::Cover);
        assert_eq!(s.masks[0].image, Image::Url(1));
        assert_eq!(
            error(&[("backgroundImage", text("url(resource://logo)"))]),
            "s.tsx:1:1: url(resource://logo): the scene declares no resource logo in \
         controls.resources"
        );
    }

    #[test]
    fn background_layers_take_sizes_positions_and_repeats() {
        use background::{ImageSize, Repeat};
        // Each layer's width and height, and position, in 100 px, and repeat.
        let layers = |style: &[(&str, Prop)]| {
            (one(style).layers.into_iter())
                .map(|l| {
                    let size = match l.size {
                        ImageSize::Sized(s) => s.map(|v| v.map(|v| v.resolve(100.0))),
                        other => panic!("{other:?}"),
                    };
                    let at = l.position.each_ref().map(|o| o.resolve(100.0));
                    (size, at, l.repeat)
                })
                .collect::<Vec<_>>()
        };
        let three = text("linear-gradient(red), linear-gradient(blue), linear-gradient(green)");
        let no = [Repeat::NoRepeat; 2];
        // Lists repeat for the images.
        assert_eq!(
            layers(&[
                ("backgroundImage", three.clone()),
                ("backgroundSize", text("10px, 50% auto")),
                ("backgroundPosition", text("right 10px top, center")),
                ("backgroundRepeat", text("no-repeat")),
            ]),
            [
                ([Some(10.0), None], [90.0, 0.0], no),
                ([Some(50.0), None], [50.0, 50.0], no),
                ([Some(10.0), None], [90.0, 0.0], no),
            ]
        );
        // x and y are lists of their own.
        assert_eq!(
            layers(&[
                ("backgroundImage", three),
                ("backgroundPosition", text("left 10px top 20px")),
                ("backgroundPositionX", text("right 5px, 30%")),
            ])
            .into_iter()
            .map(|l| l.1)
            .collect::<Vec<_>>(),
            [[95.0, 20.0], [30.0, 20.0], [95.0, 20.0]]
        );
        // The shorthand sets each, and what it leaves out goes back to its initial value. The color
        // goes with a last layer of its own.
        let background = text(
            "linear-gradient(red) center / 20px 30% repeat-x, \
         linear-gradient(blue) space round bottom 5px right 10%, white",
        );
        let s = one(&[
            ("backgroundSize", text("10px")),
            ("background", background.clone()),
        ]);
        assert_eq!(s.background, [1.0; 4]);
        assert_eq!(s.layers[1].size, ImageSize::Sized([None, None]));
        assert_eq!(
            layers(&[("background", background)]),
            [
                (
                    [Some(20.0), Some(30.0)],
                    [50.0, 50.0],
                    [Repeat::Repeat, Repeat::NoRepeat]
                ),
                ([None, None], [90.0, 95.0], [Repeat::Space, Repeat::Round]),
                ([None, None], [0.0, 0.0], [Repeat::Repeat; 2]),
            ]
        );
        let sizes = one(&[
            ("backgroundImage", text("none, none")),
            ("backgroundSize", text("cover, contain")),
        ]);
        assert_eq!(
            sizes.layers.iter().map(|l| &l.size).collect::<Vec<_>>(),
            [&ImageSize::Cover, &ImageSize::Contain]
        );
    }

    #[test]
    fn background_layers_take_origins_and_clips() {
        use background::Area::{BorderBox, ContentBox, PaddingBox, Text};
        let areas = |style: &[(&str, Prop)]| {
            (one(style).layers.iter())
                .map(|l| (l.origin, l.clip))
                .collect::<Vec<_>>()
        };
        // A layer with no image by default.
        assert_eq!(areas(&[]), [(PaddingBox, BorderBox)]);
        assert_eq!(
            areas(&[
                ("backgroundImage", text("none, none, none")),
                ("backgroundOrigin", text("content-box, border-box")),
                ("backgroundClip", text("padding-box")),
            ]),
            [
                (ContentBox, PaddingBox),
                (BorderBox, PaddingBox),
                (ContentBox, PaddingBox)
            ]
        );
        // In the shorthand one box is both, and of two the first is the origin.
        assert_eq!(
            areas(&[(
                "background",
                text("none content-box, border-box none padding-box, red")
            )]),
            [
                (ContentBox, ContentBox),
                (BorderBox, PaddingBox),
                (PaddingBox, BorderBox)
            ]
        );
        // Text is a clip alone, so a box before it is the origin.
        assert_eq!(
            areas(&[("background", text("none text, none content-box text, red"))]),
            [
                (PaddingBox, Text),
                (ContentBox, Text),
                (PaddingBox, BorderBox)
            ]
        );
        assert_eq!(
            areas(&[("backgroundClip", text("text"))]),
            [(PaddingBox, Text)]
        );
    }

    #[test]
    fn filters_take_css_functions_in_order() {
        use filter::Function as F;
        let s = one(&[
            ("color", text("blue")),
            (
                "filter",
                text(
                    "blur(0.5em) brightness(150%) GRAYSCALE(2) hue-rotate(0.5turn) \
                 drop-shadow(red 1px 2px 3px) drop-shadow(0 1px) opacity()",
                ),
            ),
        ]);
        assert_eq!(
            s.filter,
            [
                F::Blur(8.0),
                F::Brightness(1.5),
                F::Grayscale(1.0),
                F::HueRotate(180.0),
                F::DropShadow {
                    offset: [1.0, 2.0],
                    blur: 3.0,
                    color: [1.0, 0.0, 0.0, 1.0]
                },
                F::DropShadow {
                    offset: [0.0, 1.0],
                    blur: 0.0,
                    color: [0.0, 0.0, 1.0, 1.0]
                },
                F::Opacity(1.0),
            ]
        );
        assert!(s.contains_all());
        assert!(one(&[("filter", text("none"))]).filter.is_empty());
    }

    #[test]
    fn filters_take_valles_functions_in_px_within_their_ranges() {
        use filter::Function as F;
        let filter = |v: &str| one(&[("filter", text(v))]).filter;
        assert_eq!(
            filter("chromatic-aberration(-4.5px) blur(1px) chromatic-aberration(0)"),
            [
                F::ChromaticAberration(-4.5),
                F::Blur(1.0),
                F::ChromaticAberration(0.0)
            ]
        );
        let backdrop = one(&[("backdropFilter", text("chromatic-aberration(256px)"))]);
        assert_eq!(backdrop.backdrop, [F::ChromaticAberration(256.0)]);
        assert_eq!(
            filter("lens-distortion(0.25 -0.5)"),
            [F::LensDistortion([0.25, -0.5])]
        );
        assert_eq!(
            filter("film-grain(4294967295 0.5 2.5px)"),
            [F::FilmGrain {
                seed: u32::MAX,
                amount: 0.5,
                size: 2.5
            }]
        );
        assert_eq!(
            filter("radial-blur(-10px 0 128px)"),
            [F::RadialBlur {
                center: [-10.0, 0.0],
                amount: 128.0
            }]
        );
        let noise = |frequency, octaves, turbulence| {
            [F::Displacement {
                seed: 7,
                frequency,
                scale: -6.0,
                octaves,
                turbulence,
            }]
        };
        let both = filter("displacement(7 0.01 0.5 -6px)");
        assert_eq!(both, noise([0.01, 0.5], 2, false));
        let turbulent = filter("displacement(7 0 1 -6px 4 Turbulence)");
        assert_eq!(turbulent, noise([0.0, 1.0], 4, true));
        let fractal = filter("displacement(7 0 1 -6px 1 fractal)");
        assert_eq!(fractal, noise([0.0, 1.0], 1, false));
        let bloom = |knee| {
            [F::Bloom {
                radius: 12.0,
                intensity: 4.0,
                threshold: 0.5,
                knee,
            }]
        };
        assert_eq!(filter("bloom(12px 4 0.5)"), bloom(0.1));
        assert_eq!(filter("bloom(12px 4 0.5 0)"), bloom(0.0));
        for (v, says) in [
            (
                "chromatic-aberration(300px)",
                "the offset 300px is not within -256px and 256px",
            ),
            ("chromatic-aberration(1em)", "the offset 1em is not px"),
            ("chromatic-aberration(4)", "the offset 4 is not px"),
            ("chromatic-aberration()", "takes the offset"),
            ("chromatic-aberration(1px 2px)", "takes the offset"),
            (
                "lens-distortion(0.6 0)",
                "k1 0.6 is not within -0.5 and 0.5",
            ),
            ("lens-distortion(0 1px)", "k2 1px is not a number"),
            ("lens-distortion(0.1)", "takes k1, k2"),
            (
                "film-grain(1.5 0.5 2px)",
                "the seed 1.5 is not a whole number",
            ),
            ("film-grain(-1 0.5 2px)", "the seed -1 is not within"),
            ("radial-blur(10px 10px)", "takes x, y, the amount"),
            ("radial-blur(50% 10px 4px)", "x 50% is not px"),
            (
                "radial-blur(1px 1px 129px)",
                "the amount 129px is not within 0px and 128px",
            ),
            (
                "displacement(7 0.1 0.1)",
                "takes the seed, fx, fy and the scale",
            ),
            ("displacement(7 0.1 0.1 4)", "the scale 4 is not px"),
            (
                "displacement(7 0.1 1.5 4px)",
                "fy 1.5 is not within 0 and 1",
            ),
            (
                "displacement(7 0.1 0.1 4px 2.5)",
                "the octaves 2.5 is not a whole number",
            ),
            (
                "displacement(7 0.1 0.1 4px 5)",
                "the octaves 5 is not within 1 and 4",
            ),
            (
                "displacement(7 0.1 0.1 4px 2 wavy)",
                "the mode wavy is not fractal or turbulence",
            ),
            (
                "bloom(12px 1)",
                "takes the radius, the intensity and the threshold",
            ),
            ("bloom(12 1 0.5)", "the radius 12 is not px"),
            ("bloom(12px 5 0.5)", "the intensity 5 is not within 0 and 4"),
            ("bloom(12px 1 0.5 2)", "the knee 2 is not within 0 and 1"),
            (
                "film-grain(1 0.5 0.5px)",
                "the size 0.5px is not within 1px and 64px",
            ),
        ] {
            let e = error(&[("filter", text(v))]);
            assert!(e.contains(says), "{v}: {e}");
        }
    }

    #[test]
    fn clip_paths_take_basic_shapes_in_a_box() {
        use background::Area;
        use shape::{Clip, Radius, Shape};
        let clip = |v: &str| one(&[("clipPath", text(v))]).clip;
        let px = |v: f64| Math::Sum([v, 0.0]);
        let percent = |v: f64| Math::Sum([0.0, v / 100.0]);
        let center = || position::Offset {
            end: false,
            by: percent(50.0),
        };
        assert_eq!(clip("none"), None);
        assert_eq!(
            clip("Padding-Box"),
            Some(Clip {
                shape: None,
                area: Area::PaddingBox
            })
        );
        let Some(Clip {
            shape: Some(Shape::Inset { sides, radii }),
            area: Area::BorderBox,
        }) = clip("inset(1px 10% round 2px 50% / 1em)")
        else {
            panic!()
        };
        assert_eq!(sides, [px(1.0), percent(10.0), px(1.0), percent(10.0)]);
        assert_eq!(radii[0], [px(2.0), px(16.0)]);
        assert_eq!(radii[1], [percent(50.0), px(16.0)]);
        assert_eq!(
            clip("content-box CIRCLE(at right 5px top 0)"),
            Some(Clip {
                shape: Some(Shape::Circle {
                    radius: Radius::ClosestSide,
                    at: [
                        position::Offset {
                            end: true,
                            by: px(5.0)
                        },
                        position::Offset {
                            end: false,
                            by: percent(0.0)
                        }
                    ],
                }),
                area: Area::ContentBox,
            })
        );
        assert_eq!(
            clip("ellipse(farthest-side 20%)").unwrap().shape,
            Some(Shape::Ellipse {
                radii: [Radius::FarthestSide, Radius::Len(percent(20.0))],
                at: [center(), center()],
            })
        );
        assert_eq!(
            clip("polygon(EVENODD, 0 0, 100% 0, 50% calc(100% - 2px))")
                .unwrap()
                .shape,
            Some(Shape::Polygon {
                even_odd: true,
                points: vec![
                    [px(0.0), px(0.0)],
                    [percent(100.0), px(0.0)],
                    [percent(50.0), Math::Sum([-2.0, 1.0])],
                ],
            })
        );
        // Path data keeps its case: a lower case command is relative.
        assert_eq!(
            clip("path(\"M1 2 l3 0 Z\") padding-box"),
            Some(Clip {
                shape: Some(Shape::Path {
                    even_odd: false,
                    path: crate::path::parse("M1 2 L4 2 Z").unwrap(),
                }),
                area: Area::PaddingBox,
            })
        );
        let s = one(&[("clipPath", text("circle()"))]);
        assert!(!s.contains_all());
        let flat = one(&[
            ("transformStyle", text("preserve-3d")),
            ("clipPath", text("circle()")),
        ]);
        assert!(!flat.keeps_3d());
    }

    #[test]
    fn masks_are_layers_of_images_shown_by_alpha_or_luminance() {
        use background::{Area, ImageSize, Repeat};
        assert!(one(&[]).masks.is_empty());
        assert!(one(&[("maskImage", text("none, none"))]).masks.is_empty());
        let s = one(&[
            ("maskImage", text("linear-gradient(#000, #0000), none")),
            ("maskSize", text("10px auto")),
            ("maskPosition", text("right 4px bottom 2px")),
            ("maskRepeat", text("no-repeat")),
            ("maskMode", text("LUMINANCE, match-source")),
        ]);
        let [a, b] = &s.masks[..] else {
            panic!("{:?}", s.masks)
        };
        assert!(a.luminance && !b.luminance);
        assert!(matches!(b.image, gradient::Image::None));
        // Placed in the border box, where backgrounds are placed in the padding box.
        assert_eq!(a.origin, Area::BorderBox);
        assert_eq!(
            one(&[("backgroundImage", text("linear-gradient(red, blue)"))]).layers[0].origin,
            Area::PaddingBox
        );
        assert_eq!(
            a.size,
            ImageSize::Sized([Some(Math::Sum([10.0, 0.0])), None])
        );
        let from_end = |px| position::Offset {
            end: true,
            by: Math::Sum([px, 0.0]),
        };
        assert_eq!(b.position, [from_end(4.0), from_end(2.0)]);
        assert_eq!(b.repeat, [Repeat::NoRepeat; 2]);
        assert!(!s.contains_all());
        let flat = one(&[
            ("transformStyle", text("preserve-3d")),
            ("maskImage", text("linear-gradient(#000, #0000)")),
        ]);
        assert!(!flat.keeps_3d());
    }

    #[test]
    fn background_layers_take_blend_modes_which_the_shorthand_leaves() {
        let blends = |style: &[(&str, Prop)]| {
            (one(style).layers.iter())
                .map(|l| l.blend)
                .collect::<Vec<_>>()
        };
        assert_eq!(blends(&[]), [Blend::Normal]);
        let style = [
            ("backgroundBlendMode", text("multiply, Screen")),
            ("background", text("none, none, none, red")),
        ];
        assert_eq!(
            blends(&style),
            [
                Blend::Multiply,
                Blend::Screen,
                Blend::Multiply,
                Blend::Screen
            ]
        );
    }

    /// The transform functions of a style, translations resolved against a 100 px box.
    fn functions(style: &[(&str, Prop)]) -> Vec<transform::Function<f32>> {
        use transform::Function as F;
        (one(style).transform.into_iter())
            .map(|f| match f {
                F::Translate(t) => F::Translate(t.map(|l| l.resolve(100.0))),
                F::Scale(s) => F::Scale(s),
                F::Rotate(a) => F::Rotate(a),
                F::Rotate3d(axis, a) => F::Rotate3d(axis, a),
                F::Skew(a) => F::Skew(a),
                F::Matrix(m) => F::Matrix(m),
                F::Matrix3d(m) => F::Matrix3d(m),
                F::Perspective(d) => F::Perspective(d.map(|l| l.resolve(0.0))),
            })
            .collect()
    }

    #[test]
    fn transforms_take_2d_functions_around_an_origin() {
        use transform::Function as F;
        let functions = |v: &str| functions(&[("transform", text(v))]);
        assert_eq!(functions("none"), []);
        assert_eq!(
            functions("translate(10px, 50%) translateX(1em) translateY(-2px) translate(5%)"),
            [
                F::Translate([10.0, 50.0, 0.0]),
                F::Translate([16.0, 0.0, 0.0]),
                F::Translate([0.0, -2.0, 0.0]),
                F::Translate([5.0, 0.0, 0.0]),
            ]
        );
        assert_eq!(
            functions("scale(2) scale(50%, 3) scaleX(-1) scaleY(0.5)"),
            [
                F::Scale([2.0, 2.0, 1.0]),
                F::Scale([0.5, 3.0, 1.0]),
                F::Scale([-1.0, 1.0, 1.0]),
                F::Scale([1.0, 0.5, 1.0]),
            ]
        );
        assert_eq!(
            functions("rotate(0.25turn) rotateZ(-10deg) rotate(0) skew(10deg) skewY(100grad)"),
            [
                F::Rotate(90.0),
                F::Rotate(-10.0),
                F::Rotate(0.0),
                F::Skew([10.0, 0.0]),
                F::Skew([0.0, 90.0]),
            ]
        );
        // Math functions of angles come to degrees.
        assert_eq!(
            functions(
                "rotate(calc(45deg * -1)) skewX(min(0.1turn, 30deg)) rotate(calc(1rad - 1rad))"
            ),
            [F::Rotate(-45.0), F::Skew([30.0, 0.0]), F::Rotate(0.0)]
        );
        assert_eq!(
            functions(" MATRIX(1, 0, 0.5, 1, 10, -5) "),
            [F::Matrix([1.0, 0.0, 0.5, 1.0, 10.0, -5.0])]
        );
        // From the border box's top left, the center by default.
        let origin = |v: Option<Prop>| {
            let style: Vec<_> = v.into_iter().map(|v| ("transformOrigin", v)).collect();
            one(&style).transform_origin.map(|o| o.resolve(100.0))
        };
        assert_eq!(origin(None), [50.0, 50.0]);
        assert_eq!(origin(Some(text("left"))), [0.0, 50.0]);
        assert_eq!(origin(Some(text("bottom right"))), [100.0, 100.0]);
        assert_eq!(origin(Some(text("10px 20%"))), [10.0, 20.0]);
        assert_eq!(origin(Some(num(5.0))), [5.0, 50.0]);
    }

    #[test]
    fn motion_paths_take_paths_rays_and_shapes_with_their_longhands() {
        use background::Area;
        use offset::{Route, Size, Start};
        let motion = |style: &[(&str, Prop)]| one(style).motion;
        assert_eq!(motion(&[]), None);
        assert_eq!(motion(&[("offsetDistance", text("10px"))]), None);
        assert_eq!(motion(&[("offsetPath", text("none"))]), None);
        // Path data as written; its longhands as they start: auto, no angle, the origin, normal.
        let m = motion(&[("offsetPath", text("path('M0 0 L10 0 Z')"))]).unwrap();
        assert_eq!(
            m.path,
            Route::Path(crate::path::parse("M0 0 L10 0 Z").unwrap())
        );
        assert_eq!(
            (m.area, m.along, m.angle, &m.anchor, &m.position),
            (Area::BorderBox, true, 0.0, &None, &Start::Normal)
        );
        assert_eq!(m.distance.resolve(100.0), 0.0);
        let m = motion(&[
            (
                "offsetPath",
                text("Ray(45DEG sides at 10px 20%) padding-box"),
            ),
            ("offsetDistance", text("calc(50% + 1em)")),
            ("offsetRotate", text("30deg reverse")),
            ("offsetAnchor", text("left top")),
            ("offsetPosition", text("auto")),
        ])
        .unwrap();
        let Route::Ray { angle, size, at } = &m.path else {
            panic!("{:?} is not a ray", m.path);
        };
        let at = at.as_ref().map(|a| a.each_ref().map(|o| o.resolve(100.0)));
        assert_eq!((*angle, *size, at), (45.0, Size::Sides, Some([10.0, 20.0])));
        assert_eq!(m.area, Area::PaddingBox);
        assert_eq!(m.distance.resolve(100.0), 66.0);
        assert_eq!((m.along, m.angle), (true, 210.0));
        let anchor = m.anchor.map(|a| a.map(|o| o.resolve(100.0)));
        assert_eq!((anchor, m.position), (Some([0.0, 0.0]), Start::Auto));
        let m = motion(&[
            ("offsetPath", text("ray(0deg)")),
            ("offsetRotate", text("-10deg")),
        ]);
        assert_eq!(m.map(|m| (m.along, m.angle)), Some((false, -10.0)));
        // A circle or an ellipse with no `at` is around the offset position; a box alone is a path.
        let shape = |v: &str| match motion(&[("offsetPath", text(v))]).unwrap().path {
            Route::Shape { shape, at } => (shape.is_some(), at),
            other => panic!("{other:?} is not a shape"),
        };
        assert_eq!(shape("circle(10px)"), (true, false));
        assert_eq!(shape("ellipse(1px 2px at center)"), (true, true));
        assert_eq!(shape("inset(1px round 2px)"), (true, true));
        assert_eq!(shape("content-box"), (false, true));
        let m = motion(&[("offsetPath", text("content-box polygon(0 0, 1px 0)"))]);
        assert_eq!(m.map(|m| m.area), Some(Area::ContentBox));
        // It is a transform, so it contains all.
        assert!(one(&[("offsetPath", text("ray(0deg)"))]).contains_all());
        for (property, v, says) in [
            (
                "offsetPath",
                "ray(0deg contain)",
                "contain is not supported",
            ),
            (
                "offsetPath",
                "ray(0deg 10px)",
                "10px is not an angle or a size",
            ),
            ("offsetPath", "ray(closest-side)", "takes an angle"),
            ("offsetPath", "path('M0 0') content-box", "takes no box"),
            ("offsetPath", "url(#a)", "url() is not an offset path"),
            ("offsetPath", "circle() ray(0deg)", "takes one path"),
            ("offsetRotate", "auto reverse", "is not an offset rotate"),
            ("offsetAnchor", "normal", "is not an offset anchor"),
            (
                "offsetPosition",
                "left top 10px",
                "is not an offset position",
            ),
            ("offsetDistance", "auto", "auto is not a length"),
        ] {
            let e = error(&[(property, text(v))]);
            assert!(e.contains(says), "{property}: {v}: {e}");
        }
    }

    #[test]
    fn transforms_take_3d_functions_and_an_origin_z() {
        use transform::Function as F;
        assert_eq!(
            functions(&[(
                "transform",
                text("translateZ(2em) translate3d(10%, 1px, -5px) scaleZ(2) scale3d(1, 50%, 3)")
            )]),
            [
                F::Translate([0.0, 0.0, 32.0]),
                F::Translate([10.0, 1.0, -5.0]),
                F::Scale([1.0, 1.0, 2.0]),
                F::Scale([1.0, 0.5, 3.0]),
            ]
        );
        assert_eq!(
            functions(&[(
                "transform",
                text(
                    "rotateX(90deg) rotateY(-0.5turn) rotate3d(1, 1, 0, 0) perspective(0) perspective(none)"
                )
            )]),
            [
                F::Rotate3d([1.0, 0.0, 0.0], 90.0),
                F::Rotate3d([0.0, 1.0, 0.0], -180.0),
                F::Rotate3d([1.0, 1.0, 0.0], 0.0),
                F::Perspective(Some(0.0)),
                F::Perspective(None),
            ]
        );
        let identity = "1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5, 6, 7, 1";
        assert_eq!(
            functions(&[("transform", text(&format!("matrix3d({identity})")))]),
            [F::Matrix3d([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0
            ])]
        );
        // Each property's z and axes.
        assert_eq!(
            functions(&[
                ("translate", text("1px 2px 3px")),
                ("rotate", text("10deg y")),
                ("scale", text("1 2 3")),
            ]),
            [
                F::Translate([1.0, 2.0, 3.0]),
                F::Rotate3d([0.0, 1.0, 0.0], 10.0),
                F::Scale([1.0, 2.0, 3.0]),
            ]
        );
        assert_eq!(
            functions(&[("rotate", text("1 0 0 10deg"))]),
            [F::Rotate3d([1.0, 0.0, 0.0], 10.0)]
        );
        assert_eq!(
            functions(&[("rotate", text("20deg 0 0 1"))]),
            [F::Rotate(20.0)]
        );
        let z = |v: &str| one(&[("transformOrigin", text(v))]).transform_origin_z;
        assert_eq!(z("left"), 0.0);
        assert_eq!(z("left top 10px"), 10.0);
        assert_eq!(z("10px 20% -1em"), -16.0);
        let seen = |style: &[(&str, Prop)]| {
            let s = one(style);
            (
                s.perspective,
                s.perspective_origin.map(|o| o.resolve(100.0)),
            )
        };
        assert_eq!(seen(&[]), (None, [50.0, 50.0]));
        assert_eq!(
            seen(&[
                ("perspective", num(200.0)),
                ("perspectiveOrigin", text("left 10px bottom 5%"))
            ]),
            (Some(200.0), [10.0, 95.0])
        );
        assert_eq!(seen(&[("perspective", text("0"))]).0, Some(0.0));
        assert!(one(&[]).backface_visible);
        assert!(!one(&[("backfaceVisibility", text("Hidden"))]).backface_visible);
        let keeps = |style: &[(&str, Prop)]| one(style).keeps_3d();
        assert!(!keeps(&[]));
        assert!(keeps(&[("transformStyle", text("preserve-3d"))]));
        let grouped = [
            ("transformStyle", text("preserve-3d")),
            ("overflow", text("hidden")),
        ];
        assert!(!keeps(&grouped));
    }

    #[test]
    fn translate_rotate_and_scale_come_before_transform() {
        use transform::Function as F;
        assert_eq!(
            functions(&[
                ("transform", text("skew(10deg)")),
                ("scale", text("2 50%")),
                ("rotate", text("Z 90DEG")),
                ("translate", text("10px 5%")),
            ]),
            [
                F::Translate([10.0, 5.0, 0.0]),
                F::Rotate(90.0),
                F::Scale([2.0, 0.5, 1.0]),
                F::Skew([10.0, 0.0]),
            ]
        );
        assert_eq!(
            functions(&[
                ("translate", num(4.0)),
                ("rotate", text("-0.25turn z")),
                ("scale", num(3.0))
            ]),
            [
                F::Translate([4.0, 0.0, 0.0]),
                F::Rotate(-90.0),
                F::Scale([3.0, 3.0, 1.0]),
            ]
        );
        let none = [
            ("translate", text("none")),
            ("rotate", text("none")),
            ("scale", text("none")),
        ];
        assert_eq!(functions(&none), []);
    }

    #[test]
    fn radial_gradients_take_a_shape_a_size_and_a_center() {
        use gradient::{Image, Shape, Size, Space};
        // The shape, the size in px of 100 px, and the center from the top left of 100 × 100 px.
        let radial = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
            [Image::Gradient(g)] => match &g.shape {
                Shape::Radial { circle, size, at } => {
                    let size = match size {
                        Size::Radii(r) => Some(r.each_ref().map(|r| r.resolve(100.0))),
                        _ => None,
                    };
                    let at = at.each_ref().map(|o| o.resolve(100.0));
                    (*circle, size, at, g.repeating, g.space)
                }
                _ => panic!("{g:?}"),
            },
            other => panic!("{other:?}"),
        };
        let size = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
            [Image::Gradient(g)] => match &g.shape {
                Shape::Radial { size, .. } => size.clone(),
                _ => panic!("{g:?}"),
            },
            other => panic!("{other:?}"),
        };
        assert_eq!(
            radial("radial-gradient(red, blue)"),
            (false, None, [50.0, 50.0], false, Space::Srgb)
        );
        assert_eq!(size("radial-gradient(red, blue)"), Size::FarthestCorner);
        assert_eq!(
            size("radial-gradient(closest-side circle, red)"),
            Size::ClosestSide
        );
        assert_eq!(
            radial("radial-gradient(20px, red)"),
            (true, Some([20.0, 20.0]), [50.0, 50.0], false, Space::Srgb)
        );
        assert_eq!(
            radial("repeating-radial-gradient(10px 20% at left 5px bottom 10%, red)"),
            (false, Some([10.0, 20.0]), [5.0, 90.0], true, Space::Srgb)
        );
        assert_eq!(
            radial("radial-gradient(in oklab circle closest-corner at top, red)"),
            (true, None, [50.0, 0.0], false, Space::Oklab)
        );
        assert_eq!(
            radial("radial-gradient(circle at 25% calc(10px + 5%) in oklab, red)").2,
            [25.0, 15.0]
        );
        assert_eq!(
            radial("radial-gradient(at right center, red)").2,
            [100.0, 50.0]
        );
        assert_eq!(radial("radial-gradient(at 0, red)").2, [0.0, 50.0]);
    }

    #[test]
    fn conic_gradients_take_an_angle_a_center_and_angles() {
        use gradient::{Image, Shape, Space, Stop};
        // From, the center in 100 × 100 px, and the stops in degrees.
        let conic = |v: &str| match &images(&[("backgroundImage", text(v))])[..] {
            [Image::Gradient(g)] => match &g.shape {
                Shape::Conic { from, at } => {
                    let stops: Vec<Option<f32>> = (g.stops.iter())
                        .map(|s| match s {
                            Stop::Color(_, at) => at.as_ref().map(|a| a.resolve(360.0)),
                            Stop::Hint(at) => Some(at.resolve(360.0)),
                        })
                        .collect();
                    let at = at.each_ref().map(|o| o.resolve(100.0));
                    (*from, at, stops, g.repeating, g.space)
                }
                _ => panic!("{g:?}"),
            },
            other => panic!("{other:?}"),
        };
        assert_eq!(
            conic("conic-gradient(red, blue)"),
            (0.0, [50.0, 50.0], vec![None, None], false, Space::Srgb)
        );
        assert_eq!(
            conic("conic-gradient(from 90deg at 25% 0, red 10%, 25%, blue 0.25turn 180deg, green)"),
            (
                90.0,
                [25.0, 0.0],
                vec![Some(36.0), Some(90.0), Some(90.0), Some(180.0), None],
                false,
                Space::Srgb
            )
        );
        assert_eq!(
            conic("repeating-conic-gradient(in oklab from 0.5turn, red 0 10deg)"),
            (
                180.0,
                [50.0, 50.0],
                vec![Some(0.0), Some(10.0)],
                true,
                Space::Oklab
            )
        );
        assert_eq!(
            conic("conic-gradient(at left in oklab, red)").1,
            [0.0, 50.0]
        );
    }

    #[test]
    fn corner_radii_expand_as_sides_do() {
        // Each corner's radii, percentages of 100 px.
        let radii = |style: &[(&str, Prop)]| one(style).radii.map(|c| c.map(|r| r.resolve(100.0)));
        assert_eq!(
            radii(&[("borderRadius", text("1px 2px 3px / 4px 5%"))]),
            [[1.0, 4.0], [2.0, 5.0], [3.0, 4.0], [2.0, 5.0]]
        );
        assert_eq!(radii(&[("borderRadius", num(6.0))]), [[6.0; 2]; 4]);
        assert_eq!(
            radii(&[
                ("fontSize", num(20.0)),
                ("borderRadius", text("8px")),
                ("borderTopLeftRadius", text("0.5em")),
                ("borderStartEndRadius", text("calc(10% + 1px)")),
                ("borderEndEndRadius", text("max(-5%, 2px) 3px")),
                ("borderEndStartRadius", text("2px 3px")),
            ]),
            [[10.0, 10.0], [11.0, 11.0], [2.0, 3.0], [2.0, 3.0]]
        );
    }

    #[test]
    fn paint_values() {
        let s = one(&[
            ("fontSize", num(20.0)),
            ("backgroundColor", text("#ff000080")),
            ("borderRadius", text("0.5em")),
            ("opacity", text("50%")),
        ]);
        assert_eq!(s.background, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        assert_eq!(s.radii[2][1].resolve(0.0), 10.0);
        assert_eq!(s.opacity, 0.5);
        assert_eq!(
            one(&[("background", text("none"))]).background,
            color::TRANSPARENT
        );
        assert_eq!(one(&[("background", text("white"))]).background, [1.0; 4]);
        assert_eq!(one(&[("opacity", num(2.0))]).opacity, 1.0);
    }

    #[test]
    fn blends_and_isolation_group_what_is_in_a_3d_box() {
        assert_eq!(one(&[]).blend, Blend::Normal);
        let s = one(&[("mixBlendMode", text(" Color-Dodge "))]);
        assert_eq!(s.blend, Blend::ColorDodge);
        assert_eq!(
            one(&[("mixBlendMode", text("plus-lighter"))]).blend,
            Blend::PlusLighter
        );
        assert!(!one(&[]).isolate);
        assert!(one(&[("isolation", text("isolate"))]).isolate);
        let keeps = |style: &[(&str, Prop)]| one(style).keeps_3d();
        let flat = [
            ("transformStyle", text("preserve-3d")),
            ("mixBlendMode", text("screen")),
        ];
        assert!(!keeps(&flat));
        let flat = [
            ("transformStyle", text("preserve-3d")),
            ("isolation", text("isolate")),
        ];
        assert!(!keeps(&flat));
    }
}
