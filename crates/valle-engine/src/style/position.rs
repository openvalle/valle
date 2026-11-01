//! CSS positions: a point in a box, on each axis a length from its start or from its end, as a
//! gradient's `at` writes it.

use super::math::Calc;
use super::{Accept, Len, Units, css, length_text, math};

/// On one axis, a length from the box's start (its left or top), or from its `end`.
#[derive(Clone, Debug, PartialEq)]
pub struct Offset<P = Calc> {
    pub end: bool,
    pub by: P,
}

impl Offset {
    /// px from the start of a box `size` px long.
    pub fn resolve(&self, size: f32) -> f32 {
        let by = self.by.resolve(size);
        if self.end { size - by } else { by }
    }
}

impl Offset<Len> {
    pub(super) fn computed(&self, u: Units) -> Offset {
        Offset {
            end: self.end,
            by: self.by.calc(u),
        }
    }
}

const LENGTH: Accept = Accept {
    keyword: None,
    negative: true,
    percent: true,
    sizing: &[],
};

/// A word of a position.
enum Word {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    Len(Len),
}

/// x and y from one, two or four words: a keyword or a length on either axis, a keyword or a
/// length for x and then for y, keywords in either order, or two keywords each with a length
/// from that side.
pub(super) fn position(words: &[&str]) -> Result<[Offset<Len>; 2], String> {
    let bad = || {
        format!(
            "{:?} is not a position: one, two or four of left, center, right, top, bottom and \
             lengths",
            words.join(" ")
        )
    };
    let word = |w: &str| -> Result<Word, String> {
        Ok(match w {
            "left" => Word::Left,
            "right" => Word::Right,
            "top" => Word::Top,
            "bottom" => Word::Bottom,
            "center" => Word::Center,
            w if css::num(w).is_some() || math::is_math(w) => Word::Len(length_text(w, LENGTH)?),
            _ => return Err(bad()),
        })
    };
    let from = |end, by| Offset { end, by };
    let center = || from(false, Len::Percent(50.0));
    // x from a keyword or a length, and y.
    let x = |w: Word| match w {
        Word::Left => Some(from(false, Len::Percent(0.0))),
        Word::Right => Some(from(true, Len::Percent(0.0))),
        Word::Center => Some(center()),
        Word::Len(l) => Some(from(false, l)),
        _ => None,
    };
    let y = |w: Word| match w {
        Word::Top => Some(from(false, Len::Percent(0.0))),
        Word::Bottom => Some(from(true, Len::Percent(0.0))),
        Word::Center => Some(center()),
        Word::Len(l) => Some(from(false, l)),
        _ => None,
    };
    let vertical = |w: &Word| matches!(w, Word::Top | Word::Bottom);
    let horizontal = |w: &Word| matches!(w, Word::Left | Word::Right);
    let found = match words {
        [a] => match word(a)? {
            w if vertical(&w) => y(w).map(|y| [center(), y]),
            w => x(w).map(|x| [x, center()]),
        },
        [a, b] => {
            let (a, b) = (word(a)?, word(b)?);
            // Keywords go in either order.
            let keywords = !matches!(a, Word::Len(_)) && !matches!(b, Word::Len(_));
            let (a, b) = match keywords && (vertical(&a) || horizontal(&b)) {
                true => (b, a),
                false => (a, b),
            };
            x(a).zip(y(b)).map(|(x, y)| [x, y])
        }
        [a, l, b, m] => {
            let side = |w: &str, l: &str| -> Result<(Word, Len), String> {
                match (word(w)?, word(l)?) {
                    (w, Word::Len(l)) if horizontal(&w) || vertical(&w) => Ok((w, l)),
                    _ => Err(bad()),
                }
            };
            let (a, b) = (side(a, l)?, side(b, m)?);
            let ((h, x), (v, y)) = match vertical(&a.0) {
                true => (b, a),
                false => (a, b),
            };
            (horizontal(&h) && vertical(&v)).then(|| {
                [
                    from(matches!(h, Word::Right), x),
                    from(matches!(v, Word::Bottom), y),
                ]
            })
        }
        _ => None,
    };
    found.ok_or_else(bad)
}
