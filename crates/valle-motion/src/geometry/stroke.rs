//! Stroke regions are unions, not a pair of independently offset rings.

use super::*;
use geo::Validation;
use i_overlay::mesh::{
    stroke::offset::StrokeOffset,
    style::{LineJoin, StrokeStyle},
};

pub(super) fn expand(path: &PathData, width: f64) -> Result<PathData, GeometryError> {
    if !width.is_finite() || width < 0.0 {
        return Err(GeometryError::InvalidModifier);
    }
    let mut flattened = FlattenedPath::from_path(path)?;
    if width == 0.0 {
        return PathData::new(Vec::new(), Vec::new());
    }
    let mut sections = Vec::<Vec<[f64; 2]>>::new();
    let mut anchors = Vec::new();
    let line_ends = authored_line_ends(path);
    for contour in &mut flattened.contours {
        while contour_ends_at_start(&contour.points) {
            contour.closed = true;
            contour.points.pop();
        }
        contour.points.dedup();
        let n = contour.points.len();
        if n < 2 {
            continue;
        }
        let left = offset_contour(contour, width * 0.5)?;
        let right = offset_contour(contour, -width * 0.5)?;
        for i in 0..n {
            if line_ends.contains(&contour.points[i]) {
                anchors.extend([left[i], right[i]]);
            }
        }
        let at = |i: usize| {
            let p = contour.points[i % n];
            [p.x, p.y]
        };
        if contour.closed {
            for i in 0..n {
                sections.push(vec![at(i + n - 1), at(i), at(i + 1)]);
            }
        } else if n == 2 {
            sections.push(vec![at(0), at(1)]);
        } else {
            for i in 1..n - 1 {
                sections.push(vec![at(i - 1), at(i), at(i + 1)]);
            }
        }
    }
    // Union overlapping two-segment strokes. Independent full inner offsets can
    // invert even in a stroking library; these local sections describe only the
    // actual stroke region, including each join exactly once. Adjacent sections
    // overlap by a whole segment, avoiding cracks from rounded shared boundaries.
    // 2 * asin(1/4) corresponds to the four-radius miter limit.
    let style = StrokeStyle::new(width).line_join(LineJoin::Miter(0.5053605102841573));
    let shapes = sections.stroke(style, false);
    let region = MultiPolygon(
        shapes
            .into_iter()
            .filter_map(|shape| {
                let mut rings = shape.into_iter().map(|ring| {
                    LineString::new(
                        ring.into_iter()
                            .map(|p| Coord { x: p[0], y: p[1] })
                            .collect(),
                    )
                });
                rings
                    .next()
                    .map(|exterior| Polygon::new(exterior, rings.collect()))
            })
            .collect(),
    );
    budgeted_path(region, &anchors)
}

fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

fn authored_line_ends(path: &PathData) -> Vec<Point> {
    let mut result = Vec::new();
    let mut at = 0;
    let mut current = Point::new(0.0, 0.0);
    let mut start = current;
    for verb in &path.verbs {
        match verb {
            PathVerb::Move => {
                current = path.points[at];
                start = current;
                at += 1;
            }
            PathVerb::Line => {
                result.extend([current, path.points[at]]);
                current = path.points[at];
                at += 1;
            }
            PathVerb::Quad => {
                current = path.points[at + 1];
                at += 2;
            }
            PathVerb::Cubic => {
                current = path.points[at + 2];
                at += 3;
            }
            PathVerb::Close => {
                if current != start {
                    result.extend([current, start]);
                }
                current = start;
            }
        }
    }
    result
}

