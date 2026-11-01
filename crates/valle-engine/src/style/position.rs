//! CSS positions: a point in a box, on each axis a length from its start or from its end, as a
//! gradient's `at` and `background-position` write it.

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

/// x and y from one, two or four words, or `three` too: a keyword or a length on either axis, a
/// keyword or a length for x and then for y, keywords in either order, or two keywords, a side
/// with a length from it or not and the other a side with a length or center.
/// `object-position`: one, two or four words of a position.
pub(super) fn object(s: &str) -> Result<[Offset<Len>; 2], String> {
    position(&css::words(s), false).map_err(|_| {
        format!(
            "{s:?} is not an object position: one, two or four of left, center, right, top, \
             bottom and lengths"
        )
    })
}

pub(super) fn position(words: &[&str], three: bool) -> Result<[Offset<Len>; 2], String> {
    let bad = || {
        let counts = if three {
            "one to four"
        } else {
            "one, two or four"
        };
        format!(
            "{:?} is not a position: {counts} of left, center, right, top, bottom and lengths",
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
        _ if words.len() == 4 || (three && words.len() == 3) => {
            // Two keywords, each a side with a length from it or not, or center.
            let mut sides = Vec::new();
            let mut i = 0;
            while let Some(w) = words.get(i) {
                let side = word(w)?;
                let by = match words.get(i + 1).map(|w| word(w)).transpose()? {
                    Some(Word::Len(l)) if horizontal(&side) || vertical(&side) => Some(l),
                    Some(Word::Len(_)) => return Err(bad()),
                    _ => None,
                };
                i += 1 + usize::from(by.is_some());
                sides.push((side, by));
            }
            let [a, b] = <[_; 2]>::try_from(sides).map_err(|_| bad())?;
            let ((h, x), (v, y)) = match vertical(&a.0) || horizontal(&b.0) {
                true => (b, a),
                false => (a, b),
            };
            let offset = |side: Word, by: Option<Len>, end| match (side, by) {
                (Word::Center, _) => center(),
                (_, Some(by)) => from(end, by),
                (_, None) => from(end, Len::Percent(0.0)),
            };
            let fits = |w: &Word, axis: fn(&Word) -> bool| axis(w) || matches!(w, Word::Center);
            (fits(&h, horizontal) && fits(&v, vertical)).then(|| {
                let (right, bottom) = (matches!(h, Word::Right), matches!(v, Word::Bottom));
                [offset(h, x, right), offset(v, y, bottom)]
            })
        }
        _ => None,
    };
    found.ok_or_else(bad)
}
