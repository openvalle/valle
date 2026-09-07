//! Prepared polygon projection and SVG path construction.
//!
//! Input/output stay generic: polygons are rings of number pairs; results are path strings,
//! centroids and bounds. There is no Region, Feature, Map node, paint or renderer dependency.

use serde::Serialize;

use super::geo::{Fit, ProjectionKind, Projector};

pub type Point = (f64, f64);
pub type Ring = Vec<Point>;
pub type Polygon = Vec<Ring>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedPolygon {
    pub d: String,
    pub centroid: [f64; 2],
    /// `[left, top, right, bottom]`.
    pub bounds: [f64; 4],
    pub point_count: usize,
}

pub fn project_paths(
    polygons: &[Polygon],
    kind: ProjectionKind,
    width: f64,
    height: f64,
    padding: f64,
    clip: Option<ClipRect>,
) -> Result<Vec<ProjectedPolygon>, String> {
    if polygons.is_empty() {
        return Err("geoPath needs at least one polygon".into());
    }
    let unwrapped = polygons
        .iter()
        .map(|polygon| unwrap_polygon(polygon))
        .collect::<Result<Vec<_>, _>>()?;
    let all = unwrapped
        .iter()
        .flat_map(|polygon| polygon.iter())
        .flat_map(|ring| ring.iter().copied())
        .collect::<Vec<_>>();
    let lon = extent(all.iter().map(|point| point.0));
    let lat = extent(all.iter().map(|point| point.1));
    let projector = Projector::fit_kind(kind, lon, lat);
    let plane = unwrapped
        .iter()
        .map(|polygon| {
            polygon
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|&(x, y)| projector.project(x, y))
                        .collect::<Ring>()
                })
                .collect::<Polygon>()
        })
        .collect::<Vec<_>>();
    let plane_all = plane
        .iter()
        .flat_map(|polygon| polygon.iter())
        .flat_map(|ring| ring.iter().copied())
        .collect::<Vec<_>>();
    let x = extent(plane_all.iter().map(|point| point.0));
    let y = extent(plane_all.iter().map(|point| point.1));
    let fit = Fit::contain((x.0, y.0), (x.1, y.1), width, height, padding);
    let clip = clip.unwrap_or(ClipRect {
        left: 0.0,
        top: 0.0,
        right: width,
        bottom: height,
    });

    plane
        .into_iter()
        .map(|polygon| {
            let mut rings = polygon
                .into_iter()
                .map(|ring| {
                    ring.into_iter()
                        .map(|point| fit.apply(point))
                        .collect::<Ring>()
                })
                .map(|ring| clip_ring(&ring, clip))
                .filter(|ring| ring.len() >= 3)
                .collect::<Vec<_>>();
            if rings.is_empty() {
                return Err("geoPath clipping removed an entire polygon".into());
            }
            // Non-zero fill needs holes to wind opposite the outer ring. Do not trust external
            // data winding; normalize it here after y-flip and clipping.
            let outer_sign = signed_area(&rings[0]).signum();
            for ring in rings.iter_mut().skip(1) {
                if signed_area(ring).signum() == outer_sign {
                    ring.reverse();
                }
            }
            let points = rings.iter().flatten().copied().collect::<Vec<_>>();
            let x = extent(points.iter().map(|point| point.0));
            let y = extent(points.iter().map(|point| point.1));
            let centroid =
                polygon_centroid(&rings[0]).unwrap_or(((x.0 + x.1) / 2.0, (y.0 + y.1) / 2.0));
            Ok(ProjectedPolygon {
                d: path_string(&rings),
                centroid: [centroid.0, centroid.1],
                bounds: [x.0, y.0, x.1, y.1],
                point_count: points.len(),
            })
        })
        .collect()
}

fn unwrap_polygon(polygon: &Polygon) -> Result<Polygon, String> {
    if polygon.is_empty() {
        return Err("every geoPath polygon needs an outer ring".into());
    }
    let mut output = Vec::with_capacity(polygon.len());
    let mut outer_mean: Option<f64> = None;
    for source in polygon {
        let mut ring = normalized_ring(source)?;
        for index in 1..ring.len() {
            while ring[index].0 - ring[index - 1].0 > 180.0 {
                ring[index].0 -= 360.0;
            }
            while ring[index].0 - ring[index - 1].0 < -180.0 {
                ring[index].0 += 360.0;
            }
        }
        let mean = ring.iter().map(|point| point.0).sum::<f64>() / ring.len() as f64;
        if let Some(reference) = outer_mean {
            let shift = ((reference - mean) / 360.0).round() * 360.0;
            for point in &mut ring {
                point.0 += shift;
            }
        } else {
            outer_mean = Some(mean);
        }
        output.push(ring);
    }
    Ok(output)
}

