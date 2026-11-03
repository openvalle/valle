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
    fn data_round_trips() {
        let d = "M0 0 L10.5 0 Q20 0 20 10 C20 20 0 20 0 10 Z";
        assert_eq!(data(&bez(d).unwrap()), d);
    }
}
