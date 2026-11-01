//! Backgrounds: a color under layers of images, each sized, placed and repeated in its box. The
//! properties of the layers are lists, each repeated for as many images as there are.

use super::color::{self, Rgba};
use super::gradient::{self, Image, image};
use super::math::Calc;
use super::position::{Offset, position};
use super::{Accept, Len, NO_SCROLL, NO_SCROLL_INSTEAD, Units, css, length_text, math};

/// A background image, how big it is, where it is, and how it repeats.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub image: Image,
    pub size: ImageSize,
    pub position: [Offset; 2],
    pub repeat: [Repeat; 2],
}

/// `background-size`.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageSize<P = Calc> {
    /// The largest that fits in the area, keeping the image's proportions.
    Contain,
    /// The smallest that covers the area, keeping the image's proportions.
    Cover,
    /// Width and height; `None` is `auto`.
    Sized([Option<P>; 2]),
}

/// How an image repeats on an axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    Repeat,
    /// As many whole images as fit, spread out to fill the area.
    Space,
    /// As many as are nearest to fitting, stretched or shrunk to fill it.
    Round,
    NoRepeat,
}

/// The background properties as declared: each a list, `positions` one for x and one for y.
#[derive(Clone, Debug, Default)]
pub(super) struct Declared {
    pub images: Vec<gradient::Declared>,
    pub sizes: Vec<ImageSize<Len>>,
    pub positions: [Vec<Offset<Len>>; 2],
    pub repeats: Vec<[Repeat; 2]>,
}

impl Declared {
    /// A layer for each image, its other properties from their lists, repeated as needed.
    pub(super) fn computed(&self, u: Units, current: Rgba) -> Vec<Layer> {
        let start = || Offset {
            end: false,
            by: Len::Percent(0.0),
        };
        (self.images.iter().enumerate())
            .map(|(i, image)| Layer {
                image: image.computed(u, current),
                size: match nth(&self.sizes, i).unwrap_or(ImageSize::Sized([None, None])) {
                    ImageSize::Contain => ImageSize::Contain,
                    ImageSize::Cover => ImageSize::Cover,
                    ImageSize::Sized(s) => {
                        ImageSize::Sized(s.each_ref().map(|l| l.as_ref().map(|l| l.calc(u))))
                    }
                },
                position: (self.positions.each_ref())
                    .map(|list| nth(list, i).unwrap_or_else(start).computed(u)),
                repeat: nth(&self.repeats, i).unwrap_or([Repeat::Repeat; 2]),
            })
            .collect()
    }
}

/// Item `i` of a list repeated as often as needed, if it has any.
fn nth<T: Clone>(list: &[T], i: usize) -> Option<T> {
    (!list.is_empty()).then(|| list[i % list.len()].clone())
}

/// Image sizes: px or percentages of the area.
const SIZE: Accept = Accept {
    keyword: Some("auto"),
    negative: false,
    percent: true,
    sizing: &[],
};
/// Image positions on one axis.
const OFFSET: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

/// A list of values separated by commas.
fn list<T>(s: &str, what: &str, one: impl Fn(&str) -> Result<T, String>) -> Result<Vec<T>, String> {
    match s {
        "" => Err(format!("\"\" is not {what}")),
        _ => css::commas(s).into_iter().map(one).collect(),
    }
}

/// `background-image`: images, the first on top.
pub(super) fn images(s: &str) -> Result<Vec<gradient::Declared>, String> {
    list(s, "an image; use none", image)
}

/// `background-size`.
pub(super) fn sizes(s: &str) -> Result<Vec<ImageSize<Len>>, String> {
    list(s, "a size; use auto", |s| size(&css::words(s)))
}

/// `cover`, `contain`, or a width and a height, either `auto`, the height `auto` when left out.
fn size(words: &[&str]) -> Result<ImageSize<Len>, String> {
    let length = |w: &str| -> Result<Option<Len>, String> {
        Ok(match length_text(w, SIZE)? {
            Len::Auto => None,
            l => Some(l),
        })
    };
    Ok(match words {
        ["cover"] => ImageSize::Cover,
        ["contain"] => ImageSize::Contain,
        [w] => ImageSize::Sized([length(w)?, None]),
        [w, h] => ImageSize::Sized([length(w)?, length(h)?]),
        _ => {
            return Err(format!(
                "{:?} is not a size: cover, contain, or one or two lengths or auto",
                words.join(" ")
            ));
        }
    })
}

/// `background-position`: positions of one to four words.
pub(super) fn positions(s: &str) -> Result<[Vec<Offset<Len>>; 2], String> {
    let both = list(s, "a position", |s| position(&css::words(s), true))?;
    let (x, y) = both.into_iter().map(|[x, y]| (x, y)).unzip();
    Ok([x, y])
}

/// `background-position-x` or `-y`: on axis 0 or 1, center, or a side, a length from the start,
/// or both.
pub(super) fn axis_positions(s: &str, axis: usize) -> Result<Vec<Offset<Len>>, String> {
    let [start, end] = [["left", "right"], ["top", "bottom"]][axis];
    list(s, "a position", |s| {
        let from = |end, by| Offset { end, by };
        let length = |w| length_text(w, OFFSET);
        Ok(match css::words(s)[..] {
            ["center"] => from(false, Len::Percent(50.0)),
            [side] if side == start || side == end => from(side == end, Len::Percent(0.0)),
            [side, by] if side == start || side == end => from(side == end, length(by)?),
            [by] if css::num(by).is_some() || math::is_math(by) => from(false, length(by)?),
            _ => {
                return Err(format!(
                    "{s:?} is not a position: center, {start} or {end}, a length, or a side and \
                     a length"
                ));
            }
        })
    })
}

