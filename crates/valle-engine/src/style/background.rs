//! Backgrounds: a color under layers of images, each sized, placed and repeated in one of its
//! boxes, clipped to one, and blended with what is under it. The properties of the layers are
//! lists, each repeated for as many images as there are. Masks are layers of images too, which
//! show the box by their alpha or their luminance.

use super::color::{self, Rgba};
use super::computed::{BLENDS, Blend};
use super::gradient::{self, Image, image};
use super::math::Calc;
use super::position::{Offset, position};
use super::property::{NO_SCROLL, NO_SCROLL_INSTEAD};
use super::units::{Accept, Len, Units, length_text};
use super::{css, math};
use crate::front::Slot;

/// A background image, how big it is, where it is, how it repeats, the box it is placed in, the
/// one it shows in, and how it blends with the images under it and the color. The last layer's
/// `clip` is the color's too. A mask's layer does not blend, and shows the box by its image's
/// alpha, or with `luminance` by its luminance times its alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub image: Image,
    pub size: ImageSize,
    pub position: [Offset; 2],
    pub repeat: [Repeat; 2],
    pub origin: Area,
    pub clip: Area,
    pub blend: Blend,
    pub luminance: bool,
    pub composite: Composite,
}

/// `mask-composite`: how a mask layer goes with all the layers under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Composite {
    /// Over them.
    Add,
    /// Where they are not.
    Subtract,
    /// Where they are.
    Intersect,
    /// Where they are not, and they where it is not.
    Exclude,
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

/// One of a box's areas, out to an edge of its border, its padding or its content; or, as a
/// background's clip only, its text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Area {
    BorderBox,
    PaddingBox,
    ContentBox,
    /// The glyphs and decorations of the box's text and its descendants', in the border box.
    Text,
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

/// The background or mask properties as declared: each a list, `positions` one for x and one
/// for y.
#[derive(Clone, Debug, Default)]
pub(super) struct Declared {
    pub images: Vec<gradient::Declared>,
    pub sizes: Vec<ImageSize<Len>>,
    pub positions: [Vec<Offset<Len>>; 2],
    pub repeats: Vec<[Repeat; 2]>,
    pub origins: Vec<Area>,
    pub clips: Vec<Area>,
    pub blends: Vec<Blend>,
    /// Masks', whether each shows by luminance.
    pub modes: Vec<bool>,
    /// Masks', how each goes with those under it.
    pub composites: Vec<Composite>,
}

