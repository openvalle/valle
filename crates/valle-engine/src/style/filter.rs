//! Filters: `filter`, functions applied one after another to all a box paints, as one image:
//! CSS's, and Valle's for video, whose lengths are px in the box's own px.

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
    /// Valle's `chromatic-aberration()`: red moved on along x by half this many px and blue
    /// back by half, as a lens splits colors.
    ChromaticAberration(f64),
    /// Valle's `lens-distortion(k1 k2)`: each point of the border box shows the one as much
    /// farther from its center as 1 + k1 r² + k2 r⁴, r its distance over the half diagonal's;
    /// barrel for more than 0 and pincushion for less.
    LensDistortion([f64; 2]),
    /// Valle's `film-grain(seed amount size)`: noise of a pattern the seed picks, cells `size`
    /// px across from the border box's top left, added to the colors' encoded values by up to
    /// `amount` of how far each is from 0 or 1, whichever is nearer.
    FilmGrain {
        seed: u32,
        amount: f64,
        size: f64,
    },
    /// Valle's `radial-blur(x y amount)`: blurred along each ray from the point `center` px
    /// from the border box's top left, by up to `amount` px either way and never more than half
    /// of the way to the point.
    RadialBlur {
        center: [f64; 2],
        amount: f64,
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
            Function::ChromaticAberration(offset) => Function::ChromaticAberration(*offset),
            Function::LensDistortion(k) => Function::LensDistortion(*k),
            Function::FilmGrain { seed, amount, size } => Function::FilmGrain {
                seed: *seed,
                amount: *amount,
                size: *size,
            },
            Function::RadialBlur { center, amount } => Function::RadialBlur {
                center: *center,
                amount: *amount,
            },
        }
    }
}

const NAMES: &str = "blur, brightness, contrast, drop-shadow, grayscale, hue-rotate, invert, \
                     opacity, saturate or sepia, or Valle's chromatic-aberration, \
                     film-grain, lens-distortion or radial-blur";

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
    // Valle's parameters, each a number, or px where `px`, within a range.
    let valle = |takes: &[(&str, bool, [f64; 2])]| -> Result<Vec<f64>, String> {
        if args.len() != takes.len() {
            let names: Vec<&str> = takes.iter().map(|t| t.0).collect();
            return Err(named(format!("takes {}", names.join(", "))));
        }
        (args.iter().zip(takes))
            .map(|(a, &(what, px, [low, high]))| {
                let n = css::num(a).filter(|n| match px {
                    true => n.unit == "px" || (n.unit.is_empty() && n.value == 0.0),
                    false => n.unit.is_empty(),
                });
                let unit = if px { "px" } else { "" };
                match n {
                    Some(n) if (low..=high).contains(&n.value) => Ok(n.value),
                    Some(_) => Err(named(format!(
                        "{what} {a} is not within {low}{unit} and {high}{unit}"
                    ))),
                    None if px => Err(named(format!("{what} {a} is not px"))),
                    None => Err(named(format!("{what} {a} is not a number"))),
                }
            })
            .collect()
    };
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
        "chromatic-aberration" => {
            let a = valle(&[("the offset", true, [-256.0, 256.0])])?;
            Function::ChromaticAberration(a[0])
        }
        "lens-distortion" => {
            let a = valle(&[("k1", false, [-0.5, 0.5]), ("k2", false, [-0.5, 0.5])])?;
            Function::LensDistortion([a[0], a[1]])
        }
        "film-grain" => {
            let a = valle(&[
                ("the seed", false, [0.0, f64::from(u32::MAX)]),
                ("the amount", false, [0.0, 1.0]),
                ("the size", true, [1.0, 64.0]),
            ])?;
            if a[0].fract() != 0.0 {
                return Err(named(format!("the seed {} is not a whole number", a[0])));
            }
            Function::FilmGrain {
                seed: a[0] as u32,
                amount: a[1],
                size: a[2],
            }
        }
        "radial-blur" => {
            let a = valle(&[
                ("x", true, [-1e6, 1e6]),
                ("y", true, [-1e6, 1e6]),
                ("the amount", true, [0.0, 128.0]),
            ])?;
            Function::RadialBlur {
                center: [a[0], a[1]],
                amount: a[2],
            }
        }
        _ => return Err(format!("{name}() is not a filter function: {NAMES}")),
    })
}
