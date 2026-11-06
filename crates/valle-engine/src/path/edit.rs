//! The path functions scenes call (`pathLength`, `pointAt`, …): SVG path data in, numbers or SVG
//! path data out, through kurbo's curves.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};
use kurbo::{BezPath, ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg, Vec2};

use super::{Path, Segment, parse};
use crate::js;

/// How close a measured length comes to the exact one, in path units.
const ACCURACY: f64 = 1e-6;

/// Path data as kurbo's elements.
pub fn bez(d: &str) -> Result<BezPath, String> {
    let to = |p: [f32; 2]| kurbo::Point::new(f64::from(p[0]), f64::from(p[1]));
    let Path(segments) = parse(d)?;
    Ok(segments
        .iter()
        .map(|s| match *s {
            Segment::Move(p) => PathEl::MoveTo(to(p)),
            Segment::Line(p) => PathEl::LineTo(to(p)),
            Segment::Quad(c, p) => PathEl::QuadTo(to(c), to(p)),
            Segment::Cubic(c1, c2, p) => PathEl::CurveTo(to(c1), to(c2), to(p)),
            Segment::Close => PathEl::ClosePath,
        })
        .collect())
}

/// Path data for kurbo's elements, absolute, each number to a thousandth.
pub fn data(path: &BezPath) -> String {
    let n = |x: f64| js::number_to_string((x * 1000.0).round() / 1000.0 + 0.0);
    let p = |p: kurbo::Point| format!("{} {}", n(p.x), n(p.y));
    let parts: Vec<String> = (path.elements().iter())
        .map(|el| match *el {
            PathEl::MoveTo(a) => format!("M{}", p(a)),
            PathEl::LineTo(a) => format!("L{}", p(a)),
            PathEl::QuadTo(a, b) => format!("Q{} {}", p(a), p(b)),
            PathEl::CurveTo(a, b, c) => format!("C{} {} {}", p(a), p(b), p(c)),
            PathEl::ClosePath => "Z".to_string(),
        })
        .collect();
    parts.join(" ")
}

/// The path's length: its segments', a subpath's closing line included, moves not.
pub fn length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(ACCURACY)).sum()
}

/// The point at `progress` of the path's length, clamped to [0, 1], and the unit direction
/// there. A path without length gives its first point, facing right.
pub fn at(path: &BezPath, progress: f64) -> (kurbo::Point, Vec2) {
    let first = match path.elements().first() {
        Some(PathEl::MoveTo(p)) => *p,
        _ => kurbo::Point::ZERO,
    };
    let segments: Vec<(PathSeg, f64)> = (path.segments())
        .map(|s| (s, s.arclen(ACCURACY)))
        .filter(|(_, l)| *l > 0.0)
        .collect();
    let total: f64 = segments.iter().map(|s| s.1).sum();
    if total <= 0.0 || progress.is_nan() {
        return (first, Vec2::new(1.0, 0.0));
    }
    let mut left = progress.clamp(0.0, 1.0) * total;
    for (i, (segment, l)) in segments.iter().enumerate() {
        if left <= *l || i + 1 == segments.len() {
            let t = segment.inv_arclen(left.min(*l), ACCURACY);
            return (segment.eval(t), direction(segment, t));
        }
        left -= l;
    }
    unreachable!("a path with length has a segment")
}

