//! Dashed and dotted borders and outlines, as Chrome draws them: with square corners each side a
//! line of dashes or dots along its middle from corner to corner, and with rounded ones a loop of
//! them around the middle of the border, from the end of the top left corner clockwise. And dotted,
//! dashed and wavy text decorations.

use super::shape::SHOWN;
use super::{Item, Radii, SQUARE, inner, work};
use crate::layout::Rect;
use crate::path::{Path, Point, Segment};
use crate::style::color::Rgba;
use crate::style::{Blend, BorderStyle, DecorationStyle};

/// A dash along a line or a loop, from and to how far along it, or a dot centered that far along.
#[derive(Debug, PartialEq)]
enum Mark {
    Dash([f32; 2]),
    Dot(f32),
}

/// The dashes and dots of the dashed and dotted sides of a border or an outline `widths` deep
/// inside `rect` with its corners rounded by `radii`. A side next to a side in another color or
/// style keeps to its own part of the border, cut from corner to corner. Sides alike in alpha are
/// painted opaque in one layer at that alpha, as in Chrome, so where they meet they show once.
pub(super) fn dashes(
    rect: Rect,
    radii: Radii,
    widths: [f32; 4],
    styles: [BorderStyle; 4],
    colors: [Rgba; 4],
) -> Vec<Item> {
    let drawn = |k: usize| {
        matches!(styles[k], BorderStyle::Dashed | BorderStyle::Dotted)
            && widths[k] > 0.0
            && colors[k][3] > 0.0
    };
    let shown = |k: usize| widths[k] > 0.0 && colors[k][3] > 0.0 && styles[k] != BorderStyle::None;
    let (inside, _) = inner(rect, radii, widths);
    let middle = middle(rect, radii, widths);
    // One loop for four sides alike, or each side's own loop cut to its part.
    let alike =
        (1..4).all(|k| (styles[k], widths[k], colors[k]) == (styles[0], widths[0], colors[0]));
    let side = |k: usize| {
        let [r, g, b, _] = colors[k];
        let (w, color) = (widths[k], work([r, g, b, 1.0]));
        if radii != SQUARE {
            let looped = around(&middle, rect, radii, widths, w, styles[k], color);
            return match alike {
                true => looped,
                false => cut(quad(rect, inside, k), looped),
            };
        }
        // Along the side's middle, left to right or top to bottom.
        let (start, along, length) = match k {
            0 => ([rect.x, rect.y + w / 2.0], [1.0, 0.0], rect.width),
            1 => (
                [rect.x + rect.width - w / 2.0, rect.y],
                [0.0, 1.0],
                rect.height,
            ),
            2 => (
                [rect.x, rect.y + rect.height - w / 2.0],
                [1.0, 0.0],
                rect.width,
            ),
            _ => ([rect.x + w / 2.0, rect.y], [0.0, 1.0], rect.height),
        };
        let marks = placed(
            marks(length, w, styles[k], Line::Side),
            start,
            along,
            w,
            color,
        );
        // Chrome cuts a side at its corners only where the next side is unlike it.
        let unlike = [(k + 1) % 4, (k + 3) % 4]
            .into_iter()
            .any(|j| shown(j) && (styles[j], colors[j]) != (styles[k], colors[k]));
        match unlike {
            true => cut(quad(rect, inside, k), marks),
            false => marks,
        }
    };
    let mut layers: Vec<(f64, Vec<Item>)> = Vec::new();
    for k in (0..4).filter(|&k| drawn(k) && (k == 0 || !alike || radii == SQUARE)) {
        let alpha = colors[k][3];
        match layers.iter_mut().find(|(a, _)| *a == alpha) {
            Some((_, items)) => items.extend(side(k)),
            None => layers.push((alpha, side(k))),
        }
    }
    (layers.into_iter())
        .flat_map(|(opacity, items)| match opacity >= 1.0 {
            true => items,
            false => vec![Item::Group {
                opacity: opacity as f32,
                blend: Blend::Normal,
                items,
            }],
        })
        .collect()
}

/// What marks go along: a side of a border, a loop around one, or a text decoration's line.
#[derive(Clone, Copy, PartialEq)]
enum Line {
    Side,
    Loop,
    Text,
}