fn normalized_ring(source: &Ring) -> Result<Ring, String> {
    if source.len() < 3 {
        return Err("every geoPath ring needs at least three points".into());
    }
    let mut ring = source.clone();
    if ring.len() > 3 && ring.first() == ring.last() {
        ring.pop();
    }
    if ring.len() < 3 {
        return Err("every geoPath ring needs three distinct positions".into());
    }
    Ok(ring)
}

fn extent(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
        (min.min(value), max.max(value))
    })
}

fn signed_area(ring: &Ring) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        / 2.0
}

fn polygon_centroid(ring: &Ring) -> Option<Point> {
    let area6 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>();
    if area6.abs() < 1e-12 {
        return None;
    }
    let (x, y) = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .fold((0.0, 0.0), |(x, y), (a, b)| {
            let cross = a.0 * b.1 - b.0 * a.1;
            (x + (a.0 + b.0) * cross, y + (a.1 + b.1) * cross)
        });
    Some((x / (3.0 * area6), y / (3.0 * area6)))
}

fn path_string(rings: &[Ring]) -> String {
    let mut output = String::new();
    for ring in rings {
        for (index, point) in ring.iter().enumerate() {
            if index == 0 {
                output.push('M');
            } else {
                output.push('L');
            }
            output.push_str(&point.0.to_string());
            output.push(' ');
            output.push_str(&point.1.to_string());
        }
        output.push('Z');
    }
    output
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

fn clip_ring(input: &Ring, rect: ClipRect) -> Ring {
    [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]
        .into_iter()
        .fold(input.clone(), |points, edge| clip_edge(&points, rect, edge))
}

fn clip_edge(input: &Ring, rect: ClipRect, edge: Edge) -> Ring {
    if input.is_empty() {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut previous = *input.last().unwrap();
    let mut previous_inside = inside(previous, rect, edge);
    for &current in input {
        let current_inside = inside(current, rect, edge);
        if current_inside != previous_inside {
            output.push(intersection(previous, current, rect, edge));
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
    output
}

fn inside(point: Point, rect: ClipRect, edge: Edge) -> bool {
    match edge {
        Edge::Left => point.0 >= rect.left,
        Edge::Right => point.0 <= rect.right,
        Edge::Top => point.1 >= rect.top,
        Edge::Bottom => point.1 <= rect.bottom,
    }
}

fn intersection(a: Point, b: Point, rect: ClipRect, edge: Edge) -> Point {
    match edge {
        Edge::Left | Edge::Right => {
            let x = if matches!(edge, Edge::Left) {
                rect.left
            } else {
                rect.right
            };
            let t = if b.0 == a.0 {
                0.0
            } else {
                (x - a.0) / (b.0 - a.0)
            };
            (x, a.1 + (b.1 - a.1) * t)
        }
        Edge::Top | Edge::Bottom => {
            let y = if matches!(edge, Edge::Top) {
                rect.top
            } else {
                rect.bottom
            };
            let t = if b.1 == a.1 {
                0.0
            } else {
                (y - a.1) / (b.1 - a.1)
            };
            (a.0 + (b.0 - a.0) * t, y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antimeridian_ring_stays_narrow_and_hole_winds_opposite() {
        let polygons = vec![vec![
            vec![
                (170.0, 10.0),
                (-170.0, 10.0),
                (-170.0, -10.0),
                (170.0, -10.0),
            ],
            vec![(175.0, 5.0), (175.0, -5.0), (-175.0, -5.0), (-175.0, 5.0)],
        ]];
        let result = project_paths(
            &polygons,
            ProjectionKind::Mercator,
            1000.0,
            500.0,
            0.05,
            None,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].d.matches('M').count() == 2);
        assert!(result[0].bounds[2] - result[0].bounds[0] < 900.1);
    }

    #[test]
    fn explicit_clip_keeps_every_output_point_inside() {
        let polygons = vec![vec![vec![
            (-20.0, -20.0),
            (20.0, -20.0),
            (20.0, 20.0),
            (-20.0, 20.0),
        ]]];
        let result = project_paths(
            &polygons,
            ProjectionKind::Mercator,
            1000.0,
            500.0,
            0.0,
            Some(ClipRect {
                left: 200.0,
                top: 100.0,
                right: 800.0,
                bottom: 400.0,
            }),
        )
        .unwrap();
        let bounds = result[0].bounds;
        assert!(bounds[0] >= 200.0 && bounds[1] >= 100.0);
        assert!(bounds[2] <= 800.0 && bounds[3] <= 400.0);
        assert_eq!((bounds[1], bounds[3]), (100.0, 400.0));
    }
}
