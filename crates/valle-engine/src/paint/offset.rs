//! Motion paths: where a box's `offset-path` moves it and how it turns it there.

use super::matrix::Matrix4;
use super::{Radii, shape};
use crate::layout::Rect;
use crate::path::{Path, Point, Segment};
use crate::style::math::Calc;
use crate::style::offset::{Motion, Route, Size, Start};
use crate::style::position::Offset;
use crate::style::shape::Shape;

/// A piece of a path: a line, a quadratic curve, a conic one and its weight, or a cubic one, by
/// its points.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Line([Point; 2]),
    Quad([Point; 3]),
    Conic([Point; 3], f32),
    Cubic([Point; 4]),
}

/// Subpaths, each where it starts, its pieces and whether it closes.
type Contours = Vec<(Point, Vec<Piece>, bool)>;

/// The weight of a conic curve that is a quarter of an ellipse.
const QUARTER: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The move `m` makes of a box with the border box `b` and the transform origin `origin`, in the
/// px around it: its anchor to the point along the path, turned there. A ray or a shape is in
/// `within`, the box of its containing block, with its corners' radii.
pub(super) fn moved(m: &Motion, b: Rect, origin: Point, within: (Rect, Radii)) -> Matrix4 {
    let (r, radii) = within;
    let start = |at: Option<&[Offset; 2]>| match (at, &m.position) {
        (Some(at), _) | (None, Start::At(at)) => shape::center(at, r),
        (None, Start::Auto) => [b.x, b.y],
        (None, Start::Normal) => [r.x + r.width / 2.0, r.y + r.height / 2.0],
    };
    let (point, direction) = match &m.path {
        Route::Ray { angle, size, at } => {
            let from = start(at.as_ref());
            let (sin, cos) = (angle.to_radians() as f32).sin_cos();
            let d = m.distance.resolve(reach(from, [sin, -cos], *size, r));
            ([from[0] + d * sin, from[1] - d * cos], angle - 90.0)
        }
        Route::Path(p) => along(&contours(&p.moved([b.x, b.y])), &m.distance),
        Route::Shape { shape, at } => {
            let around = (!at && !matches!(m.position, Start::Normal)).then(|| start(None));
            along(&outline(shape.as_ref(), r, radii, around), &m.distance)
        }
    };
    let anchor = match &m.anchor {
        Some(a) => [b.x + a[0].resolve(b.width), b.y + a[1].resolve(b.height)],
        None => origin,
    };
    let turn = if m.along { direction } else { 0.0 } + m.angle;
    let [p, a] = [point, anchor].map(|p| p.map(f64::from));
    Matrix4::translate(p[0], p[1], 0.0)
        .times(Matrix4::rotate([0.0, 0.0, 1.0], turn))
        .times(Matrix4::translate(-a[0], -a[1], 0.0))
}

/// How long a ray from `from` the way `d` goes is, of `size` in the box `r`.
fn reach(from: Point, d: Point, size: Size, r: Rect) -> f32 {
    // Where the box is on each axis, and how far from its start and its end `from` is.
    let span = [[r.x, r.x + r.width], [r.y, r.y + r.height]];
    let [x, y] = [0, 1].map(|k| span[k].map(|v| v - from[k]));
    let corners = (x.into_iter()).flat_map(|x| y.map(|y| x.hypot(y)));
    let sides = x.into_iter().chain(y).map(f32::abs);
    let inside = (0..2).all(|k| (span[k][0]..=span[k][1]).contains(&from[k]));
    match size {
        Size::ClosestSide => sides.fold(f32::INFINITY, f32::min),
        Size::FarthestSide => sides.fold(0.0, f32::max),
        Size::ClosestCorner => corners.fold(f32::INFINITY, f32::min),
        Size::FarthestCorner => corners.fold(0.0, f32::max),
        // To the side it leaves through, and from outside nowhere.
        Size::Sides if inside => (0..2)
            .map(|k| match d[k] {
                v if v > 0.0 => (span[k][1] - from[k]) / v,
                v if v < 0.0 => (span[k][0] - from[k]) / v,
                _ => f32::INFINITY,
            })
            .fold(f32::INFINITY, f32::min),
        Size::Sides => 0.0,
    }
}

/// The point `distance` along `contours`, a percentage of their length, and the direction there
/// in degrees clockwise from the right. The last subpath closing, it goes on around; else it
/// stops at the ends. A path of no length is its first point, to the right.
fn along(contours: &Contours, distance: &Calc) -> (Point, f64) {
    let measured: Vec<(&[Piece], Vec<Span>)> = (contours.iter())
        .map(|(_, pieces, _)| (&pieces[..], spans(pieces)))
        .filter(|(_, spans)| !spans.is_empty())
        .collect();
    let length = |spans: &[Span]| spans.last().map_or(0.0, |s| s.2);
    let total: f32 = measured.iter().map(|(_, spans)| length(spans)).sum();
    let d = distance.resolve(total);
    let d = match contours.last().is_some_and(|c| c.2) {
        true if total > 0.0 => d.rem_euclid(total),
        _ => d.clamp(0.0, total),
    };
    let mut start = 0.0;
    for (pieces, spans) in &measured {
        if d <= start + length(spans) {
            return at(pieces, spans, d - start);
        }
        start += length(spans);
    }
    (contours.first().map_or([0.0; 2], |c| c.0), 0.0)
}

