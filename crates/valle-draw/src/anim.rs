//! Geometry driven by normalized progress. Arc-length trimming and shared timing windows keep
//! rendering and path-following anchors aligned; easing belongs to the caller.

use crate::geom::{Point, Vec2};
use crate::math;

/// Map progress into [start, end], clamped to 0..1 outside the window.
pub fn window(p: f64, start: f64, end: f64) -> f64 {
    ((p - start) / (end - start).max(1e-9)).clamp(0.0, 1.0)
}

/// Staggered progress for element i of n, with each element occupying the slot fraction of the
/// window.
pub fn staggered(p: f64, start: f64, end: f64, i: usize, n: usize, slot: f64) -> f64 {
    let w = window(p, start, end);
    let lead = (1.0 - slot).max(0.0);
    let offset = if n > 1 {
        lead * i as f64 / (n - 1) as f64
    } else {
        0.0
    };
    ((w - offset) / slot.max(1e-9)).clamp(0.0, 1.0)
}

/// Polyline arc-length lookup table built once for per-frame trimming and interpolation.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArcLength {
    /// Cumulative length at point i; cum[0] is zero.
    cum: Vec<f64>,
}

impl ArcLength {
    pub fn of(points: &[Point]) -> ArcLength {
        let mut cum = Vec::with_capacity(points.len());
        let mut acc = 0.0;
        cum.push(0.0);
        for w in points.windows(2) {
            acc += dist(w[0], w[1]);
            cum.push(acc);
        }
        ArcLength { cum }
    }

    pub fn total(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Return the prefix at normalized arc length t, interpolating its tip. At t >= 1 or for
    /// degenerate paths, copy the original points.
    pub fn trim(&self, points: &[Point], t: f64) -> Vec<Point> {
        let total = self.total();
        if t >= 1.0 || points.len() < 2 || total <= 0.0 {
            return points.to_vec();
        }
        if t <= 0.0 {
            return Vec::new();
        }
        let target = total * t;
        // Find the first vertex beyond the target length.
        let i = self.cum.partition_point(|&c| c <= target);
        let mut out = points[..i].to_vec();
        // Interpolate the tip within segment [i-1, i]; positive t guarantees i >= 1.
        let (c0, c1) = (self.cum[i - 1], self.cum[i]);
        let seg = (c1 - c0).max(1e-12);
        let k = (target - c0) / seg;
        out.push(lerp(points[i - 1], points[i], k));
        out
    }

    /// Return the point and unit tangent at normalized arc length s.
    pub fn point_at(&self, points: &[Point], s: f64) -> (Point, Vec2) {
        if points.is_empty() {
            return (Point::new(0.0, 0.0), Vec2::RIGHT);
        }
        if points.len() < 2 || self.total() <= 0.0 {
            return (points[0], Vec2::RIGHT);
        }
        let target = self.total() * s.clamp(0.0, 1.0);
        let i = self
            .cum
            .partition_point(|&c| c <= target)
            .min(points.len() - 1);
        let (a, b) = (points[i - 1], points[i]);
        let seg = (self.cum[i] - self.cum[i - 1]).max(1e-12);
        let k = (target - self.cum[i - 1]) / seg;
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = math::sqrt(dx * dx + dy * dy).max(1e-12);
        (lerp(a, b, k), Vec2::new(dx / len, dy / len))
    }
}

/// Flatten a quadratic Bezier with its control point offset from the midpoint by curveness times
/// chord length along the normal. Zero curvature returns a straight segment; ArcLength provides
/// uniform trimming speed.
pub fn flatten_curve(from: Point, to: Point, curveness: f64, steps: usize) -> Vec<Point> {
    if curveness.abs() < 1e-9 {
        return vec![from, to];
    }
    let mid = Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let ctrl = Point::new(mid.x - dy * curveness, mid.y + dx * curveness);
    let n = steps.max(2);
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            Point::new(
                u * u * from.x + 2.0 * u * t * ctrl.x + t * t * to.x,
                u * u * from.y + 2.0 * u * t * ctrl.y + t * t * to.y,
            )
        })
        .collect()
}

fn dist(a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    math::sqrt(dx * dx + dy * dy)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_is_arc_length_not_vertex_count() {
        // Uneven vertex density must not shift the halfway point of an equal-leg L shape.
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(2.5, 0.0),
            Point::new(5.0, 0.0),
            Point::new(7.5, 0.0),
            Point::new(10.0, 0.0),  // Corner after four segments.
            Point::new(10.0, 10.0), // One segment on the second leg.
        ];
        let arc = ArcLength::of(&pts);
        assert!((arc.total() - 20.0).abs() < 1e-12);
        let half = arc.trim(&pts, 0.5);
        assert_eq!(*half.last().unwrap(), Point::new(10.0, 0.0), "{half:?}");
        // Three-quarter progress reaches the second leg's midpoint.
        let t34 = arc.trim(&pts, 0.75);
        assert_eq!(*t34.last().unwrap(), Point::new(10.0, 5.0));
        // Zero progress is empty; full progress preserves the path.
        assert!(arc.trim(&pts, 0.0).is_empty());
        assert_eq!(arc.trim(&pts, 1.0), pts);
    }

    #[test]
    fn point_at_gives_position_and_unit_tangent() {
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
        ];
        let arc = ArcLength::of(&pts);
        let (p, tan) = arc.point_at(&pts, 0.25);
        assert_eq!(p, Point::new(5.0, 0.0));
        assert_eq!(tan, Vec2::RIGHT);
        let (p2, tan2) = arc.point_at(&pts, 0.75);
        assert_eq!(p2, Point::new(10.0, 5.0));
        assert!(
            (tan2.y - 1.0).abs() < 1e-12,
            "the trailing tangent points down in canvas coordinates: {tan2:?}"
        );
    }

    #[test]
    fn flatten_curve_bows_perpendicular_and_zero_is_straight() {
        assert_eq!(
            flatten_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 0.0, 32),
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)]
        );
        let c = flatten_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 0.3, 32);
        assert_eq!(c.len(), 33);
        // Check the curve midpoint's normal displacement.
        let mid = c[16];
        assert!((mid.x - 5.0).abs() < 1e-9);
        assert!((mid.y - 1.5).abs() < 1e-9, "{mid:?}");
        // Preserve exact endpoints.
        assert_eq!(c[0], Point::new(0.0, 0.0));
        assert_eq!(*c.last().unwrap(), Point::new(10.0, 0.0));
    }
}