fn budgeted_path(region: MultiPolygon<f64>, anchors: &[Point]) -> Result<PathData, GeometryError> {
    // i_overlay snaps intersections onto a deterministic integer grid. Restore exact
    // authored straight-edge corners when they survive the union.
    let scale = region
        .0
        .iter()
        .flat_map(|p| p.exterior().0.iter())
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0_f64, f64::max);
    let snap = scale * 1e-8;
    let rings: Vec<_> = region
        .0
        .iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
        .map(|ring| {
            ring.0
                .iter()
                .take(ring.0.len().saturating_sub(1))
                .map(|p| {
                    let point = Point::new(p.x, p.y);
                    anchors
                        .iter()
                        .copied()
                        .find(|a| distance(*a, point) <= snap)
                        .unwrap_or(point)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let count: usize = rings.iter().map(Vec::len).sum();
    if count <= MAX_FRAME_GEOMETRY_POINTS {
        return path_from_sampled_contours(rings.into_iter().map(|ring| (ring, true)).collect());
    }
    // Keep corners and straight-edge endpoints, simplifying only the intervening
    // curved runs. The global tolerance gives every contour the same error bound.
    let fixed: Vec<Vec<usize>> = rings
        .iter()
        .map(|ring| {
            let mut indices = vec![0, ring.len() / 2];
            for i in 0..ring.len() {
                let a = sub(ring[i], ring[(i + ring.len() - 1) % ring.len()]);
                let b = sub(ring[(i + 1) % ring.len()], ring[i]);
                let dot = a.x * b.x + a.y * b.y;
                let cross = a.x * b.y - a.y * b.x;
                if dot <= 0.0 || cross.abs() > dot * 0.15 || anchors.contains(&ring[i]) {
                    indices.push(i);
                }
            }
            indices.sort_unstable();
            indices.dedup();
            indices
        })
        .collect();
    let minimum: usize = fixed.iter().map(|v| v.len().max(3)).sum();
    if minimum > MAX_FRAME_GEOMETRY_POINTS {
        return Err(GeometryError::PointBudget {
            limit: MAX_FRAME_GEOMETRY_POINTS,
            actual: minimum,
        });
    }
    let simplify = |tolerance| -> Vec<Vec<Point>> {
        rings
            .iter()
            .zip(&fixed)
            .map(|(ring, fixed)| {
                let mut output = Vec::new();
                for (at, &start) in fixed.iter().enumerate() {
                    let end = fixed.get(at + 1).copied().unwrap_or(ring.len());
                    let run: Vec<_> = (start..=end).map(|i| ring[i % ring.len()]).collect();
                    let mut simplified = simplify_open_points(&run, tolerance);
                    simplified.pop();
                    output.extend(simplified);
                }
                // A closed contour must not collapse to its two partition anchors.
                if output.len() < 3 {
                    output = vec![ring[0], ring[ring.len() / 3], ring[ring.len() * 2 / 3]];
                }
                output
            })
            .collect()
    };
    let mut low = 0.0;
    let mut high = scale * 2.0;
    for _ in 0..40 {
        let mid = (low + high) * 0.5;
        if simplify(mid).iter().map(Vec::len).sum::<usize>() > MAX_FRAME_GEOMETRY_POINTS {
            low = mid;
        } else {
            high = mid;
        }
    }
    let simplified = simplify(high);
    // Simplification may not cross another contour or erase a surviving hole.
    let mut cursor = 0;
    let polygon = |points: &[Point]| {
        LineString::new(
            points
                .iter()
                .map(|p| Coord { x: p.x, y: p.y })
                .chain(points.first().map(|p| Coord { x: p.x, y: p.y }))
                .collect(),
        )
    };
    let checked = MultiPolygon(
        region
            .0
            .iter()
            .map(|original| {
                let exterior = polygon(&simplified[cursor]);
                cursor += 1;
                let interiors = original
                    .interiors()
                    .iter()
                    .map(|_| {
                        let ring = polygon(&simplified[cursor]);
                        cursor += 1;
                        ring
                    })
                    .collect();
                Polygon::new(exterior, interiors)
            })
            .collect(),
    );
    if !checked.is_valid() {
        return Err(GeometryError::PointBudget {
            limit: MAX_FRAME_GEOMETRY_POINTS,
            actual: count,
        });
    }
    path_from_sampled_contours(simplified.into_iter().map(|ring| (ring, true)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Contains;

    fn contains(path: &PathData, point: Point) -> bool {
        let flattened = FlattenedPath::from_path(path).unwrap();
        let mut winding = 0;
        for contour in flattened.contours {
            let polygon = Polygon::new(
                LineString::new(
                    contour
                        .points
                        .iter()
                        .map(|p| Coord { x: p.x, y: p.y })
                        .collect(),
                ),
                Vec::new(),
            );
            if polygon.contains(&geo::Point::new(point.x, point.y)) {
                winding += if signed_double_area(&contour.points) > 0.0 {
                    1
                } else {
                    -1
                };
            }
        }
        winding != 0
    }

    #[test]
    fn thick_circle_has_no_inverted_inner_hole() {
        let circle =
            PathData::arc(Point::new(0.0, 0.0), 10.0, 0.0, core::f64::consts::TAU).unwrap();
        let stroke = circle.stroke_to_path(40.0).unwrap();
        assert!(contains(&stroke, Point::new(0.0, 0.0)));
        assert!(contains(&stroke, Point::new(29.0, 0.0)));
        assert!(!contains(&stroke, Point::new(31.0, 0.0)));
        assert_eq!(
            stroke
                .verbs
                .iter()
                .filter(|v| **v == PathVerb::Move)
                .count(),
            1
        );
        assert!(circle.stroke_to_path(0.0).unwrap().points.is_empty());
    }

    #[test]
    fn collapsed_concave_neck_preserves_both_surviving_holes() {
        let points = [
            (0., 0.),
            (100., 0.),
            (100., 40.),
            (160., 40.),
            (160., 0.),
            (260., 0.),
            (260., 100.),
            (160., 100.),
            (160., 60.),
            (100., 60.),
            (100., 100.),
            (0., 100.),
        ];
        let mut verbs = vec![PathVerb::Move];
        verbs.extend(std::iter::repeat_n(PathVerb::Line, points.len() - 1));
        verbs.push(PathVerb::Close);
        let path = PathData::new(
            verbs,
            points.into_iter().map(|(x, y)| Point::new(x, y)).collect(),
        )
        .unwrap();
        let stroke = path.stroke_to_path(30.0).unwrap();
        assert!(!contains(&stroke, Point::new(50., 50.)));
        assert!(!contains(&stroke, Point::new(210., 50.)));
        assert!(contains(&stroke, Point::new(130., 50.)));
        assert!(contains(&stroke, Point::new(5., 50.)));
    }

    #[test]
    fn complex_curves_fit_the_frame_budget_without_losing_their_shape() {
        for count in [40, 100, 150] {
            let radius = 500.0;
            let step = core::f64::consts::TAU / count as f64;
            let handle = 4.0 / 3.0 * valle_draw::math::tan(step / 4.0);
            let mut points = vec![Point::new(radius, 0.0)];
            let mut verbs = vec![PathVerb::Move];
            for i in 0..count {
                let a = i as f64 * step;
                let b = (i + 1) as f64 * step;
                let (sa, ca) = (valle_draw::math::sin(a), valle_draw::math::cos(a));
                let (sb, cb) = (valle_draw::math::sin(b), valle_draw::math::cos(b));
                points.extend([
                    Point::new(radius * (ca - handle * sa), radius * (sa + handle * ca)),
                    Point::new(radius * (cb + handle * sb), radius * (sb - handle * cb)),
                    Point::new(radius * cb, radius * sb),
                ]);
                verbs.push(PathVerb::Cubic);
            }
            verbs.push(PathVerb::Close);
            let path = PathData::new(verbs, points).unwrap();
            let stroke = path.stroke_to_path(6.0).unwrap();
            assert!(
                stroke.points.len() <= MAX_FRAME_GEOMETRY_POINTS,
                "{count}: {}",
                stroke.points.len()
            );
            assert!(!contains(&stroke, Point::new(0., 0.)));
            for p in &stroke.points {
                let r = distance(*p, Point::new(0., 0.));
                assert!((r - 497.).abs().min((r - 503.).abs()) < 0.1, "{count}: {r}");
            }
        }
    }
}