/// A subpath's spans: each the index of its piece, the piece's t where it ends, of `MAX_T`, and
/// how far along the subpath it ends.
type Span = (usize, i32, f32);

/// All of a piece's t, which Skia measures in fixed point.
const MAX_T: i32 = 0x3FFF_FFFF;

/// A fixed point t as a number from 0 to 1.
fn scalar(t: i32) -> f32 {
    t as f32 * (1.0 / MAX_T as f32)
}

/// The point `d` along a subpath of `pieces` with `spans`, and the direction there, as Skia's
/// SkContourMeasure finds them, which Chrome's are: the span's t at `d` as far between its ends
/// as `d` is, on the piece.
fn at(pieces: &[Piece], spans: &[Span], d: f32) -> (Point, f64) {
    let k = spans
        .iter()
        .position(|s| s.2 >= d)
        .unwrap_or(spans.len() - 1);
    let (piece, end_t, end) = spans[k];
    let (start_t, start) = match k.checked_sub(1).map(|j| spans[j]) {
        Some((p, t, e)) if p == piece => (scalar(t), e),
        Some((_, _, e)) => (0.0, e),
        None => (0.0, 0.0),
    };
    let t = start_t + (scalar(end_t) - start_t) * (d - start) / (end - start);
    let (point, [x, y]) = eval(pieces[piece], t);
    (point, f64::from(y.atan2(x)).to_degrees())
}

/// Where `pieces` are split into lines, as Skia's SkContourMeasure splits them, which Chrome
/// measures a path's length by: a curve in halves until it is within half a px of a line.
fn spans(pieces: &[Piece]) -> Vec<Span> {
    let mut s = Spans {
        out: Vec::new(),
        distance: 0.0,
        piece: 0,
    };
    for (i, &piece) in pieces.iter().enumerate() {
        s.piece = i;
        match piece {
            Piece::Line([a, b]) => s.end(far(a, b), MAX_T),
            Piece::Quad(p) => s.quad(p, [0, MAX_T]),
            Piece::Conic(p, w) => s.conic(p, w, [0, MAX_T], [p[0], p[2]]),
            Piece::Cubic(p) => s.cubic(p, [0, MAX_T]),
        }
    }
    s.out
}

/// How far a curve may be from a line, in px on either axis.
const TOLERANCE: f32 = 0.5;

struct Spans {
    out: Vec<Span>,
    distance: f32,
    piece: usize,
}

impl Spans {
    /// A span `d` long to `t`; one of no length goes.
    fn end(&mut self, d: f32, t: i32) {
        let before = self.distance;
        self.distance += d;
        if self.distance > before {
            self.out.push((self.piece, t, self.distance));
        }
    }

    fn quad(&mut self, p: [Point; 3], [lo, hi]: [i32; 2]) {
        // How far its middle is from the middle of its ends.
        let off = [0, 1].map(|k| p[1][k] / 2.0 - (p[0][k] + p[2][k]) / 4.0);
        if splits(lo, hi) && off[0].abs().max(off[1].abs()) > TOLERANCE {
            let [ab, bc] = [mid(p[0], p[1]), mid(p[1], p[2])];
            let abc = mid(ab, bc);
            let half = (lo + hi) >> 1;
            self.quad([p[0], ab, abc], [lo, half]);
            self.quad([abc, bc, p[2]], [half, hi]);
        } else {
            self.end(far(p[0], p[2]), hi);
        }
    }

    fn cubic(&mut self, p: [Point; 4], [lo, hi]: [i32; 2]) {
        // How far its control points are from the points a third and two thirds along its ends.
        let off = |c: Point, f: f32| {
            let [x, y] = [0, 1].map(|k| p[0][k] + (p[3][k] - p[0][k]) * f - c[k]);
            x.abs().max(y.abs()) > TOLERANCE
        };
        if splits(lo, hi) && (off(p[1], 1.0 / 3.0) || off(p[2], 2.0 / 3.0)) {
            let [ab, bc, cd] = [mid(p[0], p[1]), mid(p[1], p[2]), mid(p[2], p[3])];
            let [abc, bcd] = [mid(ab, bc), mid(bc, cd)];
            let abcd = mid(abc, bcd);
            let half = (lo + hi) >> 1;
            self.cubic([p[0], ab, abc, abcd], [lo, half]);
            self.cubic([abcd, bcd, cd, p[3]], [half, hi]);
        } else {
            self.end(far(p[0], p[3]), hi);
        }
    }