/// A path's subpaths as segments, a closing line made a segment, and whether each is closed.
/// A subpath of only a move keeps its point.
fn subpaths(path: &BezPath) -> Vec<(Vec<PathSeg>, bool, kurbo::Point)> {
    let mut out: Vec<(Vec<PathSeg>, bool, kurbo::Point)> = Vec::new();
    let (mut start, mut at) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
    // A segment after a close starts a subpath where that one started.
    let open = |out: &mut Vec<(Vec<PathSeg>, bool, kurbo::Point)>, p: kurbo::Point| {
        if out.last().is_none_or(|s| s.1) {
            out.push((Vec::new(), false, p));
        }
    };
    for el in path.elements() {
        let segment = match *el {
            PathEl::MoveTo(p) => {
                (start, at) = (p, p);
                out.push((Vec::new(), false, p));
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(kurbo::Line::new(at, p)),
            PathEl::QuadTo(c, p) => PathSeg::Quad(kurbo::QuadBez::new(at, c, p)),
            PathEl::CurveTo(c1, c2, p) => PathSeg::Cubic(kurbo::CubicBez::new(at, c1, c2, p)),
            PathEl::ClosePath => {
                open(&mut out, start);
                let last = out.last_mut().expect("opened");
                if at != start {
                    last.0.push(PathSeg::Line(kurbo::Line::new(at, start)));
                }
                last.1 = true;
                at = start;
                continue;
            }
        };
        open(&mut out, at);
        out.last_mut().expect("opened").0.push(segment);
        at = segment.end();
    }
    out
}

/// `roundCorners(path, radius)`: each corner, where two segments meet at an angle, cut back on
/// both and joined by a curve: a circular arc of `radius` between lines, smaller where a segment
/// is too short for it (at most half of each is cut). The ends of an open subpath stay.
pub fn round_corners(path: &BezPath, radius: f64) -> BezPath {
    if radius.is_nan() || radius <= 0.0 {
        return path.clone();
    }
    let mut out = BezPath::new();
    for (segments, closed, first) in subpaths(path) {
        let n = segments.len();
        if n == 0 {
            out.move_to(first);
            continue;
        }
        let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
        // The corner before each segment: the cut on both sides, and the turn.
        let corners: Vec<Option<(f64, f64)>> = (0..n)
            .map(|i| {
                if i == 0 && !closed {
                    return None;
                }
                let before = (i + n - 1) % n;
                let (a, b) = (
                    direction(&segments[before], 1.0),
                    direction(&segments[i], 0.0),
                );
                let turn = a.cross(b).atan2(a.dot(b)).abs();
                if turn < 1e-6 {
                    return None;
                }
                let cut = (radius * (turn / 2.0).tan())
                    .min(lengths[before] / 2.0)
                    .min(lengths[i] / 2.0);
                (cut > 0.0).then_some((cut, turn))
            })
            .collect();
        // Each segment's part left between its corners.
        let span = |i: usize| {
            let s = &segments[i];
            let cut_start = corners[i].map_or(0.0, |c| c.0);
            let cut_end = corners[(i + 1) % n]
                .filter(|_| closed || i + 1 < n)
                .map_or(0.0, |c| c.0);
            let t0 = if cut_start > 0.0 {
                s.inv_arclen(cut_start, ACCURACY)
            } else {
                0.0
            };
            let t1 = match cut_end {
                0.0 => 1.0,
                cut => s.inv_arclen(lengths[i] - cut, ACCURACY),
            };
            (s.subsegment(t0..t1.max(t0)), t0, t1.max(t0))
        };
        let spans: Vec<(PathSeg, f64, f64)> = (0..n).map(span).collect();
        let parts: Vec<PathSeg> = spans.iter().map(|s| s.0).collect();
        out.move_to(parts[0].start());
        for i in 0..n {
            push(&mut out, parts[i]);
            let next = (i + 1) % n;
            if next == 0 && !closed {
                break;
            }
            if let Some((cut, turn)) = corners[next] {
                let (from, to) = (parts[i].end(), parts[next].start());
                // From the whole segments: a part may have no length left.
                let a = direction(&segments[i], spans[i].2);
                let b = direction(&segments[next], spans[next].1);
                // A circular arc's handles, for the radius the cut makes.
                let handle = cut * 4.0 / 3.0 * (turn / 4.0).tan() / (turn / 2.0).tan();
                out.curve_to(from + a * handle, to - b * handle, to);
            }
        }
        if closed {
            out.close_path();
        }
    }
    out
}

/// A subpath as cubic segments, a line one whose handles sit on its ends, as AE's and Lottie's
/// paths are: vertices with handles.
fn cubics(segments: &[PathSeg]) -> Vec<kurbo::CubicBez> {
    (segments.iter())
        .map(|s| match *s {
            PathSeg::Line(l) => kurbo::CubicBez::new(l.p0, l.p0, l.p1, l.p1),
            PathSeg::Quad(q) => q.raise(),
            PathSeg::Cubic(c) => c,
        })
        .collect()
}

/// Each subpath's cubics moved point by point by `f(point, subpath's cubics)`, or with
/// `pieces` cubics a segment first, so a bend shows inside a segment.
fn moved(
    path: &BezPath,
    pieces: usize,
    f: impl Fn(kurbo::Point, &[kurbo::CubicBez], bool) -> kurbo::Point,
) -> BezPath {
    let mut out = BezPath::new();
    for (segments, closed, first) in subpaths(path) {
        let mut list = cubics(&segments);
        if pieces > 1 {
            let k = pieces as f64;
            list = (list.iter())
                .flat_map(|c| {
                    (0..pieces).map(move |i| c.subsegment(i as f64 / k..(i + 1) as f64 / k))
                })
                .collect();
        }
        let Some(start) = list.first() else {
            out.move_to(f(first, &list, false));
            continue;
        };
        out.move_to(f(start.p0, &list, false));
        for c in &list {
            // The handles, even where they sit on the vertices, as a line's do.
            let (a, b) = (f(c.p1, &list, true), f(c.p2, &list, true));
            out.curve_to(a, b, f(c.p3, &list, false));
        }
        if closed {
            out.close_path();
        }
    }
    out
}

/// The mean of a subpath's vertices.
fn center(list: &[kurbo::CubicBez]) -> kurbo::Point {
    let sum = list.iter().fold(Vec2::ZERO, |s, c| s + c.p0.to_vec2());
    (sum / list.len().max(1) as f64).to_point()
}

/// `puckerBloat(path, amount)`, as AE and Lottie do it: each subpath's vertices pulled toward
/// their mean by `amount` (a fraction, 1 for 100%) and its handles pushed away by as much, so a
/// positive amount bloats the segments out between vertices drawn in, a negative one puckers.
pub fn pucker_bloat(path: &BezPath, amount: f64) -> BezPath {
    moved(path, 1, |p, list, handle| {
        let c = center(list);
        let k = if handle { -amount } else { amount };
        p + (c - p) * k
    })
}

/// `twist(path, angle)`: the path turned about its bounding box's center, by `angle` degrees at
/// the center and less going out, none at the farthest point. Each segment is cut into pieces
/// first so the turn bends it.
pub fn twist(path: &BezPath, angle: f64) -> BezPath {
    let bounds = kurbo::Shape::bounding_box(path);
    let c = bounds.center();
    let far = (path.elements().iter())
        .flat_map(|el| el.end_point())
        .map(|p| (p - c).hypot())
        .fold(0.0f64, f64::max);
    if far <= 0.0 {
        return path.clone();
    }
    moved(path, 8, |p, _, _| {
        let turn = angle.to_radians() * (1.0 - ((p - c).hypot() / far).min(1.0));
        kurbo::Affine::rotate_about(turn, c) * p
    })
}

/// `zigzag(path, size, ridges)`: each subpath as a line through its vertices and `ridges` points
/// a segment between them, spread evenly along it, each moved `size` off the path to alternate
/// sides, as AE's Zig Zag with corner points.
pub fn zigzag(path: &BezPath, size: f64, ridges: usize) -> BezPath {
    let mut out = BezPath::new();
    for (segments, closed, first) in subpaths(path) {
        let mut points: Vec<(kurbo::Point, Vec2)> = Vec::new();
        for (i, s) in segments.iter().enumerate() {
            let length = s.arclen(ACCURACY);
            // The vertex at the start: off along the bisector of the two normals.
            let before = match i {
                0 if !closed => None,
                0 => segments.last(),
                _ => segments.get(i - 1),
            };
            let normal = |d: Vec2| Vec2::new(-d.y, d.x);
            let outward = normal(direction(s, 0.0));
            let n = match before {
                Some(b) => {
                    let m = normal(direction(b, 1.0)) + outward;
                    if m.hypot() > 1e-9 {
                        m / m.hypot()
                    } else {
                        outward
                    }
                }
                None => outward,
            };
            points.push((s.start(), n));
            for k in 1..=ridges {
                let t = s.inv_arclen(length * k as f64 / (ridges + 1) as f64, ACCURACY);
                points.push((s.eval(t), normal(direction(s, t))));
            }
        }
        if let (Some(last), false) = (segments.last(), closed) {
            let d = direction(last, 1.0);
            points.push((last.end(), Vec2::new(-d.y, d.x)));
        }
        let Some(&(p, n)) = points.first() else {
            out.move_to(first);
            continue;
        };
        out.move_to(p + n * size);
        for (i, &(p, n)) in points.iter().enumerate().skip(1) {
            let side = if i % 2 == 0 { size } else { -size };
            out.line_to(p + n * side);
        }
        if closed {
            out.close_path();
        }
    }
    out
}

/// How far a flattened curve may stray from the curve, in path units.
const FLAT: f64 = 0.02;

/// The path's subpaths flattened to polygons, each closed, for i_overlay. An open subpath counts
/// as closed, as a fill takes it.
fn polygons(path: &BezPath) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), FLAT, |el| match el {
        PathEl::MoveTo(p) => out.push(vec![[p.x, p.y]]),
        PathEl::LineTo(p) => {
            if let Some(last) = out.last_mut() {
                last.push([p.x, p.y]);
            }
        }
        _ => {}
    });
    for p in &mut out {
        if p.len() > 1 && p.first() == p.last() {
            p.pop();
        }
    }
    out.retain(|p| p.len() >= 3);
    out
}

