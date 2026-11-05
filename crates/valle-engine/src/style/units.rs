//! CSS lengths and their conversion to layout units.

use std::rc::Rc;

use taffy::{Dimension, LengthPercentage, LengthPercentageAuto};

use super::declaration::{LARGEST, MIN_SIZE, ZERO, shown, text, values};
use super::math::{Calc, Math};
use super::{css, math};

use crate::template::V;

/// The root font size: `rem`.
const REM: f64 = 16.0;

/// A length as declared; `em` resolves when the style is computed.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Len {
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
pub(super) enum Sizing {
    MinContent,
    MaxContent,
    /// `fit-content`: the max-content size, but no wider than the box would stretch, and no
    /// narrower than the min-content size.
    FitContent,
    Stretch,
    /// `flex-basis: content`.
    Content,
}

/// What lengths resolve against besides their percentage base: the font size and the canvas, in
/// px.
#[derive(Clone, Copy)]
pub(super) struct Units {
    pub(super) em: f64,
    pub(super) canvas: [f64; 2],
}

impl Len {
    /// As CSS clamps what a math function comes to: an infinity to the largest length, and NaN
    /// to 0.
    pub(super) fn finite(&self) -> Len {
        let f = |v: f64| match v.is_nan() {
            true => 0.0,
            false => v.clamp(-LARGEST, LARGEST),
        };
        match *self {
            Len::Px(v) => Len::Px(f(v)),
            Len::Percent(v) => Len::Percent(f(v)),
            Len::Em(v) => Len::Em(f(v)),
            Len::Vw(v) => Len::Vw(f(v)),
            Len::Vh(v) => Len::Vh(f(v)),
            Len::Vmin(v) => Len::Vmin(f(v)),
            Len::Vmax(v) => Len::Vmax(f(v)),
            ref l => l.clone(),
        }
    }

    /// Whether it is below 0; a math function is not, and clamps to 0 where it has to.
    pub(super) fn negative(&self) -> bool {
        match *self {
            Len::Auto | Len::Calc(_) | Len::Sizing(_) => false,
            Len::Px(v) | Len::Percent(v) | Len::Em(v) => v < 0.0,
            Len::Vw(v) | Len::Vh(v) | Len::Vmin(v) | Len::Vmax(v) => v < 0.0,
        }
    }