    /// A conic curve from `t` `lo` to `hi`, where it is at `ends`.
    fn conic(&mut self, p: [Point; 3], w: f32, [lo, hi]: [i32; 2], ends: [Point; 2]) {
        let half = (lo + hi) >> 1;
        let middle = eval(Piece::Conic(p, w), scalar(half)).0;
        if !middle.iter().all(|v| v.is_finite()) {
            return;
        }
        let off = [0, 1].map(|k| middle[k] - (ends[0][k] + ends[1][k]) * 0.5);
        if splits(lo, hi) && off[0].abs().max(off[1].abs()) > TOLERANCE {
            self.conic(p, w, [lo, half], [ends[0], middle]);
            self.conic(p, w, [half, hi], [middle, ends[1]]);
        } else {
            self.end(far(ends[0], ends[1]), hi);
        }
    }
}

/// Whether a span of t from `lo` to `hi` may be split.
fn splits(lo: i32, hi: i32) -> bool {
    (hi - lo) >> 10 != 0
}

/// Halfway from `a` to `b`, as Skia splits curves.
fn mid(a: Point, b: Point) -> Point {
    [0, 1].map(|k| a[k] + (b[k] - a[k]) * 0.5)
}

fn far(a: Point, b: Point) -> f32 {
    let [x, y] = [b[0] - a[0], b[1] - a[1]];
    (x * x + y * y).sqrt()
}

/// A piece's point at `t` and its direction there, as Skia evaluates them: a curve's tangent
/// where it has none, at an end on a control point, the way to the next point.
fn eval(piece: Piece, t: f32) -> (Point, Point) {
    let sub = |a: Point, b: Point| [a[0] - b[0], a[1] - b[1]];
    match piece {
        Piece::Line([a, b]) => ([0, 1].map(|k| a[k] + (b[k] - a[k]) * t), sub(b, a)),
        Piece::Quad([p0, p1, p2]) => {
            let point = [0, 1].map(|k| {
                let [a, b, c] = [p2[k] - 2.0 * p1[k] + p0[k], 2.0 * (p1[k] - p0[k]), p0[k]];
                (a * t + b) * t + c
            });
            let tangent = match (t == 0.0 && p0 == p1) || (t == 1.0 && p1 == p2) {
                true => sub(p2, p0),
                false => [0, 1].map(|k| {
                    let b = p1[k] - p0[k];
                    let a = p2[k] - p1[k] - b;
                    2.0 * (a * t + b)
                }),
            };
            (point, tangent)
        }
        Piece::Conic([p0, p1, p2], w) => {
            let numer = [0, 1].map(|k| {
                let m = p1[k] * w;
                let [a, b, c] = [p2[k] - 2.0 * m + p0[k], 2.0 * (m - p0[k]), p0[k]];
                (a * t + b) * t + c
            });
            let b = 2.0 * (w - 1.0);
            let denom = (-b * t + b) * t + 1.0;
            let point = numer.map(|v| v / denom);
            let tangent = match (t == 0.0 && p0 == p1) || (t == 1.0 && p1 == p2) {
                true => sub(p2, p0),
                false => [0, 1].map(|k| {
                    let (p20, p10) = (p2[k] - p0[k], p1[k] - p0[k]);
                    let c = w * p10;
                    let a = w * p20 - p20;
                    let b = p20 - c - c;
                    (a * t + b) * t + c
                }),
            };
            (point, tangent)
        }
        Piece::Cubic([p0, p1, p2, p3]) => {
            let point = [0, 1].map(|k| {
                let a = p3[k] + 3.0 * (p1[k] - p2[k]) - p0[k];
                let b = 3.0 * (p2[k] - 2.0 * p1[k] + p0[k]);
                let c = 3.0 * (p1[k] - p0[k]);
                ((a * t + b) * t + c) * t + p0[k]
            });
            let tangent = if t == 0.0 && p0 == p1 {
                Some(sub(p2, p0))
            } else if t == 1.0 && p2 == p3 {
                Some(sub(p3, p1))
            } else {
                None
            };
            let tangent = match tangent {
                Some(v) if v == [0.0; 2] => sub(p3, p0),
                Some(v) => v,
                None => [0, 1].map(|k| {
                    let a = p3[k] + 3.0 * (p1[k] - p2[k]) - p0[k];
                    let b = 2.0 * (p2[k] - 2.0 * p1[k] + p0[k]);
                    let c = p1[k] - p0[k];
                    (a * t + b) * t + c
                }),
            };
            (point, tangent)
        }
    }
}

