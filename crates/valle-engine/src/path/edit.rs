//! The path functions scenes call (`pathLength`, `pointAt`, …): SVG path data in, numbers or SVG
//! path data out, through kurbo's curves.

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

    /// What the pinned Chrome measures: each path's total length and its points at tenths of it.
    #[test]
    fn paths_measure_as_chrome_measures_them() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/baselines/chrome/properties.json"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let list: serde_json::Value = serde_json::from_str(&text).unwrap();
        let paths = list["paths"].as_object().unwrap();
        assert!(paths.len() > 8);
        for (d, chrome) in paths {
            let p = bez(d).unwrap();
            let total = chrome["length"].as_f64().unwrap();
            let ours = length(&p);
            assert!(
                (ours - total).abs() <= 1e-3 * total.max(1.0),
                "{d}: {ours}, {total}"
            );
            for (i, point) in chrome["points"].as_array().unwrap().iter().enumerate() {
                let (x, y) = (point[0].as_f64().unwrap(), point[1].as_f64().unwrap());
                let (q, _) = at(&p, i as f64 / 10.0);
                let off = (q.x - x).hypot(q.y - y);
                assert!(off <= 0.05 + 1e-3 * total, "{d} at {i}/10: {q:?}, {x} {y}");
            }
        }
    }

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

    #[test]
    fn data_round_trips() {
        let d = "M0 0 L10.5 0 Q20 0 20 10 C20 20 0 20 0 10 Z";
        assert_eq!(data(&bez(d).unwrap()), d);
    }
}
