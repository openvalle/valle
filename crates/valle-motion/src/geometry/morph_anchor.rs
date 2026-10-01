//! Perimeter correspondence constrained by authored boundary points.

use super::{FlattenedContour, GeometryError, POLAR_MORPH_POINTS, Point, point_lerp};

struct ContourMeasure<'a> {
    points: &'a [Point],
    prefix: Vec<f64>,
    total: f64,
    tolerance: f64,
}

#[derive(Clone, Copy)]
struct AnchorLocation {
    distance: f64,
    point: Point,
}

impl<'a> ContourMeasure<'a> {
    fn new(contour: &'a FlattenedContour) -> Result<Self, GeometryError> {
        let points = contour.points.as_slice();
        let mut prefix = Vec::with_capacity(points.len() + 1);
        prefix.push(0.0);
        for edge in 0..points.len() {
            let a = points[edge];
            let b = points[(edge + 1) % points.len()];
            prefix.push(prefix[edge] + valle_draw::math::hypot(b.x - a.x, b.y - a.y));
        }
        let total = prefix[points.len()];
        if !total.is_finite() || total <= 0.0 {
            return Err(GeometryError::DegenerateContour);
        }
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for point in points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        Ok(Self {
            points,
            prefix,
            total,
            // Curve flattening can put a true-curve anchor slightly off the polyline.
            tolerance: ((max_x - min_x).max(max_y - min_y) * 0.001).max(1e-12),
        })
    }

    fn locate(&self, anchor: Point) -> Result<AnchorLocation, GeometryError> {
        if !anchor.x.is_finite() || !anchor.y.is_finite() {
            return Err(GeometryError::InvalidMorphAnchors);
        }
        let mut best = (f64::INFINITY, 0usize, 0.0, Point::new(0.0, 0.0));
        for edge in 0..self.points.len() {
            let a = self.points[edge];
            let b = self.points[(edge + 1) % self.points.len()];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let squared = dx * dx + dy * dy;
            let along = if squared > 0.0 {
                ((anchor.x - a.x) * dx + (anchor.y - a.y) * dy) / squared
            } else {
                0.0
            }
            .clamp(0.0, 1.0);
            let projected = point_lerp(a, b, along);
            let error = valle_draw::math::hypot(anchor.x - projected.x, anchor.y - projected.y);
            if error < best.0 {
                best = (error, edge, along, projected);
            }
        }
        if best.0 > self.tolerance {
            return Err(GeometryError::InvalidMorphAnchors);
        }
        let mut distance =
            self.prefix[best.1] + (self.prefix[best.1 + 1] - self.prefix[best.1]) * best.2;
        if distance >= self.total {
            distance = 0.0;
        }
        Ok(AnchorLocation {
            distance,
            point: best.3,
        })
    }

    fn sample(&self, distance: f64) -> Point {
        let target = distance.rem_euclid(self.total);
        let end = self
            .prefix
            .partition_point(|value| *value < target)
            .clamp(1, self.points.len());
        let edge = end - 1;
        let span = self.prefix[end] - self.prefix[edge];
        let along = if span > 0.0 {
            (target - self.prefix[edge]) / span
        } else {
            0.0
        };
        point_lerp(
            self.points[edge],
            self.points[(edge + 1) % self.points.len()],
            along,
        )
    }
}

pub(super) fn anchor_matches_contour(contour: &FlattenedContour, anchor: Point) -> bool {
    ContourMeasure::new(contour)
        .and_then(|measure| measure.locate(anchor))
        .is_ok()
}

fn same_cyclic_order(expected: &[usize], actual: &[usize]) -> bool {
    actual
        .iter()
        .position(|item| *item == expected[0])
        .is_some_and(|start| {
            expected
                .iter()
                .enumerate()
                .all(|(at, item)| actual[(start + at) % actual.len()] == *item)
        })
}