/// Marks along a line `w` wide from `start` going `along`, as rects: dashes, and dots round, as
/// wide as the line.
fn placed(marks: Vec<Mark>, start: Point, along: [f32; 2], w: f32, color: [f32; 4]) -> Vec<Item> {
    (marks.into_iter())
        .map(|mark| {
            let at = |s: f32| [start[0] + along[0] * s, start[1] + along[1] * s];
            let ([from, to], radii) = match mark {
                Mark::Dash([a, b]) => ([at(a), at(b)], SQUARE),
                Mark::Dot(s) => ([at(s - w / 2.0), at(s + w / 2.0)], [[w / 2.0; 2]; 4]),
            };
            // Across the line, half its width each way.
            let [x0, y0] = [from[0] - along[1] * w / 2.0, from[1] - along[0] * w / 2.0];
            let [x1, y1] = [to[0] + along[1] * w / 2.0, to[1] + along[0] * w / 2.0];
            Item::Rect {
                rect: Rect {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                },
                radii,
                color,
            }
        })
        .collect()
}

/// The dots or dashes of a piece of a text decoration `w` thick in `style`, whose line runs
/// `length` from `start`, as Chrome spaces them along it in whole px, the width rounded to whole
/// px: those in `piece`, cut to it.
pub(super) fn text(
    piece: Rect,
    start: f32,
    length: f32,
    w: f32,
    style: DecorationStyle,
    color: [f32; 4],
) -> Vec<Item> {
    let style = match style {
        DecorationStyle::Dotted => BorderStyle::Dotted,
        _ => BorderStyle::Dashed,
    };
    let [lo, hi] = [piece.x - start, piece.x + piece.width - start];
    let (mut kept, mut cut) = (Vec::new(), Vec::new());
    for mark in marks(length.round(), w.round().max(1.0), style, Line::Text) {
        match mark {
            Mark::Dash([a, b]) if a.max(lo) < b.min(hi).min(length) => {
                kept.push(Mark::Dash([a.max(lo), b.min(hi).min(length)]))
            }
            Mark::Dot(s) if lo <= s - w / 2.0 && s + w / 2.0 <= hi => kept.push(mark),
            Mark::Dot(s) if lo < s + w / 2.0 && s - w / 2.0 < hi => cut.push(mark),
            _ => {}
        }
    }
    let at = [start, piece.y + piece.height / 2.0];
    let mut items = placed(kept, at, [1.0, 0.0], w, color);
    if !cut.is_empty() {
        items.push(Item::Mask {
            mask: vec![Item::Rect {
                rect: piece,
                radii: SQUARE,
                color: SHOWN,
            }],
            items: placed(cut, at, [1.0, 0.0], w, color),
        });
    }
    items
}

/// A piece of a wavy text decoration `w` thick whose line starts at `start`, as Chrome draws it:
/// its waves from the start, around the middle of `piece`, cut to it.
pub(super) fn wave(piece: Rect, start: f32, w: f32, color: [f32; 4]) -> Vec<Item> {
    let [step, control] = crate::text::wave(w);
    let axis = piece.y + piece.height / 2.0;
    let first = ((piece.x - start) / (2.0 * step)).floor() as i32;
    let last = ((piece.x + piece.width - start) / (2.0 * step)).ceil() as i32;
    // Each wave a curve down then up, as the control points take it, y down.
    let mut middle: Vec<Point> = Vec::new();
    for k in first..last {
        let x0 = start + k as f32 * 2.0 * step;
        let n = if middle.is_empty() { 0 } else { 1 };
        middle.extend((n..=24).map(|n| {
            let u = n as f32 / 24.0;
            let x = x0 + step * 3.0 * u * (1.0 - u) + 2.0 * step * u * u * u;
            [x, axis + control * 3.0 * u * (1.0 - u) * (1.0 - 2.0 * u)]
        }));
    }
    // Stroked: half the width either side of it.
    let normal = |i: usize| {
        let [a, b] = [
            middle[i.saturating_sub(1)],
            middle[(i + 1).min(middle.len() - 1)],
        ];
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        [(a[1] - b[1]) / d, (b[0] - a[0]) / d]
    };
    let off = |i: usize, by: f32| {
        let n = normal(i);
        [middle[i][0] + n[0] * by, middle[i][1] + n[1] * by]
    };
    let outline: Vec<Point> = (0..middle.len())
        .map(|i| off(i, w / 2.0))
        .chain((0..middle.len()).rev().map(|i| off(i, -w / 2.0)))
        .collect();
    let mut path = vec![Segment::Move(outline[0])];
    path.extend(outline[1..].iter().map(|&p| Segment::Line(p)));
    vec![Item::Mask {
        mask: vec![Item::Rect {
            rect: piece,
            radii: SQUARE,
            color: SHOWN,
        }],
        items: vec![Item::Path {
            path: Path(path),
            even_odd: false,
            color,
        }],
    }]
}