    /// px, against `base` for percentages; the keywords are 0.
    pub(super) fn px(&self, u: Units, base: f64) -> f64 {
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
    pub(super) fn calc(&self, u: Units) -> Calc {
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
    pub(super) fn scaled(&self, k: f64) -> Len {
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
pub(super) struct Lengths {
    pub(super) u: Units,
    pub(super) first: usize,
    pub(super) calcs: Vec<Calc>,
}

impl Lengths {
    /// `l` made by Taffy's constructors for px, percentages and calc handles; `auto` is the
    /// caller's.
    pub(super) fn taffy<T>(
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

    pub(super) fn lpa(&mut self, l: &Len) -> LengthPercentageAuto {
        type T = LengthPercentageAuto;
        match l {
            Len::Auto => T::auto(),
            l => self.taffy(l, T::length, T::percent, T::calc),
        }
    }

    pub(super) fn lp(&mut self, l: &Len) -> LengthPercentage {
        type T = LengthPercentage;
        self.taffy(l, T::length, T::percent, T::calc)
    }

    pub(super) fn dim(&mut self, l: &Len) -> Dimension {
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
    pub(super) fn rect<T>(
        &mut self,
        s: &[Len; 4],
        f: fn(&mut Lengths, &Len) -> T,
    ) -> taffy::Rect<T> {
        taffy::Rect {
            top: f(self, &s[0]),
            right: f(self, &s[1]),
            bottom: f(self, &s[2]),
            left: f(self, &s[3]),
        }
    }
}

/// What a length may be.
#[derive(Clone, Copy)]
pub(super) struct Accept {
    /// The keyword that is `Len::Auto`.
    pub(super) keyword: Option<&'static str>,
    pub(super) negative: bool,
    pub(super) percent: bool,
    /// The sizing keywords it takes.
    pub(super) sizing: &'static [&'static str],
}

/// The sizing keywords of `width` and `height`.
const SIZING: &[&str] = &["min-content", "max-content", "fit-content", "stretch"];

pub(super) const INSET: Accept = Accept {
    keyword: Some("auto"),
    negative: true,
    percent: true,
    sizing: &[],
};

pub(super) const SIZE: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
    sizing: SIZING,
};

pub(super) const MAX_SIZE: Accept = Accept {
    keyword: Some("none"),
    ..MIN_SIZE
};

pub(super) const PADDING: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};

pub(super) const GAP: Accept = Accept {
    keyword: Some("normal"),
    negative: false,
    percent: true,
    sizing: &[],
};

pub(super) const RADIUS: Accept = Accept {
    keyword: None,
    negative: false,
    percent: true,
    sizing: &[],
};

/// `border-width`.
pub(super) const WIDTH: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
    sizing: &[],
};

/// `box-shadow` blurs.
pub(super) const BLUR: Accept = Accept {
    keyword: None,
    negative: false,
    percent: false,
    sizing: &[],
};

/// `outline-offset`, and `box-shadow` offsets and spreads.
pub(super) const OFFSET: Accept = Accept {
    keyword: None,
    negative: true,
    percent: false,
    sizing: &[],
};

pub(super) const SPACING: Accept = Accept {
    keyword: Some("normal"),
    negative: true,
    percent: false,
    sizing: &[],
};

pub(super) const THICKNESS: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
    sizing: &[],
};

pub(super) const INDENT: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

pub(super) const UNDERLINE_OFFSET: Accept = Accept {
    keyword: Some("auto"),
    negative: true,
    percent: true,
    sizing: &[],
};

/// An SVG length: a CSS length or percentage, or a number of px, as SVG 2 takes in its
/// properties.
pub(super) fn svg_length(v: &V, negative: bool) -> Result<Len, String> {
    let a = Accept {
        keyword: None,
        negative,
        percent: true,
        sizing: &[],
    };
    match v {
        V::Str(_) => match css::num(&text(v)?) {
            Some(n) if n.unit.is_empty() => length(&V::Num(n.value), a),
            _ => length(v, a),
        },
        _ => length(v, a),
    }
}

pub(super) fn length_text(s: &str, a: Accept) -> Result<Len, String> {
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
pub(super) fn sizing(s: &str, a: Accept) -> Result<Option<Sizing>, String> {
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
pub(super) fn unit(n: css::Num, a: Accept) -> Result<Len, String> {
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
pub(super) fn viewport(unit: &str) -> Option<fn(f64) -> Len> {
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
pub(super) fn length(v: &V, a: Accept) -> Result<Len, String> {
    match v {
        V::Num(x) if !x.is_finite() => Err(format!("{x} is not a finite length")),
        V::Num(x) if !a.negative && *x < 0.0 => Err(format!("{x} is negative")),
        V::Num(x) => Ok(Len::Px(*x)),
        V::Bool(_) => Err(format!("{} is not a length", shown(v))),
        V::Str(_) => length_text(&text(v)?, a),
    }
}

/// The lengths of a shorthand: one to four for the sides of a box, one or two for gaps.
pub(super) fn lengths(v: &V, a: Accept, most: usize) -> Result<Vec<Len>, String> {
    values(v, most, "lengths", |v| length(v, a))
}

#[cfg(test)]
mod tests {

    use taffy::{Dimension, LengthPercentageAuto};

    use super::*;

    use crate::testing::styles::{auto_px, error, one, px, text};

    #[test]
    fn viewport_units_take_the_canvas() {
        let l = one(&[
            ("width", text("10vw")),
            ("height", text("10vh")),
            ("paddingLeft", text("10vmin")),
            ("paddingRight", text("10vmax")),
            ("paddingTop", text("10dvh")),
            ("paddingBottom", text("10svi")),
            ("marginLeft", text("-5lvb")),
        ])
        .layout;
        assert_eq!(l.size.width, Dimension::length(20.0));
        assert_eq!(l.size.height, Dimension::length(10.0));
        let padding = taffy::Rect {
            top: px(10.0),
            right: px(20.0),
            bottom: px(20.0),
            left: px(10.0),
        };
        assert_eq!(l.padding, padding);
        assert_eq!(l.margin.left, auto_px(-5.0));
        assert_eq!(one(&[("fontSize", text("10vw"))]).font.size, 20.0);
        assert!(
            error(&[("width", text("1vq"))])
                .ends_with("1vq: vq is not supported; use px, em, rem, %, vw, vh, vmin or vmax")
        );
    }

    #[test]
    fn math_constants_and_infinite_lengths() {
        let s = one(&[
            ("width", text("calc(infinity * 1px)")),
            ("height", text("calc(pi * 10px)")),
        ]);
        let l = &s.layout;
        // An infinity is the largest length.
        assert_eq!(l.size.width, Dimension::length(33_554_431.0));
        assert_eq!(
            l.size.height,
            Dimension::length((std::f64::consts::PI * 10.0) as f32)
        );
    }

    #[test]
    fn math_functions_are_px_or_calcs_for_layout() {
        let s = one(&[
            ("fontSize", text("calc(1em + 4px)")),
            ("width", text("calc(100% - 2em)")),
            ("height", text("calc(2 * (1vw + 3px))")),
            ("minWidth", text("clamp(10px, 50%, 2em)")),
            ("paddingLeft", text("calc(10px - 20px)")),
            ("marginLeft", text("calc(10px - 20px)")),
        ]);
        assert_eq!(s.font.size, 20.0);
        let l = &s.layout;
        // With a percentage in it, a calc layout resolves; it clamps at 0 where lengths have to.
        assert_eq!(l.size.width, Dimension::calc(math::handle(0)));
        assert_eq!(
            l.min_size.width,
            LengthPercentageAuto::calc(math::handle(1))
        );
        let calcs = &s.calcs;
        let [width, min] = [0, 1].map(|i| move |basis| calcs[i].resolve(basis));
        assert_eq!((width(200.0), width(10.0)), (160.0, 0.0));
        assert_eq!((min(200.0), min(60.0), min(10.0)), (40.0, 30.0, 10.0));
        // Without one, it is px.
        assert_eq!(l.size.height, Dimension::length(10.0));
        assert_eq!((l.padding.left, l.margin.left), (px(0.0), auto_px(-10.0)));
        for (bad, e) in [
            ("calc(1px+2px)", "+ and - need white space around them"),
            ("calc(1px * 2px)", "two lengths do not multiply"),
            ("calc(1px / 0)", "it divides by 0"),
            ("calc(1 + 1px)", "a number and a length do not add"),
            ("min(1px, 2)", "numbers and lengths do not compare"),
            ("calc(2)", "calc(2) is a number, not a length"),
            ("calc(1px, 2px)", "calc() takes one sum"),
            (
                "clamp(1px, 2px)",
                "clamp() takes a minimum, a value and a maximum",
            ),
            ("calc((1px)", "it ends too soon"),
            ("calc(1px) 2px", "calc(1px) 2px is not a length"),
            (
                "calc(1in)",
                "in is not supported; use px, em, rem, %, vw, vh, vmin or vmax",
            ),
        ] {
            let got = error(&[("width", text(bad))]);
            assert!(
                got.starts_with("s.tsx:1:1: style width: ") && got.ends_with(e),
                "{got}"
            );
        }
        let spacing = error(&[("letterSpacing", text("calc(1% + 1px)"))]);
        assert!(spacing.ends_with("percentages are not supported here"));
    }

    #[test]
    fn sizing_keywords_go_to_taffy() {
        let l = one(&[
            ("width", text("fit-content")),
            ("height", text("max-content")),
            ("flexBasis", text("content")),
        ])
        .layout;
        assert_eq!(l.size.width, Dimension::fit_content());
        assert_eq!(l.size.height, Dimension::max_content());
        assert_eq!(l.flex_basis, Dimension::content());
        let stretch = one(&[("width", text("Stretch"))]).layout.size.width;
        assert_eq!(stretch, Dimension::stretch());
        let e = error(&[("minWidth", text("min-content"))]);
        assert!(e.ends_with(
            "min-content: only width, height and flex-basis take sizing keywords so far"
        ));
        assert!(error(&[("width", text("content"))]).ends_with("content is not a length"));
        let e = error(&[("width", text("fit-content(30%)"))]);
        assert!(e.ends_with(
            "browsers take fit-content() only for grid tracks; use fit-content and max-width"
        ));
    }
}