fn segment_samples(spans: &[Vec<f64>]) -> Vec<usize> {
    let segments = spans[0].len();
    let minimum = if segments == 1 { 1 } else { 4 };
    let remaining = POLAR_MORPH_POINTS - segments * minimum;
    let weights = (0..segments)
        .map(|segment| spans.iter().map(|path| path[segment]).fold(0.0, f64::max))
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f64>();
    let quotas = weights
        .iter()
        .map(|weight| remaining as f64 * weight / total)
        .collect::<Vec<_>>();
    let mut counts = quotas
        .iter()
        .map(|quota| minimum + quota.floor() as usize)
        .collect::<Vec<_>>();
    let leftover = POLAR_MORPH_POINTS - counts.iter().sum::<usize>();
    let mut remainder = (0..segments).collect::<Vec<_>>();
    remainder.sort_by(|a, b| {
        (quotas[*b] - quotas[*b].floor())
            .total_cmp(&(quotas[*a] - quotas[*a].floor()))
            .then_with(|| a.cmp(b))
    });
    for segment in remainder.into_iter().take(leftover) {
        counts[segment] += 1;
    }
    counts
}

pub(super) fn sample_anchored_contours(
    contours: &[FlattenedContour],
    anchors: &[Vec<Point>],
) -> Result<Vec<Vec<Point>>, GeometryError> {
    if anchors.is_empty()
        || anchors.len() > 32
        || anchors.len() * 4 > POLAR_MORPH_POINTS
        || anchors.iter().any(|row| row.len() != contours.len())
    {
        return Err(GeometryError::InvalidMorphAnchors);
    }
    let measures = contours
        .iter()
        .map(ContourMeasure::new)
        .collect::<Result<Vec<_>, _>>()?;
    let located = measures
        .iter()
        .enumerate()
        .map(|(path, measure)| {
            anchors
                .iter()
                .map(|row| measure.locate(row[path]))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut order = (0..anchors.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| located[0][*a].distance.total_cmp(&located[0][*b].distance));
    for (path, positions) in located.iter().enumerate() {
        let mut sorted = (0..anchors.len()).collect::<Vec<_>>();
        sorted.sort_by(|a, b| positions[*a].distance.total_cmp(&positions[*b].distance));
        if !same_cyclic_order(&order, &sorted) {
            return Err(GeometryError::InvalidMorphAnchors);
        }
        let gap = measures[path].total * 1e-9;
        for adjacent in sorted.windows(2) {
            if positions[adjacent[1]].distance - positions[adjacent[0]].distance <= gap {
                return Err(GeometryError::InvalidMorphAnchors);
            }
        }
        if anchors.len() > 1
            && positions[sorted[0]].distance + measures[path].total
                - positions[*sorted.last().unwrap()].distance
                <= gap
        {
            return Err(GeometryError::InvalidMorphAnchors);
        }
    }
    let ordered = located
        .iter()
        .map(|path| order.iter().map(|index| path[*index]).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let spans = ordered
        .iter()
        .zip(&measures)
        .map(|(path, measure)| {
            (0..anchors.len())
                .map(|segment| {
                    let start = path[segment].distance;
                    let end = path[(segment + 1) % path.len()].distance;
                    if path.len() == 1 {
                        measure.total
                    } else if end > start {
                        end - start
                    } else {
                        end + measure.total - start
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let counts = segment_samples(&spans);
    let mut output = Vec::with_capacity(contours.len());
    for (path_index, (path, measure)) in ordered.iter().zip(&measures).enumerate() {
        let mut points = Vec::with_capacity(POLAR_MORPH_POINTS);
        for (segment, count) in counts.iter().enumerate() {
            points.push(path[segment].point);
            for sample in 1..*count {
                let distance = path[segment].distance
                    + spans[path_index][segment] * sample as f64 / *count as f64;
                points.push(measure.sample(distance));
            }
        }
        output.push(points);
    }
    Ok(output)
}