impl Declared {
    /// A layer for each image, at least one, its other properties from their lists, repeated as
    /// needed; placed in `origin` unless it says, the padding box for backgrounds and the border
    /// box for masks.
    pub(super) fn computed(
        &self,
        u: Units,
        current: Rgba,
        origin: Area,
        resources: &[Slot],
    ) -> Result<Vec<Layer>, String> {
        let start = || Offset {
            end: false,
            by: Len::Percent(0.0),
        };
        (0..self.images.len().max(1))
            .map(|i| {
                let image = match nth(&self.images, i) {
                    Some(m) => m.computed(u, current, resources)?,
                    None => Image::None,
                };
                Ok(Layer {
                    image,
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
                    origin: nth(&self.origins, i).unwrap_or(origin),
                    clip: nth(&self.clips, i).unwrap_or(Area::BorderBox),
                    blend: nth(&self.blends, i).unwrap_or(Blend::Normal),
                    luminance: nth(&self.modes, i).unwrap_or(false),
                    composite: nth(&self.composites, i).unwrap_or(Composite::Add),
                })
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

/// `background-position`: positions of one to four words; `mask-position`, of one, two or four,
/// unless `three`.
pub(super) fn positions(s: &str, three: bool) -> Result<[Vec<Offset<Len>>; 2], String> {
    let both = list(s, "a position", |s| position(&css::words(s), three))?;
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

/// `background-origin`, or `background-clip` if `clip`.
pub(super) fn areas(s: &str, clip: bool) -> Result<Vec<Area>, String> {
    list(s, "a box", |s| match area(s) {
        Some(a) => Ok(a),
        None if clip && s == "text" => Ok(Area::Text),
        None => Err(format!(
            "{s:?} is not a box: border-box, padding-box or content-box"
        )),
    })
}

/// `background-blend-mode`: the modes of `mix-blend-mode` but plus-lighter.
pub(super) fn blends(s: &str) -> Result<Vec<Blend>, String> {
    let modes = &BLENDS[..BLENDS.len() - 1];
    list(s, "a blend mode", |s| {
        match modes.iter().find(|(k, _)| *k == s) {
            Some((_, b)) => Ok(*b),
            None => {
                let names: Vec<&str> = modes.iter().map(|(k, _)| *k).collect();
                Err(format!("{s} is not one of {}", names.join(", ")))
            }
        }
    })
}

/// `mask-mode`: whether each layer shows by luminance; an image's own mode is alpha.
pub(super) fn modes(s: &str) -> Result<Vec<bool>, String> {
    list(s, "a mask mode", |s| match s {
        "alpha" | "match-source" => Ok(false),
        "luminance" => Ok(true),
        _ => Err(format!(
            "{s:?} is not a mask mode: alpha, luminance or match-source"
        )),
    })
}

/// `mask-composite`.
pub(super) fn composites(s: &str) -> Result<Vec<Composite>, String> {
    list(s, "a mask composite", |s| match s {
        "add" => Ok(Composite::Add),
        "subtract" => Ok(Composite::Subtract),
        "intersect" => Ok(Composite::Intersect),
        "exclude" => Ok(Composite::Exclude),
        _ => Err(format!(
            "{s:?} is not a mask composite: add, subtract, intersect or exclude"
        )),
    })
}

pub(super) fn area(w: &str) -> Option<Area> {
    match w {
        "border-box" => Some(Area::BorderBox),
        "padding-box" => Some(Area::PaddingBox),
        "content-box" => Some(Area::ContentBox),
        _ => None,
    }
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
/// slash, a repeat, and a box for its origin and clip or two boxes, the origin's first, in any
/// order, and the last a color too. What it leaves out is initial.
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
        let mut boxes = Vec::new();
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
            if let Some(a) = area(w).or((w == "text").then_some(Area::Text)) {
                if boxes.len() == 2 {
                    return Err(not("it has more than two boxes"));
                }
                boxes.push(a);
                i += 1;
                continue;
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
        // Text is a clip alone: with a box before it, the box is the origin.
        let origin = match boxes[..] {
            [Area::Text, _] => return Err(not("text is a clip only, so it goes last")),
            [Area::Text] | [] => Area::PaddingBox,
            [a, ..] => a,
        };
        d.origins.push(origin);
        d.clips
            .push(boxes.last().copied().unwrap_or(Area::BorderBox));
    }
    Ok((d, color.unwrap_or(color::TRANSPARENT)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_keeps_nested_commas_and_sizes_attached_to_their_layers() {
        let (d, color) = background("linear-gradient(rgb(255, 0, 0), blue) right 5px bottom 10px / calc(50% - 2px) 20px no-repeat, white").unwrap();
        let layers = d
            .computed(
                Units {
                    em: 16.0,
                    canvas: [200.0, 100.0],
                },
                [0.0; 4],
                Area::PaddingBox,
                &[],
            )
            .unwrap();
        assert_eq!(color, [1.0; 4]);
        assert_eq!(layers.len(), 2);
        let ImageSize::Sized([Some(w), Some(h)]) = &layers[0].size else {
            panic!("size")
        };
        assert_eq!((w.resolve(100.0), h.resolve(100.0)), (48.0, 20.0));
        assert_eq!(
            layers[0].position.each_ref().map(|p| p.resolve(100.0)),
            [95.0, 90.0]
        );
        assert_eq!(layers[0].repeat, [Repeat::NoRepeat; 2]);
        assert_eq!(layers[1].size, ImageSize::Sized([None, None]));
    }

    #[test]
    fn invalid_layers_do_not_become_partial_backgrounds() {
        for s in [
            "red, none",
            "red blue",
            "none / cover",
            "none center / cover / cover",
            "none border-box padding-box content-box",
            "none center / -1px",
        ] {
            assert!(background(s).is_err(), "{s}");
        }
    }
}
