//! Nonlinear textPath placement: each outline point follows the path's tangent and normal.
use kurbo::{ParamCurve, PathEl};

use super::Track;
use crate::path::{Path, Segment};

impl Track {
    fn warp_point(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let at = f64::from(x).clamp(0.0, self.length);
        let Some((p, angle)) = self.point(at) else {
            return [0.0; 2];
        };
        let (sin, cos) = angle.to_radians().sin_cos();
        let beyond = f64::from(x) - at;
        [
            (p[0] + cos * beyond - sin * f64::from(y)) as f32,
            (p[1] + sin * beyond + cos * f64::from(y)) as f32,
        ]
    }

    pub(super) fn warp(&self, path: &Path) -> Path {
        if self.segments.len() == 1 && matches!(self.segments[0].0, kurbo::PathSeg::Line(_)) {
            return path.mapped(|p| self.warp_point(p));
        }
        let mut out = Path::default();
        let (mut previous, mut start) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
        kurbo::flatten(
            super::super::bez(path, 1.0).elements().iter().copied(),
            0.01,
            |el| {
                let (end, close) = match el {
                    PathEl::MoveTo(p) => {
                        out.0
                            .push(Segment::Move(self.warp_point([p.x as f32, p.y as f32])));
                        previous = p;
                        start = p;
                        return;
                    }
                    PathEl::LineTo(p) => (p, false),
                    PathEl::ClosePath => (start, true),
                    _ => unreachable!("flattened outline"),
                };
                // Even a straight glyph edge bends along a curved baseline. Sample at most one
                // user unit apart, in addition to the outline's own subpixel flattening tolerance.
                let steps = previous.distance(end).ceil().max(1.0) as usize;
                let line = kurbo::Line::new(previous, end);
                for i in 1..=steps {
                    let p = line.eval(i as f64 / steps as f64);
                    out.0
                        .push(Segment::Line(self.warp_point([p.x as f32, p.y as f32])));
                }
                if close {
                    out.0.push(Segment::Close);
                }
                previous = end;
            },
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_stretch_is_exact_and_extends_endpoint_tangents() {
        let track = Track::new(&crate::path::parse("M10 20L110 20").unwrap(), None);
        let outline = crate::path::parse("M-5 -10Q20 -40 50 -10L105 10Z").unwrap();
        assert_eq!(track.warp(&outline), outline.moved([10.0, 20.0]));
    }

    #[test]
    fn stretch_bends_the_whole_outline_and_preserves_normal_distance() {
        let track = Track::new(
            &crate::path::parse("M0 100Q100 -100 200 100").unwrap(),
            None,
        );
        for s in [0.0, track.length * 0.25, track.length * 0.5, track.length] {
            let (p, _) = track.point(s).unwrap();
            let q = track.warp_point([s as f32, -20.0]);
            assert!(((f64::from(q[0]) - p[0]).hypot(f64::from(q[1]) - p[1]) - 20.0).abs() < 0.001);
        }
        let outline = crate::path::parse("M10 0L120 0L120 -20L10 -20Z").unwrap();
        let warped = track.warp(&outline);
        assert!(warped.0.len() > 200);
        assert!(matches!(warped.0.last(), Some(Segment::Close)));
        assert_eq!(warped, track.warp(&outline));
    }
}
