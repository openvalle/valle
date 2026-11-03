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
    fn data_round_trips() {
        let d = "M0 0 L10.5 0 Q20 0 20 10 C20 20 0 20 0 10 Z";
        assert_eq!(data(&bez(d).unwrap()), d);
    }
}