/// The marks along a side, a loop or a text decoration's line `length` long, `w` wide in `style`.
/// Dashes are three times as long as the width under 3 px and twice as long from 3 px, with gaps
/// two widths and one, as near as a whole number of them fits, or the line is solid; a line
/// starts and ends with a dash. Dots from 4 px are round, as wide as the line, a width apart as
/// near as fits, and from the line's ends, or one where it is too short for two. Under that they
/// are square: a side has them at both ends, and between them a width apart from its end, half a
/// width further on, in whole px, where the first would touch the one at its start; a text line,
/// a width apart from its start.
fn marks(length: f32, w: f32, style: BorderStyle, line: Line) -> Vec<Mark> {
    let closed = line == Line::Loop;
    let round = style == BorderStyle::Dotted && w > 3.0;
    if round {
        if length <= 2.0 * w {
            return vec![Mark::Dot(if closed { 0.0 } else { w / 2.0 })];
        }
        let gap = spaced(length, w, w, closed);
        let from = if closed { 0.0 } else { w / 2.0 };
        return (0..count(length, w, gap, closed))
            .map(|n| Mark::Dot(from + n as f32 * (w + gap)))
            .collect();
    }
    let (dash, gap) = match style {
        BorderStyle::Dashed if w >= 3.0 => (2.0 * w, w),
        BorderStyle::Dashed => (3.0 * w, 2.0 * w),
        _ => (w, w),
    };
    if length <= 2.0 * dash {
        return vec![Mark::Dash([0.0, length])];
    }
    if style == BorderStyle::Dotted && line == Line::Text {
        let starts = (0..)
            .map(|n| n as f32 * 2.0 * w)
            .take_while(|&s| s < length);
        return starts.map(|s| Mark::Dash([s, s + w])).collect();
    }
    if style == BorderStyle::Dotted && line == Line::Side {
        let mut between: Vec<f32> = (1..)
            .map(|n| length - w - n as f32 * 2.0 * w)
            .take_while(|&s| s >= w)
            .collect();
        if between.last() == Some(&w) {
            between.iter_mut().for_each(|s| *s += (w / 2.0).floor());
        }
        return [0.0, length - w]
            .into_iter()
            .chain(between)
            .map(|s| Mark::Dash([s, s + w]))
            .collect();
    }
    let gap = spaced(length, dash, gap, closed);
    (0..count(length, dash, gap, closed))
        .map(|n| {
            let s = n as f32 * (dash + gap);
            Mark::Dash([s, s + dash])
        })
        .collect()
}

/// Chrome's gap between marks `mark` long along a line or a loop `length` long: of the whole
/// numbers of them that fit with gaps at least as near `gap` as fits, the one whose gap is nearest
/// it. A loop has as many gaps as marks, a line one fewer.
fn spaced(length: f32, mark: f32, gap: f32, closed: bool) -> f32 {
    let room = if closed { length } else { length + gap };
    let fewer = (room / (mark + gap)).floor();
    let gaps = |n: f32| if closed { n } else { n - 1.0 };
    let wider = (length - fewer * mark) / gaps(fewer);
    let narrower = (length - (fewer + 1.0) * mark) / gaps(fewer + 1.0);
    match narrower <= 0.0 || (wider - gap).abs() < (narrower - gap).abs() {
        true => wider,
        false => narrower,
    }
}

/// How many marks `mark` long `gap` apart fill a line or a loop `length` long.
fn count(length: f32, mark: f32, gap: f32, closed: bool) -> usize {
    let room = if closed { length } else { length + gap };
    (room / (mark + gap)).round() as usize
}

