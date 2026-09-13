//! Prepare-time monotone-X cubic fitting. One cubic per adjacent sample pair.

use valle_draw::PathVerb;
use valle_draw::Point;

use crate::geometry::{GeometryError, PathData};

/// Fit a monotone cubic through strictly increasing-X points. Gaps and non-finite samples fail.
pub fn monotone_x(points: &[Point]) -> Result<PathData, String> {
    if points.len() < 2 {
        return Err("curve needs at least two points".into());
    }
    if points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err("curve points must be finite".into());
    }
    if points.windows(2).any(|pair| pair[1].x <= pair[0].x) {
        return Err("curve X coordinates must be strictly increasing".into());
    }

    let n = points.len();
    let mut slope = vec![0.0; n - 1];
    for index in 0..n - 1 {
        slope[index] =
            (points[index + 1].y - points[index].y) / (points[index + 1].x - points[index].x);
    }
    let mut tangent = vec![0.0; n];
    tangent[0] = slope[0];
    tangent[n - 1] = slope[n - 2];
    for index in 1..n - 1 {
        tangent[index] = if slope[index - 1] * slope[index] <= 0.0 {
            0.0
        } else {
            (slope[index - 1] + slope[index]) / 2.0
        };
    }
    for index in 0..n - 1 {
        let d = slope[index];
        if d == 0.0 {
            tangent[index] = 0.0;
            tangent[index + 1] = 0.0;
            continue;
        }
        let alpha = tangent[index] / d;
        let beta = tangent[index + 1] / d;
        let length = alpha * alpha + beta * beta;
        if length > 9.0 {
            let tau = 3.0 / valle_draw::math::sqrt(length);
            tangent[index] = tau * alpha * d;
            tangent[index + 1] = tau * beta * d;
        }
    }

    let mut verbs = Vec::with_capacity(n);
    let mut out = Vec::with_capacity(1 + (n - 1) * 3);
    verbs.push(PathVerb::Move);
    out.push(points[0]);
    for index in 0..n - 1 {
        let dx = points[index + 1].x - points[index].x;
        verbs.push(PathVerb::Cubic);
        out.push(Point::new(
            points[index].x + dx / 3.0,
            points[index].y + tangent[index] * dx / 3.0,
        ));
        out.push(Point::new(
            points[index + 1].x - dx / 3.0,
            points[index + 1].y - tangent[index + 1] * dx / 3.0,
        ));
        out.push(points[index + 1]);
    }
    PathData::new(verbs, out).map_err(|error| match error {
        GeometryError::NonFinite => "curve points must be finite".into(),
        _ => error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cubic_at(points: &[Point], t: f64) -> Point {
        let s = 1.0 - t;
        let a = points[0];
        let b = points[1];
        let c = points[2];
        let d = points[3];
        Point::new(
            s * s * s * a.x + 3.0 * s * s * t * b.x + 3.0 * s * t * t * c.x + t * t * t * d.x,
            s * s * s * a.y + 3.0 * s * s * t * b.y + 3.0 * s * t * t * c.y + t * t * t * d.y,
        )
    }

    #[test]
    fn monotone_x_passes_through_samples_without_extra_extrema() {
        let samples = [
            Point::new(0.0, 0.0),
            Point::new(1.0, 2.0),
            Point::new(2.0, 2.0),
            Point::new(3.0, 1.0),
        ];
        let path = monotone_x(&samples).unwrap();
        assert_eq!(path.verbs.len(), samples.len());
        assert_eq!(path.verbs[0], PathVerb::Move);
        assert!(path.verbs[1..].iter().all(|verb| *verb == PathVerb::Cubic));
        for (index, sample) in samples.iter().enumerate() {
            let point = if index == 0 {
                path.points[0]
            } else {
                path.points[index * 3]
            };
            assert_eq!(point, *sample);
        }
        for index in 0..samples.len() - 1 {
            let cubic = [
                path.points[index * 3],
                path.points[index * 3 + 1],
                path.points[index * 3 + 2],
                path.points[index * 3 + 3],
            ];
            let lo = samples[index].y.min(samples[index + 1].y);
            let hi = samples[index].y.max(samples[index + 1].y);
            for step in 0..=16 {
                let point = cubic_at(&cubic, step as f64 / 16.0);
                assert!(
                    point.y + 1e-9 >= lo && point.y - 1e-9 <= hi,
                    "extra extremum {point:?} on segment {index}"
                );
                if step > 0 {
                    assert!(point.x + 1e-12 >= cubic_at(&cubic, (step - 1) as f64 / 16.0).x);
                }
            }
        }
        assert!(monotone_x(&[Point::new(0.0, 0.0)]).is_err());
        assert!(monotone_x(&[Point::new(0.0, 0.0), Point::new(0.0, 1.0)]).is_err());
        assert!(monotone_x(&[Point::new(0.0, 0.0), Point::new(1.0, f64::NAN)]).is_err());
    }
}