/// A path's subpaths, each closing one closed by a line back to its start if it is not there.
fn contours(p: &Path) -> Contours {
    let mut out: Contours = Vec::new();
    let mut at = [0.0; 2];
    for s in &p.0 {
        let piece = match *s {
            Segment::Move(q) => {
                out.push((q, Vec::new(), false));
                at = q;
                continue;
            }
            Segment::Line(q) => Some(Piece::Line([at, q])),
            Segment::Quad(c, q) => Some(Piece::Quad([at, c, q])),
            Segment::Cubic(c1, c2, q) => Some(Piece::Cubic([at, c1, c2, q])),
            Segment::Close => None,
        };
        if out.is_empty() {
            out.push((at, Vec::new(), false));
        }
        let (start, pieces, closed) = out.last_mut().expect("a subpath");
        match piece {
            Some(piece) => {
                at = end(piece);
                pieces.push(piece);
            }
            None => {
                if at != *start {
                    pieces.push(Piece::Line([at, *start]));
                }
                *closed = true;
                at = *start;
            }
        }
    }
    out
}

/// Where a piece ends.
fn end(piece: Piece) -> Point {
    match piece {
        Piece::Line([_, p]) | Piece::Quad([_, _, p]) | Piece::Conic([_, _, p], _) => p,
        Piece::Cubic([_, _, _, p]) => p,
    }
}

/// A basic shape in the box `r`, or the box itself with its corners rounded by `radii`, as Chrome
/// makes it a path: an inset or a box from its top left corner's end, a circle or an ellipse from
/// its right, around `around` if it says so, and a polygon from its first point, each clockwise
/// and closed, its corners and ellipses conic curves.
fn outline(shape: Option<&Shape>, r: Rect, radii: Radii, around: Option<Point>) -> Contours {
    let ellipse = |c: Point, [rx, ry]: [f32; 2]| {
        let [x0, y0, x1, y1] = [c[0] - rx, c[1] - ry, c[0] + rx, c[1] + ry];
        let [cx, cy] = [x0 / 2.0 + x1 / 2.0, y0 / 2.0 + y1 / 2.0];
        let ends = [[x1, cy], [cx, y1], [x0, cy], [cx, y0], [x1, cy]];
        let corners = [[x1, y1], [x0, y1], [x0, y0], [x1, y0]];
        let pieces = (0..4).map(|k| Piece::Conic([ends[k], corners[k], ends[k + 1]], QUARTER));
        vec![(ends[0], pieces.collect(), true)]
    };
    match shape {
        None => rounded(r, radii),
        Some(Shape::Inset { sides, radii }) => {
            let (inside, radii) = shape::inset(sides, radii, r);
            rounded(inside, radii)
        }
        Some(Shape::Circle { radius, at }) => {
            let c = around.unwrap_or_else(|| shape::center(at, r));
            ellipse(c, [shape::circle(radius, c, r); 2])
        }
        Some(Shape::Ellipse { radii, at }) => {
            let c = around.unwrap_or_else(|| shape::center(at, r));
            ellipse(c, shape::radii(radii, c, r))
        }
        Some(Shape::Polygon { points, .. }) => {
            let at = |[x, y]: &[Calc; 2]| [r.x + x.resolve(r.width), r.y + y.resolve(r.height)];
            let points: Vec<Point> = points.iter().map(at).collect();
            let next = points.iter().cycle().skip(1);
            let pieces = (points.iter().zip(next)).map(|(&a, &b)| Piece::Line([a, b]));
            vec![(points[0], pieces.collect(), true)]
        }
        Some(Shape::Path { path, .. }) => contours(&path.moved([r.x, r.y])),
    }
}

/// A rectangle with its corners rounded, from the end of its top left corner, clockwise:
/// sides, and corners as conic curves.
fn rounded(r: Rect, radii: Radii) -> Contours {
    let [tl, tr, br, bl] = radii;
    let [x0, y0, x1, y1] = [r.x, r.y, r.x + r.width, r.y + r.height];
    // Each side's ends, and the corner after it.
    let sides = [
        (
            [x0 + tl[0], y0],
            [x1 - tr[0], y0],
            [x1, y0],
            [x1, y0 + tr[1]],
        ),
        (
            [x1, y0 + tr[1]],
            [x1, y1 - br[1]],
            [x1, y1],
            [x1 - br[0], y1],
        ),
        (
            [x1 - br[0], y1],
            [x0 + bl[0], y1],
            [x0, y1],
            [x0, y1 - bl[1]],
        ),
        (
            [x0, y1 - bl[1]],
            [x0, y0 + tl[1]],
            [x0, y0],
            [x0 + tl[0], y0],
        ),
    ];
    let pieces = (sides.into_iter())
        .flat_map(|(a, b, corner, c)| [Piece::Line([a, b]), Piece::Conic([b, corner, c], QUARTER)]);
    vec![(sides[0].0, pieces.collect(), true)]
}