/// The middle of a border, as Chrome finds it: the rectangle half its widths inside `rect`, an
/// odd px nearer the outside, in whole px, with the corners rounded as much less.
fn middle(rect: Rect, radii: Radii, widths: [f32; 4]) -> (Rect, Radii) {
    let (half, radii) = inner(rect, radii, widths.map(|w| (w / 2.0).floor()));
    let [x0, y0] = [half.x.round(), half.y.round()];
    let [x1, y1] = [
        (half.x + half.width).round(),
        (half.y + half.height).round(),
    ];
    let snapped = Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    };
    (snapped, radii)
}

/// The points of a loop around the rectangle `middle` with its rounded corners, from the end of
/// the top left corner clockwise, no further apart than a third of a px along its corners.
fn points((r, radii): &(Rect, Radii)) -> Vec<Point> {
    let [l, t, right, b] = [r.x, r.y, r.x + r.width, r.y + r.height];
    // Each corner's center, and the angles it turns from and to, y down.
    let corners = [
        ([right - radii[1][0], t + radii[1][1]], radii[1], -90.0f32),
        ([right - radii[2][0], b - radii[2][1]], radii[2], 0.0),
        ([l + radii[3][0], b - radii[3][1]], radii[3], 90.0),
        ([l + radii[0][0], t + radii[0][1]], radii[0], 180.0),
    ];
    let mut points = vec![[l + radii[0][0], t]];
    for (c, [rx, ry], from) in corners {
        let steps = ((rx.max(ry) * std::f32::consts::FRAC_PI_2 * 3.0).ceil() as usize).max(1);
        for n in 0..=steps {
            let a = (from + 90.0 * n as f32 / steps as f32).to_radians();
            points.push([c[0] + rx * a.cos(), c[1] + ry * a.sin()]);
        }
    }
    points.push([l + radii[0][0], t]);
    points.dedup();
    points
}

/// The marks of a loop around the border's `middle`, of a side `w` wide in `style` and `color`,
/// cut to the border: dots, or dashes.
fn around(
    middle: &(Rect, Radii),
    rect: Rect,
    radii: Radii,
    widths: [f32; 4],
    w: f32,
    style: BorderStyle,
    color: [f32; 4],
) -> Vec<Item> {
    let points = points(middle);
    let lengths: Vec<f32> = (points.windows(2))
        .scan(0.0, |s, p| {
            *s += (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]);
            Some(*s)
        })
        .collect();
    let length = lengths.last().copied().unwrap_or(0.0);
    // The point `s` along the loop, and the segment it is on.
    let at = |s: f32| {
        let i = lengths.partition_point(|&e| e < s).min(lengths.len() - 1);
        let start = if i == 0 { 0.0 } else { lengths[i - 1] };
        let [a, b] = [points[i], points[i + 1]];
        let f = ((s - start) / (lengths[i] - start)).clamp(0.0, 1.0);
        ([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], i)
    };
    // To the right of the way the segment goes, outward, as the loop goes clockwise, y down.
    let normal = |i: usize| {
        let [a, b] = [points[i], points[i + 1]];
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        [(a[1] - b[1]) / d, (b[0] - a[0]) / d]
    };
    let mut path = Vec::new();
    let mut marked = Vec::new();
    // Chrome spaces them along the loop's length in whole px.
    for mark in marks(length.floor(), w, style, Line::Loop) {
        match mark {
            Mark::Dot(s) => {
                let ([x, y], _) = at(s);
                marked.push(Item::Rect {
                    rect: Rect {
                        x: x - w / 2.0,
                        y: y - w / 2.0,
                        width: w,
                        height: w,
                    },
                    radii: [[w / 2.0; 2]; 4],
                    color,
                });
            }
            // A stretch of a stroke 2.2 widths wide, which the border cuts, as Chrome draws it.
            Mark::Dash([a, b]) => {
                let ((p, i), (q, j)) = (at(a), at(b));
                let middle: Vec<(Point, usize)> = std::iter::once((p, i))
                    .chain((i + 1..=j).map(|k| (points[k], k.min(lengths.len() - 1))))
                    .chain(std::iter::once((q, j)))
                    .collect();
                let off = |(p, i): &(Point, usize), by: f32| {
                    let n = normal(*i);
                    [p[0] + n[0] * by, p[1] + n[1] * by]
                };
                let h = 1.1 * w;
                let outer = middle.iter().map(|m| off(m, h));
                let inner = middle.iter().rev().map(|m| off(m, -h));
                let mut ring = outer.chain(inner);
                path.extend(ring.next().map(Segment::Move));
                path.extend(ring.map(Segment::Line));
            }
        }
    }
    if !path.is_empty() {
        marked.push(Item::Path {
            path: Path(path),
            even_odd: false,
            color,
        });
    }
    let (inside, inner_radii) = inner(rect, radii, widths);
    let border = Item::Ring {
        rect,
        radii,
        inner: inside,
        inner_radii,
        colors: [SHOWN; 4],
    };
    vec![Item::Mask {
        mask: vec![border],
        items: marked,
    }]
}