/// Path data of i_overlay's shapes: an outline and its holes each.
fn shaped(shapes: Vec<Vec<Vec<[f64; 2]>>>) -> BezPath {
    let mut out = BezPath::new();
    for contour in shapes.iter().flatten().filter(|c| c.len() >= 3) {
        out.move_to((contour[0][0], contour[0][1]));
        for p in &contour[1..] {
            out.line_to((p[0], p[1]));
        }
        out.close_path();
    }
    out
}

/// How `inflatePath` joins the sides it moves out: AE's Offset Paths' joins, a miter cut off where
/// it would reach past `limit` times the distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    Miter(f64),
    Round,
    Bevel,
}

/// `inflatePath(path, distance)`: the filled shape (non-zero) grown by `distance`, or shrunk by a
/// negative one, as AE's Offset Paths and Clipper2's `InflatePaths`; curves are flattened.
pub fn inflate(path: &BezPath, distance: f64, join: Join) -> BezPath {
    let shapes = polygons(path).simplify_shape(FillRule::NonZero);
    if distance == 0.0 || !distance.is_finite() {
        return shaped(shapes);
    }
    let join = match join {
        // The miter limit as the sharpest angle it keeps: a ratio of 1 / sin(angle / 2).
        Join::Miter(limit) => LineJoin::Miter(2.0 * (1.0 / limit.max(1.0)).asin()),
        // Arcs in pieces at most FLAT off the circle.
        Join::Round => {
            LineJoin::Round(2.0 * (1.0 - FLAT / distance.abs()).max(0.0).acos().max(0.01))
        }
        Join::Bevel => LineJoin::Bevel,
    };
    shaped(shapes.outline(&OutlineStyle::new(distance).line_join(join)))
}