/// `background-repeat`: `repeat-x`, `repeat-y`, or one or two of repeat, space, round and
/// no-repeat, x then y.
pub(super) fn repeats(s: &str) -> Result<Vec<[Repeat; 2]>, String> {
    list(s, "a repeat", |s| repeat(&css::words(s)))
}

fn repeat(words: &[&str]) -> Result<[Repeat; 2], String> {
    let one = |w: &str| match w {
        "repeat" => Some(Repeat::Repeat),
        "space" => Some(Repeat::Space),
        "round" => Some(Repeat::Round),
        "no-repeat" => Some(Repeat::NoRepeat),
        _ => None,
    };
    let r = match words {
        ["repeat-x"] => Some([Repeat::Repeat, Repeat::NoRepeat]),
        ["repeat-y"] => Some([Repeat::NoRepeat, Repeat::Repeat]),
        [w] => one(w).map(|r| [r; 2]),
        [x, y] => one(x).zip(one(y)).map(|(x, y)| [x, y]),
        _ => None,
    };
    r.ok_or_else(|| {
        format!(
            "{:?} is not a repeat: repeat-x, repeat-y, or one or two of repeat, space, round and \
             no-repeat",
            words.join(" ")
        )
    })
}

/// The words of a position, as the `background` shorthand finds them.
fn positional(w: &str) -> bool {
    ["left", "right", "top", "bottom", "center"].contains(&w)
        || css::num(w).is_some()
        || math::is_math(w)
}

fn repeating(w: &str) -> bool {
    [
        "repeat-x",
        "repeat-y",
        "repeat",
        "space",
        "round",
        "no-repeat",
    ]
    .contains(&w)
}

/// `background`: layers separated by commas, each an image, a position with a size after a
/// slash, and a repeat, in any order, and the last a color too. What it leaves out is initial.
pub(super) fn background(s: &str) -> Result<(Declared, Rgba), String> {
    let layers = css::commas(s);
    if layers.is_empty() {
        return Err("\"\" is not a background; use none".into());
    }
    let mut d = Declared::default();
    let mut color = None;
    for (n, layer) in layers.iter().enumerate() {
        let not = |what: &str| format!("{layer:?} is not a background layer: {what}");
        let halves = css::slashes(layer);
        let (before, after) = match &halves[..] {
            [all] => (css::words(all), Vec::new()),
            [before, after] => (css::words(before), css::words(after)),
            _ => return Err(not("it has more than one slash")),
        };
        // The words, `None` for the slash.
        let words: Vec<Option<&str>> = (before.iter().map(|w| Some(*w)))
            .chain((halves.len() == 2).then_some(None))
            .chain(after.iter().map(|w| Some(*w)))
            .collect();
        let (mut image, mut at, mut sized, mut repeated) = (None, None, None, None);
        let mut i = 0;
        while i < words.len() {
            let run = |i: usize, test: fn(&str) -> bool| {
                (words[i..].iter())
                    .take_while(|w| w.is_some_and(test))
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>()
            };
            let Some(w) = words[i] else {
                return Err(not("a slash goes between a position and a size"));
            };
            if positional(w) && at.is_none() {
                let p = run(i, positional);
                at = Some(position(&p, true)?);
                i += p.len();
                if i < words.len() && words[i].is_none() {
                    let s = run(i + 1, |w| {
                        ["cover", "contain", "auto"].contains(&w)
                            || css::num(w).is_some()
                            || math::is_math(w)
                    });
                    let s = &s[..s.len().min(2)];
                    sized = Some(size(s)?);
                    i += 1 + s.len();
                }
                continue;
            }
            if repeating(w) && repeated.is_none() {
                let r = run(i, repeating);
                let r = &r[..r.len().min(2)];
                repeated = Some(repeat(r)?);
                i += r.len();
                continue;
            }
            if ["scroll", "fixed", "local"].contains(&w) {
                return Err(format!("{w}: {NO_SCROLL}; {NO_SCROLL_INSTEAD}"));
            }
            if w.ends_with("-box") || w == "text" {
                return Err(format!(
                    "{w}: background-origin and -clip are not supported yet"
                ));
            }
            if let Ok(c) = color::parse(w) {
                if n + 1 < layers.len() || color.is_some() {
                    return Err(not("only the last layer takes a color, and only one"));
                }
                color = Some(c);
            } else if w != "none" && css::function(w).is_none() {
                return Err(format!("{w} is not a color or an image"));
            } else if image.is_none() {
                image = Some(self::image(w)?);
            } else {
                return Err(not("it has two images"));
            }
            i += 1;
        }
        d.images.push(image.unwrap_or(Image::None));
        let [x, y] = at.unwrap_or_else(|| {
            [0, 1].map(|_| Offset {
                end: false,
                by: Len::Percent(0.0),
            })
        });
        d.positions[0].push(x);
        d.positions[1].push(y);
        d.sizes
            .push(sized.unwrap_or(ImageSize::Sized([None, None])));
        d.repeats.push(repeated.unwrap_or([Repeat::Repeat; 2]));
    }
    Ok((d, color.unwrap_or(color::TRANSPARENT)))
}
