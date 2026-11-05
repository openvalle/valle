//! Font and text style values.

use std::rc::Rc;

use super::color::Rgba;
use super::declaration::{Declared, factor, number, shown, text};
use super::math::Calc;
use super::units::{Len, PADDING, length_text};
use super::{css, math};

use crate::template::V;

/// `font-style`: upright, italic, or oblique by an angle, degrees, clockwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique(f64),
}

impl FontStyle {
    /// Whether Chrome slants a face with no italic of its own for it: for italic, and for
    /// oblique by 14deg or more, by a quarter of the height whatever the angle.
    pub fn slants(self) -> bool {
        match self {
            FontStyle::Normal => false,
            FontStyle::Italic => true,
            FontStyle::Oblique(angle) => angle >= 14.0,
        }
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
    pub style: DecorationStyle,
    pub color: Rgba,
    /// `text-decoration-thickness` and `text-underline-offset`, px and fractions of the font
    /// size of the decorating box, which percentages are of; `None` is `auto`.
    pub thickness: Option<Calc>,
    pub offset: Option<Calc>,
    /// The node that declares it.
    pub by: usize,
}

/// `text-decoration-style`: a decoration's lines solid, two lines, dots, dashes or a wave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// The inherited text properties, computed.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// px.
    pub size: f64,
    /// 1–1000.
    pub weight: f64,
    /// `font-stretch`, % of the normal width.
    pub stretch: f64,
    pub style: FontStyle,
    pub family: Rc<[Family]>,
    pub line_height: LineHeight,
    pub align: TextAlign,
    /// `text-indent` of the first line, which the text's own box takes, not a Span's; its
    /// percentages are of the width lines break to fit.
    pub indent: Option<Calc>,
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
    pub(super) fn initial() -> Font {
        Font {
            size: 16.0,
            weight: 400.0,
            stretch: 100.0,
            style: FontStyle::Normal,
            family: Rc::new([Family::SansSerif]),
            line_height: LineHeight::Normal,
            align: TextAlign::Start,
            indent: None,
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
    Emoji,
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
    /// Lines full to both edges, but for a paragraph's last.
    Justify,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Weight {
    Absolute(f64),
    Bolder,
    Lighter,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Leading {
    Normal,
    Factor(f64),
    Len(Len),
}

pub(super) fn weight(v: &V) -> Result<Weight, String> {
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

/// `font-stretch`: a keyword, or a percentage of the normal width, 0% or more.
pub(super) fn stretch(v: &V) -> Result<f64, String> {
    let keywords = [
        ("ultra-condensed", 50.0),
        ("extra-condensed", 62.5),
        ("condensed", 75.0),
        ("semi-condensed", 87.5),
        ("normal", 100.0),
        ("semi-expanded", 112.5),
        ("expanded", 125.0),
        ("extra-expanded", 150.0),
        ("ultra-expanded", 200.0),
    ];
    let s = text(v)?;
    if let Some((_, p)) = keywords.iter().find(|(k, _)| *k == s) {
        return Ok(*p);
    }
    match css::num(&s) {
        Some(css::Num { value, unit: "%" }) if value >= 0.0 => Ok(value),
        _ => {
            let names: Vec<&str> = keywords.iter().map(|(k, _)| *k).collect();
            Err(format!(
                "{s} is not a percentage, 0% or more, or one of {}",
                names.join(", ")
            ))
        }
    }
}

/// `font-style`: `normal`, `italic`, or `oblique` and an angle from -90deg to 90deg, 14deg if
/// left out.
pub(super) fn font_style(v: &V) -> Result<FontStyle, String> {
    let s = text(v)?;
    match css::words(&s)[..] {
        ["normal"] => Ok(FontStyle::Normal),
        ["italic"] => Ok(FontStyle::Italic),
        ["oblique"] => Ok(FontStyle::Oblique(14.0)),
        ["oblique", a] => match math::degrees(a) {
            Ok(d) if (-90.0..=90.0).contains(&d) => Ok(FontStyle::Oblique(d)),
            _ => Err(format!("{a} is not an angle from -90deg to 90deg")),
        },
        _ => Err(format!(
            "{s} is not normal, italic, or oblique and an angle"
        )),
    }
}

pub(super) fn leading(v: &V) -> Result<Leading, String> {
    if let V::Num(_) = v {
        return Ok(Leading::Factor(factor(v)?));
    }
    let s = text(v)?;
    if s == "normal" {
        return Ok(Leading::Normal);
    }
    match css::num(&s) {
        Some(n) if n.unit.is_empty() => Ok(Leading::Factor(factor(&V::Num(n.value))?)),
        // A math function of numbers is a factor, clamped to 0 as CSS clamps one.
        _ if math::is_math(&s) && math::number(&s).is_ok() => {
            Ok(Leading::Factor(math::number(&s)?.max(0.0)))
        }
        _ => Ok(Leading::Len(length_text(&s, PADDING)?)),
    }
}

/// CSS's generic families other than the three built in.
const OTHER_GENERICS: [&str; 9] = [
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "math",
    "fangsong",
];

/// The keywords of each `font-variant-*` longhand, or `font-kerning`, in groups, each keyword
/// with the features it sets; a value takes at most one of a group.
pub(super) type Groups =
    &'static [&'static [(&'static str, &'static [(&'static [u8; 4], u16)])]];

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

pub(super) const POSITION: Groups = &[&[("sub", &[(b"subs", 1)]), ("super", &[(b"sups", 1)])]];

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

    pub(super) fn groups(self) -> Groups {
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
pub(super) fn variants(v: &V, variant: Option<Variant>, d: &mut Declared) -> Result<(), String> {
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
pub(super) fn features(v: &V) -> Result<Rc<[Feature]>, String> {
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
pub(super) fn variations(v: &V) -> Result<Rc<[Variation]>, String> {
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

pub(super) fn families(v: &V) -> Result<Rc<[Family]>, String> {
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
                    "emoji" => Family::Emoji,
                    g if OTHER_GENERICS.contains(&g) => {
                        return Err(format!(
                            "{g} is not built in: the built-in families are sans-serif, serif, \
                             monospace and emoji, and others come from font resources"
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

/// CSS Fonts' relative weights.
pub(super) fn bolder(w: f64) -> f64 {
    match w {
        w if w < 350.0 => 400.0,
        w if w < 550.0 => 700.0,
        w if w < 900.0 => 900.0,
        w => w,
    }
}

pub(super) fn lighter(w: f64) -> f64 {
    match w {
        w if w < 100.0 => w,
        w if w < 550.0 => 100.0,
        w if w < 750.0 => 400.0,
        _ => 700.0,
    }
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

#[cfg(test)]
mod tests {

    use std::rc::Rc;

    use crate::style::math::Calc;

    use crate::template::{Kind, Node, Prop};

    use super::*;
    use crate::style::*;
    use crate::testing::styles::{error, num, one, styles, text, view};

    #[test]
    fn white_space_sets_its_collapsing_and_wrapping() {
        let font = |v: &str| one(&[("whiteSpace", text(v))]).font;
        for (v, collapse, wrap) in [
            ("pre", Collapse::Preserve, false),
            ("nowrap", Collapse::Collapse, false),
            ("break-spaces", Collapse::BreakSpaces, true),
            ("Preserve-Breaks NOWRAP", Collapse::PreserveBreaks, false),
            ("nowrap preserve", Collapse::Preserve, false),
            ("wrap", Collapse::Collapse, true),
        ] {
            let f = font(v);
            assert_eq!((f.collapse, f.wrap), (collapse, wrap), "{v}");
        }
        let f = one(&[
            ("whiteSpace", text("nowrap")),
            ("whiteSpaceCollapse", text("preserve")),
        ])
        .font;
        assert_eq!((f.collapse, f.wrap), (Collapse::Preserve, false));
        for bad in ["pre pre", "wrap nowrap", "auto"] {
            let e = error(&[("whiteSpace", text(bad))]);
            assert!(e.contains("is not a white-space"), "{bad}: {e}");
        }
    }

    #[test]
    fn text_decorations_pass_to_the_text_in_flow_inside() {
        let tree = vec![
            view(
                &[
                    ("color", text("red")),
                    ("fontSize", num(20.0)),
                    ("textDecoration", text("underline overline 10%")),
                    ("textUnderlineOffset", text("0.5em")),
                ],
                &[1, 2],
            ),
            view(
                &[
                    ("textDecorationLine", text("line-through")),
                    ("textDecorationColor", text("blue")),
                    ("textDecorationThickness", num(3.0)),
                    ("textUnderlineOffset", text("20%")),
                    ("textDecorationSkipInk", text("none")),
                ],
                &[],
            ),
            view(&[("position", text("absolute"))], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        let red = color::parse("red").unwrap();
        let px = |c: &Option<Calc>| c.as_ref().map(|c| c.resolve(10.0));
        let d = &s[0].decorations;
        assert_eq!(d.len(), 1);
        assert_eq!(
            (d[0].lines, d[0].color, d[0].by),
            ([true, true, false], red, 0)
        );
        // Percentages are of the decorating box's font size; em is of the declaring box's.
        assert_eq!(
            (px(&d[0].thickness), px(&d[0].offset)),
            (Some(1.0), Some(10.0))
        );
        let d = &s[1].decorations;
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], s[0].decorations[0]);
        assert_eq!((d[1].lines, d[1].by), ([false, false, true], 1));
        assert_eq!(d[1].color, color::parse("blue").unwrap());
        assert_eq!(
            (px(&d[1].thickness), px(&d[1].offset)),
            (Some(3.0), Some(2.0))
        );
        assert!(s[0].font.skip_ink && !s[1].font.skip_ink);
        assert!(s[2].decorations.is_empty());
    }

    #[test]
    fn text_decoration_sets_its_longhands_or_refuses() {
        let d = |v: &str| one(&[("textDecoration", text(v))]).decorations;
        assert!(d("none").is_empty());
        let all = &d("Line-Through underline solid 2px #00f")[0];
        assert_eq!(
            (all.lines, all.style),
            ([true, false, true], DecorationStyle::Solid)
        );
        let wavy = one(&[
            ("textDecoration", text("underline wavy")),
            ("textDecorationStyle", text("dotted")),
        ]);
        assert_eq!(wavy.decorations[0].style, DecorationStyle::Dotted);
        assert_eq!(all.color, color::parse("#00f").unwrap());
        assert_eq!(all.thickness.as_ref().map(|t| t.resolve(16.0)), Some(2.0));
        let reset = one(&[
            ("textDecorationColor", text("red")),
            ("textDecoration", text("underline")),
        ]);
        assert_eq!(reset.decorations[0].color, color::BLACK);
        // currentcolor is the glyphs' fill color, as in Chrome.
        let filled = one(&[
            ("WebkitTextFillColor", text("red")),
            ("textDecoration", text("underline")),
        ]);
        assert_eq!(filled.decorations[0].color, color::parse("red").unwrap());
        for (bad, message) in [
            ("underline red overline", "twice"),
            ("none underline", "is not a text-decoration-line"),
            ("underline wavy dashed", "twice"),
            ("underline from-font", "from-font is not supported yet"),
            ("underline 2", "needs a unit"),
            ("blink", "is not a color"),
        ] {
            let e = error(&[("textDecoration", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
        let e = error(&[("textDecorationThickness", num(-1.0))]);
        assert!(e.contains("is negative"), "{e}");
    }

    #[test]
    fn text_overflow_is_clip_or_an_ellipsis_of_the_box_alone() {
        let tree = vec![
            view(&[("textOverflow", text("Ellipsis"))], &[1]),
            view(&[], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        assert!(s[0].ellipsis && !s[1].ellipsis);
        assert!(!one(&[("textOverflow", text("clip"))]).ellipsis);
        let e = error(&[("textOverflow", text("'…'"))]);
        assert!(e.contains("is not one of clip, ellipsis"), "{e}");
    }

    #[test]
    fn line_clamp_needs_a_vertical_webkit_box_of_a_text() {
        let clamp = |style: &[(&str, Prop)]| {
            let node = Node {
                kind: Kind::Text {
                    content: Vec::new(),
                },
                ..view(style, &[])
            };
            styles(vec![node], &[]).map(|s| s[0].clamp)
        };
        let boxed = [
            ("display", text("-webkit-box")),
            ("WebkitBoxOrient", text("vertical")),
        ];
        let lines = |v| [&boxed[..], &[("WebkitLineClamp", v)]].concat();
        assert_eq!(clamp(&lines(num(2.0))), Ok(Some(2)));
        assert_eq!(clamp(&lines(text("3"))), Ok(Some(3)));
        assert_eq!(clamp(&lines(text("none"))), Ok(None));
        assert_eq!(clamp(&[("WebkitLineClamp", num(2.0))]), Ok(None));
        let row = [boxed[0].clone(), ("WebkitLineClamp", num(2.0))];
        assert_eq!(clamp(&row), Ok(None));
        for (bad, message) in [
            (num(0.0), "0 is not a number of lines"),
            (num(1.5), "1.5 is not an integer"),
        ] {
            let e = clamp(&lines(bad)).unwrap_err();
            assert!(e.contains(message), "{e}");
        }
        let e = error(&boxed);
        assert!(e.contains("display: -webkit-box is only for a Text"), "{e}");
    }

    #[test]
    fn text_wrap_sets_its_mode_and_style_or_refuses() {
        let wrap = |v: &str| {
            let f = one(&[("textWrap", text(v))]).font;
            (f.wrap, f.balance)
        };
        assert_eq!(wrap("balance"), (true, true));
        assert_eq!(wrap("nowrap"), (false, false));
        assert_eq!(wrap("Balance NoWrap"), (false, true));
        assert_eq!(wrap("stable"), (true, false));
        let kept = one(&[
            ("textWrapStyle", text("balance")),
            ("whiteSpace", text("nowrap")),
        ]);
        assert_eq!((kept.font.wrap, kept.font.balance), (false, true));
        for (bad, message) in [
            ("pretty", "pretty is not supported yet"),
            ("wrap nowrap", "twice"),
            ("balance auto", "twice"),
            ("even", "is not one of auto, balance"),
        ] {
            let e = error(&[("textWrap", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
    }

    #[test]
    fn text_stroke_sets_its_longhands_or_refuses() {
        let font = |style: &[(&str, Prop)]| one(style).font;
        // The keywords are in proportion to the font size, as in Chrome.
        let both = font(&[
            ("fontSize", num(24.0)),
            ("WebkitTextStroke", text("thick rgb(0 0 255)")),
        ]);
        let blue = color::parse("blue").unwrap();
        assert_eq!((both.stroke, both.stroke_color), (2.5, Some(blue)));
        // Left out of the shorthand, the width is 0 and the color currentcolor.
        let reset = font(&[
            ("WebkitTextStrokeWidth", num(2.0)),
            ("WebkitTextStrokeColor", text("red")),
            ("WebkitTextStroke", text("currentcolor")),
        ]);
        assert_eq!((reset.stroke, reset.stroke_color), (0.0, None));
        assert_eq!(font(&[("WebkitTextStroke", num(1.5))]).stroke, 1.5);
        // A decoration's currentcolor is the stroke's color where the stroke shows in a color of
        // its own, as in Chrome, and else the fill color.
        let decoration = |stroke: &str| {
            one(&[
                ("color", text("red")),
                ("WebkitTextFillColor", text("yellow")),
                ("WebkitTextStroke", text(stroke)),
                ("textDecoration", text("underline")),
            ])
            .decorations[0]
                .color
        };
        let yellow = color::parse("yellow").unwrap();
        assert_eq!(decoration("2px blue"), blue);
        for hidden in ["2px currentcolor", "2px transparent", "0 blue"] {
            assert_eq!(decoration(hidden), yellow, "{hidden}");
        }
        for (key, bad, message) in [
            ("WebkitTextStroke", text("2px red 3px"), "twice"),
            ("WebkitTextStroke", text("2 red"), "needs a unit"),
            ("WebkitTextStrokeWidth", num(-1.0), "is negative"),
            (
                "WebkitTextStrokeWidth",
                text("10%"),
                "percentages are not supported",
            ),
            ("WebkitTextStrokeColor", text("2px"), "is not a color"),
        ] {
            let e = error(&[(key, bad)]);
            assert!(e.contains(message), "{key}: {e}");
        }
    }

    #[test]
    fn text_shadows_inherit_in_px_with_currentcolor_kept() {
        let tree = vec![
            view(
                &[
                    ("fontSize", num(20.0)),
                    ("textShadow", text("0.1em 2px, -1px 0 3px blue")),
                ],
                &[1],
            ),
            view(&[("color", text("red"))], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        let blue = color::parse("blue").unwrap();
        let shadows = [
            TextShadow {
                offset: [2.0, 2.0],
                blur: 0.0,
                color: None,
            },
            TextShadow {
                offset: [-1.0, 0.0],
                blur: 3.0,
                color: Some(blue),
            },
        ];
        assert_eq!(&s[0].font.shadows[..], shadows);
        assert_eq!(s[1].font.shadows, s[0].font.shadows);
        for (bad, message) in [
            ("1px 1px 1px 1px", "is not a text shadow"),
            ("inset 1px 1px", "is not a text shadow"),
            ("1px", "needs two to four lengths"),
            ("1px 1px -2px", "is negative"),
        ] {
            let e = error(&[("textShadow", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
        assert!(one(&[("textShadow", text("none"))]).font.shadows.is_empty());
    }

    #[test]
    fn font_features_are_quoted_tags_with_their_values() {
        let features = |v: &str| {
            let f = &one(&[("fontFeatureSettings", text(v))]).font.features;
            (f.iter())
                .map(|f| (std::str::from_utf8(&f.tag).unwrap().to_owned(), f.value))
                .collect::<Vec<_>>()
        };
        let feature = |tag: &str, value| (tag.to_owned(), value);
        assert_eq!(
            features("'liga' OFF, \"ss03\", 'salt' 2, 'Kern' on"),
            [
                feature("liga", 0),
                feature("ss03", 1),
                feature("salt", 2),
                feature("Kern", 1),
            ]
        );
        assert_eq!(features("Normal"), []);
        for (bad, message) in [
            ("liga", "a tag of 4 characters, quoted"),
            ("'lig'", "a tag of 4 characters, quoted"),
            ("'liga' -1", "-1 is not a feature value"),
            ("'liga' 1.5", "1.5 is not a feature value"),
            ("'liga' 70000", "70000 is over 65535"),
            ("'liga' 1 2", "is not a quoted tag and its value"),
            ("'liga',", "is not a quoted tag and its value"),
        ] {
            let e = error(&[("fontFeatureSettings", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
    }

    #[test]
    fn font_style_is_italic_or_oblique_by_an_angle() {
        let style = |v: &str| one(&[("fontStyle", text(v))]).font.style;
        assert_eq!(style("Italic"), FontStyle::Italic);
        assert_eq!(style("oblique"), FontStyle::Oblique(14.0));
        assert_eq!(style("oblique -0.25turn"), FontStyle::Oblique(-90.0));
        assert_eq!(style("normal"), FontStyle::Normal);
        // Chrome slants a face for italic, and for oblique by 14deg or more.
        let slants = |v: &str| style(v).slants();
        assert!(slants("italic") && slants("oblique") && slants("oblique 90deg"));
        assert!(!slants("oblique 13deg") && !slants("oblique -20deg") && !slants("normal"));
        for (bad, message) in [
            (
                "oblique 91deg",
                "91deg is not an angle from -90deg to 90deg",
            ),
            ("oblique 10", "10 is not an angle"),
            (
                "italic 10deg",
                "is not normal, italic, or oblique and an angle",
            ),
            ("slanted", "is not normal, italic, or oblique and an angle"),
        ] {
            let e = error(&[("fontStyle", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
    }

    #[test]
    fn font_stretch_is_a_keyword_or_a_percentage() {
        let stretch = |v: &str| one(&[("fontStretch", text(v))]).font.stretch;
        assert_eq!(stretch("Semi-Condensed"), 87.5);
        assert_eq!(stretch("ultra-expanded"), 200.0);
        assert_eq!(stretch("62.5%"), 62.5);
        assert_eq!(stretch("0%"), 0.0);
        for bad in ["-10%", "75", "wide"] {
            let e = error(&[("fontStretch", text(bad))]);
            assert!(e.contains("is not a percentage, 0% or more"), "{bad}: {e}");
        }
    }

    #[test]
    fn font_variations_are_quoted_axes_with_numbers() {
        let variations = |v: &str| {
            let f = &one(&[("fontVariationSettings", text(v))]).font.variations;
            (f.iter())
                .map(|v| (std::str::from_utf8(&v.tag).unwrap().to_owned(), v.value))
                .collect::<Vec<_>>()
        };
        let axis = |tag: &str, value| (tag.to_owned(), value);
        assert_eq!(
            variations("'wght' 650, \"wdth\" 75.5, 'XOPQ' -0"),
            [axis("wght", 650.0), axis("wdth", 75.5), axis("XOPQ", 0.0)]
        );
        assert!(variations("'XOPQ' -0")[0].1.is_sign_positive());
        assert_eq!(variations("normal"), []);
        for (bad, message) in [
            ("'wght'", "an axis takes a number after its tag"),
            ("'wght' bold", "bold is not an axis value"),
            ("'wght' 10px", "10px is not an axis value"),
            (
                "wght 400",
                "does not start with a tag of 4 characters, quoted",
            ),
            ("'wght' 1 2", "is not a quoted tag and its value"),
        ] {
            let e = error(&[("fontVariationSettings", text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
    }

    #[test]
    fn font_variants_set_their_features_one_longhand_at_a_time() {
        let variants = |style: &[(&str, Prop)]| {
            let f = one(style).font;
            (f.variants.iter())
                .map(|v| {
                    (v.iter())
                        .map(|f| format!("{}={}", std::str::from_utf8(&f.tag).unwrap(), f.value))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            variants(&[("fontVariant", text("Small-Caps tabular-nums ordinal sub"))]),
            ["", "", "smcp=1", "tnum=1 ordn=1", "", "", "subs=1"]
        );
        assert_eq!(
            variants(&[
                (
                    "fontVariantNumeric",
                    text("oldstyle-nums diagonal-fractions")
                ),
                (
                    "fontVariantLigatures",
                    text("no-common-ligatures contextual")
                ),
                ("fontVariantCaps", text("all-small-caps")),
                ("fontVariantEastAsian", text("jis04 full-width ruby")),
                ("fontVariantAlternates", text("historical-forms")),
                ("fontVariantPosition", text("super")),
                ("fontKerning", text("none")),
            ]),
            [
                "kern=0",
                "liga=0 clig=0 calt=1",
                "c2sc=1 smcp=1",
                "onum=1 frac=1",
                "jp04=1 fwid=1 ruby=1",
                "hist=1",
                "sups=1",
            ]
        );
        // The shorthand sets every longhand but kerning; none turns ligatures off.
        assert_eq!(
            variants(&[
                ("fontKerning", text("none")),
                ("fontVariantCaps", text("small-caps")),
                ("fontVariant", text("none")),
            ]),
            [
                "kern=0",
                "liga=0 clig=0 dlig=0 hlig=0 calt=0",
                "",
                "",
                "",
                "",
                ""
            ]
        );
        assert_eq!(
            variants(&[("fontKerning", text("normal"))]),
            variants(&[("fontKerning", text("auto"))])
        );
        for (key, bad, message) in [
            (
                "fontVariantCaps",
                "petite-caps",
                "petite-caps is not supported yet",
            ),
            ("fontVariant", "unicase", "unicase is not supported yet"),
            (
                "fontVariantAlternates",
                "swash(fancy)",
                "needs @font-feature-values",
            ),
            (
                "fontVariantNumeric",
                "lining-nums oldstyle-nums",
                "set the same: give one",
            ),
            (
                "fontVariantNumeric",
                "ordinal ordinal",
                "set the same: give one",
            ),
            (
                "fontVariantCaps",
                "none",
                "none is not one of normal, small-caps",
            ),
            (
                "fontVariant",
                "none small-caps",
                "none is not a keyword of a font-variant",
            ),
            ("fontKerning", "on", "on is not one of normal, auto, none"),
        ] {
            let e = error(&[(key, text(bad))]);
            assert!(e.contains(message), "{bad}: {e}");
        }
    }

    #[test]
    fn text_properties_inherit() {
        let tree = vec![
            view(&[], &[1, 4]),
            view(
                &[
                    ("fontSize", num(20.0)),
                    ("color", text("RED")),
                    ("fontFamily", text("Brand Sans, 'Noto Serif', serif")),
                    ("lineHeight", num(1.5)),
                    ("letterSpacing", text("0.1em")),
                    ("fontWeight", text("bold")),
                    ("textAlign", text("center")),
                    ("wordSpacing", text("0.25em")),
                    ("whiteSpace", text("pre-line")),
                    ("textTransform", text("uppercase")),
                    ("wordBreak", text("break-all")),
                    ("wordWrap", text("break-word")),
                    ("textWrapStyle", text("balance")),
                    ("WebkitTextFillColor", text("blue")),
                    ("WebkitTextStroke", text("0.1em #0f0")),
                    ("fontFeatureSettings", text("'smcp', \"tnum\" 1")),
                    ("fontKerning", text("none")),
                    ("fontVariant", text("small-caps slashed-zero")),
                    ("fontVariationSettings", text("'wdth' 80")),
                    ("fontStretch", text("condensed")),
                    ("fontStyle", text("oblique 20deg")),
                ],
                &[2],
            ),
            view(
                &[("fontSize", text("2em")), ("fontWeight", text("bolder"))],
                &[3],
            ),
            view(
                &[
                    ("fontSize", text("50%")),
                    ("lineHeight", text("150%")),
                    ("fontWeight", text("lighter")),
                ],
                &[],
            ),
            view(&[], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        let feature = |tag: &[u8; 4], value| Feature { tag: *tag, value };
        let initial = Font {
            size: 16.0,
            weight: 400.0,
            stretch: 100.0,
            style: FontStyle::Normal,
            family: Rc::new([Family::SansSerif]),
            line_height: LineHeight::Normal,
            align: TextAlign::Start,
            indent: None,
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
        };
        assert_eq!(*s[0].font, initial);
        assert_eq!(s[0].color, color::BLACK);
        assert_eq!(*s[4].font, initial);

        let brand = Font {
            size: 20.0,
            weight: 700.0,
            stretch: 75.0,
            style: FontStyle::Oblique(20.0),
            family: Rc::new([
                Family::Named("Brand Sans".into()),
                Family::Named("Noto Serif".into()),
                Family::Serif,
            ]),
            line_height: LineHeight::Factor(1.5),
            align: TextAlign::Center,
            indent: None,
            spacing: 2.0,
            word_spacing: 5.0,
            collapse: Collapse::PreserveBreaks,
            wrap: true,
            balance: true,
            word_break: WordBreak::BreakAll,
            overflow_wrap: OverflowWrap::BreakWord,
            transform: TextTransform::Uppercase,
            variants: [
                Rc::new([feature(b"kern", 0)]),
                Rc::new([]),
                Rc::new([feature(b"smcp", 1)]),
                Rc::new([feature(b"zero", 1)]),
                Rc::new([]),
                Rc::new([]),
                Rc::new([]),
            ],
            features: Rc::new([
                Feature {
                    tag: *b"smcp",
                    value: 1,
                },
                Feature {
                    tag: *b"tnum",
                    value: 1,
                },
            ]),
            variations: Rc::new([Variation {
                tag: *b"wdth",
                value: 80.0,
            }]),
            fill: Some(color::parse("blue").unwrap()),
            stroke: 2.0,
            stroke_color: Some(color::parse("#0f0").unwrap()),
            shadows: Rc::new([]),
            skip_ink: true,
            underline_offset: None,
        };
        assert_eq!(*s[1].font, brand);
        assert_eq!(s[1].color, [1.0, 0.0, 0.0, 1.0]);
        // Spacing inherits as px; a factor line height inherits as the factor.
        let bigger = Font {
            size: 40.0,
            weight: 900.0,
            ..brand.clone()
        };
        assert_eq!(*s[2].font, bigger);
        assert_eq!(s[2].color, s[1].color);
        let smaller = Font {
            size: 20.0,
            weight: 700.0,
            line_height: LineHeight::Px(30.0),
            ..brand
        };
        assert_eq!(*s[3].font, smaller);
    }
}