/// `pathBoolean(a, b, op)` of the two filled shapes (non-zero): `union`, `intersect`, `subtract`
/// (b from a) or `xor`; curves are flattened.
pub fn boolean(a: &BezPath, b: &BezPath, op: &str) -> Result<BezPath, String> {
    let rule = match op {
        "union" => OverlayRule::Union,
        "intersect" => OverlayRule::Intersect,
        "subtract" => OverlayRule::Difference,
        "xor" => OverlayRule::Xor,
        _ => {
            return Err(format!(
                "pathBoolean op {op} is not union, intersect, subtract or xor"
            ));
        }
    };
    Ok(shaped(polygons(a).overlay(
        &polygons(b),
        rule,
        FillRule::NonZero,
    )))
}

/// `morph(from, to, progress, { samples })`: `from` at 0, `to` at 1, and between them:
///
/// - Paths of the same commands, segment for segment, move point by point, their curves kept.
/// - Others are resampled: each subpath to `samples` points evenly along it (by default one every
///   2 units, 16 to 256), paired with the other's by size, the largest with the largest, a subpath
///   left over shrinking to its center. A closed pair turns the same way round and starts where
///   the points move least; an open one may run backwards for that.
pub fn morph(from: &BezPath, to: &BezPath, progress: f64, samples: Option<usize>) -> BezPath {
    let t = if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0)
    };
    if t == 0.0 {
        return from.clone();
    }
    if t == 1.0 {
        return to.clone();
    }
    let (a, b) = (subpaths(from), subpaths(to));
    let kind = |s: &PathSeg| std::mem::discriminant(s);
    let alike = a.len() == b.len()
        && a.iter().zip(&b).all(|(p, q)| {
            let kinds = p.0.iter().zip(&q.0).all(|(x, y)| kind(x) == kind(y));
            p.1 == q.1 && p.0.len() == q.0.len() && kinds
        });
    let lerp = |p: kurbo::Point, q: kurbo::Point| p.lerp(q, t);
    let mut out = BezPath::new();
    if alike {
        for ((p, closed, p0), (q, _, q0)) in a.iter().zip(&b) {
            let (p, q) = (cubics(p), cubics(q));
            let start = |list: &[kurbo::CubicBez], first| list.first().map_or(first, |c| c.p0);
            out.move_to(lerp(start(&p, *p0), start(&q, *q0)));
            for (c, d) in p.iter().zip(&q) {
                out.curve_to(lerp(c.p1, d.p1), lerp(c.p2, d.p2), lerp(c.p3, d.p3));
            }
            if *closed {
                out.close_path();
            }
        }
        return out;
    }
    let size = |s: &(Vec<PathSeg>, bool, kurbo::Point)| {
        let points = resampled(&s.0, s.2, s.1, 32);
        signed_area(&points).abs()
    };
    let by_size = |list: &[(Vec<PathSeg>, bool, kurbo::Point)]| {
        let mut order: Vec<usize> = (0..list.len()).collect();
        order.sort_by(|&i, &j| size(&list[j]).total_cmp(&size(&list[i])));
        order
    };
    let (oa, ob) = (by_size(&a), by_size(&b));
    for k in 0..oa.len().max(ob.len()) {
        let (p, q) = (oa.get(k).map(|&i| &a[i]), ob.get(k).map(|&i| &b[i]));
        let length = |s: Option<&(Vec<PathSeg>, bool, kurbo::Point)>| {
            s.map_or(0.0, |s| s.0.iter().map(|g| g.arclen(ACCURACY)).sum::<f64>())
        };
        let n = samples
            .unwrap_or_else(|| ((length(p).max(length(q)) / 2.0).ceil() as usize).clamp(16, 256));
        let points = |s: Option<&(Vec<PathSeg>, bool, kurbo::Point)>| {
            s.map(|s| (resampled(&s.0, s.2, s.1, n), s.1))
        };
        let (mut pa, mut pb) = (points(p), points(q));
        // A subpath with no partner shrinks to, or grows from, its own center.
        let point = |list: &(Vec<kurbo::Point>, bool)| {
            let c = list.0.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / n as f64;
            (vec![c.to_point(); n], list.1)
        };
        let (pa, pb) = match (pa.take(), pb.take()) {
            (Some(x), Some(y)) => (x, y),
            (Some(x), None) => (x.clone(), point(&x)),
            (None, Some(y)) => (point(&y), y),
            (None, None) => continue,
        };
        let pb = aligned(&pa, pb);
        out.move_to(lerp(pa.0[0], pb.0[0]));
        for (p, q) in pa.0.iter().zip(&pb.0).skip(1) {
            out.line_to(lerp(*p, *q));
        }
        let closed = if t < 0.5 { pa.1 } else { pb.1 };
        if closed {
            out.close_path();
        }
    }
    out
}