/// Side `k`'s part of a border on `rect` around `inside`: from its outer corners to its inner ones.
fn quad(rect: Rect, inside: Rect, k: usize) -> Path {
    let [l, t, r, b] = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
    let [il, it] = [inside.x, inside.y];
    let [ir, ib] = [inside.x + inside.width, inside.y + inside.height];
    let corners = match k {
        0 => [[l, t], [r, t], [ir, it], [il, it]],
        1 => [[r, t], [r, b], [ir, ib], [ir, it]],
        2 => [[r, b], [l, b], [il, ib], [ir, ib]],
        _ => [[l, b], [l, t], [il, it], [il, ib]],
    };
    let mut segments = vec![Segment::Move(corners[0])];
    segments.extend(corners[1..].iter().map(|&p| Segment::Line(p)));
    Path(segments)
}

/// `items` shown only inside `part`.
fn cut(part: Path, items: Vec<Item>) -> Vec<Item> {
    vec![Item::Mask {
        mask: vec![Item::Path {
            path: part,
            even_odd: false,
            color: SHOWN,
        }],
        items,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashes(length: f32, w: f32, style: BorderStyle, closed: bool) -> Vec<[f32; 2]> {
        let line = if closed { Line::Loop } else { Line::Side };
        (marks(length, w, style, line).into_iter())
            .map(|m| match m {
                Mark::Dash(d) => d,
                Mark::Dot(s) => [s, s],
            })
            .collect()
    }

    #[test]
    fn dashes_fit_lines_as_chrome_fits_them() {
        use BorderStyle::{Dashed, Dotted};
        // As Chrome 156 draws the sides of an 80 × 40 box: 1 px dashes 3 long, 16 of them 2.13
        // apart; 6 px ones 12 long, 5 on the top and 3 on a side, 5 and 2 apart.
        let top = dashes(80.0, 1.0, Dashed, false);
        assert_eq!((top.len(), top[1][0]), (16, 3.0 + 32.0 / 15.0));
        assert_eq!(dashes(80.0, 6.0, Dashed, false)[1], [17.0, 29.0]);
        assert_eq!(dashes(40.0, 6.0, Dashed, false).len(), 3);
        assert_eq!(dashes(80.0, 4.0, Dashed, false)[1], [12.0, 20.0]);
        // Too short for two dashes, a side is solid.
        assert_eq!(dashes(40.0, 10.0, Dashed, false), [[0.0, 40.0]]);
        // Square dots at the ends and from the end; round ones from both, 3.6 apart at 4 px.
        assert_eq!(
            dashes(10.0, 2.0, Dotted, false),
            [[0.0, 2.0], [8.0, 10.0], [4.0, 6.0]]
        );
        assert_eq!(
            dashes(7.0, 1.0, Dotted, false)[..3],
            [[0.0, 1.0], [6.0, 7.0], [4.0, 5.0]]
        );
        let twelve = [[0.0, 2.0], [10.0, 12.0], [7.0, 9.0], [3.0, 5.0]];
        assert_eq!(dashes(12.0, 2.0, Dotted, false), twelve);
        let round = dashes(80.0, 4.0, Dotted, false);
        assert_eq!(
            (round.len(), round[0][0], round[10][0].round()),
            (11, 2.0, 78.0)
        );
        // A loop has as many gaps as dashes.
        assert_eq!(dashes(100.0, 2.0, Dashed, true).len(), 10);
    }
}
