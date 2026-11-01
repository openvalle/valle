//! Filters: `filter`, functions applied one after another to all a box paints, as one image.

use super::color::Rgba;
use super::css::{self, angle};
use super::math;
use super::{Len, Shadowed, Units, shadow};

/// A filter function; amounts are numbers, 1 for 100 %.
#[derive(Clone, Debug, PartialEq)]
pub enum Function<L = f64, C = Rgba> {
    /// A Gaussian blur of this standard deviation, px.
    Blur(L),
    Brightness(f64),
    Contrast(f64),
    /// Within [0, 1], as invert, opacity and sepia are.
    Grayscale(f64),
    /// Degrees.
    HueRotate(f64),
    Invert(f64),
    Opacity(f64),
    Saturate(f64),
    Sepia(f64),
    /// The image's shape in a color, moved, blurred by a standard deviation, under it. A color of
    /// `None` is `currentcolor`.
    DropShadow {
        offset: [L; 2],
        blur: L,
        color: C,
    },
}

impl Function<Len, Option<Rgba>> {
    pub(super) fn computed(&self, u: Units, current: Rgba) -> Function {
        match self {
            Function::Blur(l) => Function::Blur(l.px(u, 0.0)),
            Function::Brightness(a) => Function::Brightness(*a),
            Function::Contrast(a) => Function::Contrast(*a),
            Function::Grayscale(a) => Function::Grayscale(*a),
            Function::HueRotate(a) => Function::HueRotate(*a),
            Function::Invert(a) => Function::Invert(*a),
            Function::Opacity(a) => Function::Opacity(*a),
            Function::Saturate(a) => Function::Saturate(*a),
            Function::Sepia(a) => Function::Sepia(*a),
            Function::DropShadow {
                offset,
                blur,
                color,
            } => Function::DropShadow {
                offset: offset.each_ref().map(|l| l.px(u, 0.0)),
                blur: blur.px(u, 0.0),
                color: color.unwrap_or(current),
            },
        }
    }
}

const NAMES: &str = "blur, brightness, contrast, drop-shadow, grayscale, hue-rotate, invert, \
                     opacity, saturate or sepia";

/// `filter`: `none`, or functions separated by spaces.
pub(super) fn filter(s: &str) -> Result<Vec<Function<Len, Option<Rgba>>>, String> {
    match s {
        "none" => Ok(Vec::new()),
        "" => Err("\"\" is not a filter; use none".into()),
        _ => css::words(s).into_iter().map(function).collect(),
    }
}

fn function(w: &str) -> Result<Function<Len, Option<Rgba>>, String> {
    let Some((name, inner)) = css::function(w) else {
        return Err(format!("{w} is not a filter function: {NAMES}"));
    };
    let named = |e: String| format!("{name}(): {e}");
    let args = css::words(inner);
    // A number or a percentage, not negative, 1 when left out; at most 1 where `most`.
    let amount = |most: bool| -> Result<f64, String> {
        let a = match args[..] {
            [] => 1.0,
            [a] => match css::num(a) {
                Some(n) if n.unit.is_empty() => n.value,
                Some(n) if n.unit == "%" => n.value / 100.0,
                _ => return Err(named(format!("{a} is not a number or a percentage"))),
            },
            _ => return Err(named("takes one number or percentage".into())),
        };
        match a < 0.0 {
            true => Err(named(format!("{a} is negative"))),
            false if most => Ok(a.min(1.0)),
            false => Ok(a),
        }
    };
    Ok(match name {
        "blur" => Function::Blur(match args[..] {
            [] => Len::Px(0.0),
            [l] => super::length_text(l, super::BLUR).map_err(named)?,
            _ => return Err(named("takes one length".into())),
        }),
        "brightness" => Function::Brightness(amount(false)?),
        "contrast" => Function::Contrast(amount(false)?),
        "grayscale" => Function::Grayscale(amount(true)?),
        "invert" => Function::Invert(amount(true)?),
        "opacity" => Function::Opacity(amount(true)?),
        "saturate" => Function::Saturate(amount(false)?),
        "sepia" => Function::Sepia(amount(true)?),
        "hue-rotate" => Function::HueRotate(match args[..] {
            [] => 0.0,
            [a] => angle(a).ok_or_else(|| named(format!("{a} is not an angle")))?,
            _ => return Err(named("takes one angle".into())),
        }),
        "drop-shadow" => {
            let lengths = args
                .iter()
                .filter(|w| css::num(w).is_some() || math::is_math(w));
            if !(2..=3).contains(&lengths.count()) || args.contains(&"inset") {
                return Err(named(
                    "takes a color and two or three lengths: x, y and a standard deviation".into(),
                ));
            }
            let Shadowed {
                offset,
                blur,
                color,
                ..
            } = shadow(inner, false).map_err(named)?;
            Function::DropShadow {
                offset,
                blur,
                color,
            }
        }
        _ => return Err(format!("{name}() is not a filter function: {NAMES}")),
    })
}