/// `n` points evenly along a subpath: round a closed one, its start not repeated, or from end to
/// end of an open one.
fn resampled(
    segments: &[PathSeg],
    first: kurbo::Point,
    closed: bool,
    n: usize,
) -> Vec<kurbo::Point> {
    let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if total <= 0.0 {
        return vec![segments.first().map_or(first, |s| s.start()); n];
    }
    let gaps = if closed { n } else { n.max(2) - 1 };
    let (mut i, mut before) = (0, 0.0);
    (0..n)
        .map(|k| {
            let d = total * k as f64 / gaps as f64;
            while i + 1 < segments.len() && before + lengths[i] < d {
                before += lengths[i];
                i += 1;
            }
            let s = &segments[i];
            let into = (d - before).clamp(0.0, lengths[i]);
            s.eval(if lengths[i] > 0.0 {
                s.inv_arclen(into, ACCURACY)
            } else {
                0.0
            })
        })
        .collect()
}

/// Twice the area a polygon's points wind round, positive clockwise on screen.
fn signed_area(points: &[kurbo::Point]) -> f64 {
    (0..points.len())
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % points.len()]);
            p.x * q.y - q.x * p.y
        })
        .sum()
}

/// `to`'s points ordered to move least from `from`'s: a closed pair turned the same way round and
/// started at the best point, an open pair run forwards or backwards.
fn aligned(
    from: &(Vec<kurbo::Point>, bool),
    mut to: (Vec<kurbo::Point>, bool),
) -> (Vec<kurbo::Point>, bool) {
    let n = from.0.len();
    let cost = |list: &[kurbo::Point], shift: usize| -> f64 {
        (0..n)
            .map(|i| (from.0[i] - list[(i + shift) % n]).hypot2())
            .sum()
    };
    if from.1 && to.1 {
        if signed_area(&from.0) * signed_area(&to.0) < 0.0 {
            to.0.reverse();
        }
        let best = (0..n)
            .min_by(|&i, &j| cost(&to.0, i).total_cmp(&cost(&to.0, j)))
            .unwrap_or(0);
        to.0.rotate_left(best);
    } else {
        let mut back = to.0.clone();
        back.reverse();
        if cost(&back, 0) < cost(&to.0, 0) {
            to.0 = back;
        }
    }
    to
}

/// Appends a segment from where the path is.
fn push(out: &mut BezPath, s: PathSeg) {
    match s {
        PathSeg::Line(l) => out.line_to(l.p1),
        PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
        PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
    }
}

/// The unit direction along a segment at `t`, from its chord where the derivative vanishes, as at
/// a cubic's end whose control point sits on it.
fn direction(segment: &PathSeg, t: f64) -> Vec2 {
    let d = match segment {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
    };
    let d = if d.hypot() > 1e-12 {
        d
    } else {
        segment.end() - segment.start()
    };
    match d.hypot() {
        l if l > 0.0 => d / l,
        _ => Vec2::new(1.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_follow_the_path() {
        let p = bez("M0 0 L10 0 L10 10").unwrap();
        assert_eq!(
            at(&p, 0.25),
            (kurbo::Point::new(5.0, 0.0), Vec2::new(1.0, 0.0))
        );
        assert_eq!(
            at(&p, 0.75),
            (kurbo::Point::new(10.0, 5.0), Vec2::new(0.0, 1.0))
        );
        assert_eq!(at(&p, 7.0).0, kurbo::Point::new(10.0, 10.0));
        // A cubic whose first control point is its start still faces along it.
        let c = bez("M0 0 C0 0 10 0 10 0").unwrap();
        assert_eq!(at(&c, 0.0).1, Vec2::new(1.0, 0.0));
        assert_eq!(
            at(&bez("M3 4").unwrap(), 0.5).0,
            kurbo::Point::new(3.0, 4.0)
        );
    }

    #[test]
    fn corners_round_to_their_radius() {
        let square = bez("M0 0 H100 V100 H0 Z").unwrap();
        let round = round_corners(&square, 10.0);
        // Four sides less two radii each, and a circle of the radius (a cubic's within 0.03%).
        let expected = 4.0 * 80.0 + std::f64::consts::TAU * 10.0;
        assert!(
            (length(&round) - expected).abs() < 0.01,
            "{}",
            length(&round)
        );
        assert!(data(&round).starts_with("M10 0 L90 0 C95.523 0 100 4.477 100 10 L100 90"));
        // Too big a radius rounds as much as the sides allow: a circle.
        let circle = round_corners(&square, 1000.0);
        let l = length(&circle);
        assert!((l - std::f64::consts::PI * 100.0).abs() < 0.1, "{l}");
        // An open path keeps its ends; a smooth join stays as it is.
        let open = round_corners(&bez("M0 0 L50 0 L50 50").unwrap(), 5.0);
        assert!(data(&open).starts_with("M0 0 L45 0 C"));
        assert!(data(&open).ends_with("L50 50"));
        let smooth = bez("M0 0 C50 0 50 0 100 0 C150 0 150 0 200 0").unwrap();
        assert_eq!(data(&round_corners(&smooth, 5.0)), data(&smooth));
        assert_eq!(data(&round_corners(&square, 0.0)), data(&square));
    }

    #[test]
    fn pucker_and_bloat_move_vertices_and_handles() {
        let square = bez("M0 0 H100 V100 H0 Z").unwrap();
        let bloat = data(&pucker_bloat(&square, 0.5));
        // Vertices halfway in, handles halfway out from the center.
        assert!(
            bloat.starts_with("M25 25 C-25 -25 125 -25 75 25"),
            "{bloat}"
        );
        assert_eq!(
            data(&pucker_bloat(&square, 0.0)),
            "M0 0 C0 0 100 0 100 0 C100 0 100 100 100 100 C100 100 0 100 0 100 C0 100 0 0 0 0 Z"
        );
    }

    #[test]
    fn twists_turn_the_middle_most() {
        let line = bez("M0 50 L100 50").unwrap();
        // The ends are the farthest, so they stay; the middle turns with the center.
        let t = twist(&line, 90.0);
        let (start, mid) = (at(&t, 0.0).0, at(&t, 0.5).0);
        assert!((start - kurbo::Point::new(0.0, 50.0)).hypot() < 1e-9);
        assert!((mid - kurbo::Point::new(50.0, 50.0)).hypot() < 1e-6);
        assert!((at(&t, 0.25).0.y - 50.0).abs() > 5.0);
    }

    #[test]
    fn zigzags_alternate_sides() {
        let z = data(&zigzag(&bez("M0 0 L100 0").unwrap(), 10.0, 3));
        assert_eq!(z, "M0 10 L25 -10 L50 10 L75 -10 L100 10");
        let closed = data(&zigzag(&bez("M0 0 H100 V100 H0 Z").unwrap(), 5.0, 1));
        assert_eq!(closed.matches('L').count(), 7, "{closed}");
    }

    /// The area a path fills, non-zero, by its flattened contours' signed areas.
    fn area(path: &BezPath) -> f64 {
        kurbo::Shape::area(path).abs()
    }

    #[test]
    fn booleans_add_up() {
        let a = bez("M0 0 H100 V100 H0 Z").unwrap();
        let b = bez("M50 50 H150 V150 H50 Z").unwrap();
        let of = |op| area(&boolean(&a, &b, op).unwrap());
        assert!((of("union") - 17500.0).abs() < 1e-6);
        assert!((of("intersect") - 2500.0).abs() < 1e-6);
        assert!((of("subtract") - 7500.0).abs() < 1e-6);
        assert!((of("xor") - 15000.0).abs() < 1e-6);
        // A circle less a smaller one inside: a ring, its hole the other way round.
        let ring = boolean(
            &bez("M0 50 A50 50 0 1 1 100 50 A50 50 0 1 1 0 50 Z").unwrap(),
            &bez("M25 50 A25 25 0 1 1 75 50 A25 25 0 1 1 25 50 Z").unwrap(),
            "subtract",
        )
        .unwrap();
        let expected = std::f64::consts::PI * (2500.0 - 625.0);
        assert!(
            (area(&ring) - expected).abs() < expected * 1e-3,
            "{}",
            area(&ring)
        );
        assert!(boolean(&a, &b, "difference").is_err());
    }

    #[test]
    fn inflated_shapes_grow_by_the_distance() {
        let square = bez("M0 0 H100 V100 H0 Z").unwrap();
        let bevel = |d| area(&inflate(&square, d, Join::Miter(4.0)));
        assert!((bevel(10.0) - 120.0 * 120.0).abs() < 1e-6);
        assert!((bevel(-10.0) - 80.0 * 80.0).abs() < 1e-6);
        let round = area(&inflate(&square, 10.0, Join::Round));
        let expected = 10000.0 + 4.0 * 1000.0 + std::f64::consts::PI * 100.0;
        assert!((round - expected).abs() < 2.0, "{round}");
        let cut = area(&inflate(&square, 10.0, Join::Bevel));
        assert!((cut - (14400.0 - 4.0 * 50.0)).abs() < 1e-6, "{cut}");
        // Wound the other way, the same shape.
        let back = bez("M0 0 V100 H100 V0 Z").unwrap();
        assert!((area(&inflate(&back, 10.0, Join::Miter(4.0))) - 14400.0).abs() < 1e-6);
        assert!(area(&inflate(&square, -60.0, Join::Miter(4.0))) < 1e-9);
    }

    #[test]
    fn alike_paths_morph_point_by_point() {
        let a = bez("M0 0 L100 0 Q100 50 50 50 Z").unwrap();
        let b = bez("M0 100 L100 100 Q100 0 50 150 Z").unwrap();
        let half = data(&morph(&a, &b, 0.5, None));
        assert!(half.starts_with("M0 50 C0 50 100 50 100 50 C"), "{half}");
        assert_eq!(data(&morph(&a, &b, 0.0, None)), data(&a));
        assert_eq!(data(&morph(&a, &b, 7.0, None)), data(&b));
    }

    #[test]
    fn other_paths_are_resampled_and_aligned() {
        let square = bez("M0 0 H100 V100 H0 Z").unwrap();
        // The same square started at another corner and wound the other way: nothing moves.
        let other = bez("M100 100 V0 H0 V100 H100 V100 Z").unwrap();
        let mid = morph(&square, &other, 0.5, Some(64));
        assert!((area(&mid) - 10000.0).abs() < 1.0, "{}", area(&mid));
        // A circle into a square stays about as big as both, never folding in.
        let circle = bez("M0 50 A50 50 0 1 1 100 50 A50 50 0 1 1 0 50 Z").unwrap();
        let blend = area(&morph(&circle, &square, 0.5, None));
        assert!(blend > 7800.0 && blend < 10000.0, "{blend}");
        // A subpath with no partner shrinks to its center.
        let two = bez("M0 0 H100 V100 H0 Z M200 0 H220 V20 H200 Z").unwrap();
        let near = morph(&two, &square, 0.99, None);
        assert!(area(&near) < 10000.0 + 1.0);
    }

    #[test]
    fn data_round_trips() {
        let d = "M0 0 L10.5 0 Q20 0 20 10 C20 20 0 20 0 10 Z";
        assert_eq!(data(&bez(d).unwrap()), d);
    }
}
