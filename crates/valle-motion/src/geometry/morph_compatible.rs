//! Compatible triangulations for pairs of simple polygons.
//!
//! A shared set of visible diagonals avoids extra vertices when possible.
//! Otherwise, each polygon is mapped piecewise linearly to the same convex
//! reference polygon and the two triangulations are overlaid there. The exact
//! rational overlay gives both polygons the same connectivity and Steiner
//! vertices. Morph interpolation is a separate step.

use super::{
    CompatibleMorphMesh, EnclosedCompatibleMorphMesh, FlattenedContour, GeometryError, PathData,
    Point, adaptive_flatten, morph_check, robust_turn,
};
use core::cmp::Ordering;
use geo::TriangulateEarcut;
use geo::algorithm::kernels::Orientation;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MAX_DIRECT_VERTICES: usize = 256;
const MAX_REFINED_VERTICES: usize = 64;
const MAX_MESH_TRIANGLES: usize = 20_000;
const MAX_BARYCENTRIC_INTERIOR_VERTICES: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BarycentricNeighbor {
    vertex: usize,
    from_weight: f64,
    to_weight: f64,
}

/// A prepared Floater–Gotsman convex-combination morph. All non-outer vertices
/// are represented by strictly positive weights on their mesh neighbors; the
/// three vertices of the common convex outer triangle stay fixed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompatibleBarycentricMorph {
    mesh: EnclosedCompatibleMorphMesh,
    rows: Vec<Vec<BarycentricNeighbor>>,
}

impl CompatibleBarycentricMorph {
    pub(super) fn from_mesh(mesh: EnclosedCompatibleMorphMesh) -> Option<Self> {
        let count = mesh.from_vertices.len();
        if count != mesh.to_vertices.len()
            || count < 6
            || count - 3 > MAX_BARYCENTRIC_INTERIOR_VERTICES
            || mesh.outer_vertices != [0, 1, 2]
            || mesh.from_vertices[..3] != mesh.to_vertices[..3]
            || mesh.outline_vertices.len() < 3
            || mesh.outline_vertices.len() + 5 > MAX_REFINED_VERTICES
            || mesh.triangles.len() != count * 2 - 5
            || mesh
                .outline_vertices
                .iter()
                .any(|&index| index < 3 || index >= count)
            || mesh
                .outline_vertices
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != mesh.outline_vertices.len()
            || mesh
                .triangles
                .iter()
                .any(|triangle| triangle.iter().any(|&index| index >= count))
            || [&mesh.from_vertices, &mesh.to_vertices]
                .iter()
                .any(|points| {
                    points
                        .iter()
                        .any(|point| !point.x.is_finite() || !point.y.is_finite())
                        || mesh.triangles.iter().any(|[a, b, c]| {
                            robust_turn(points[*a], points[*b], points[*c])
                                != Orientation::CounterClockwise
                        })
                        || morph_check::closed_contour_winding(
                            &mesh
                                .outline_vertices
                                .iter()
                                .map(|&index| points[index])
                                .collect::<Vec<_>>(),
                        ) != Some(Ordering::Greater)
                })
        {
            return None;
        }
        let mut edge_counts = HashMap::<(usize, usize), usize>::new();
        for [a, b, c] in &mesh.triangles {
            for (left, right) in [(*a, *b), (*b, *c), (*c, *a)] {
                *edge_counts
                    .entry((left.min(right), left.max(right)))
                    .or_default() += 1;
            }
        }
        if [(0, 1), (1, 2), (0, 2)]
            .iter()
            .any(|edge| edge_counts.get(edge) != Some(&1))
            || edge_counts.iter().any(|(edge, count)| {
                *count
                    != if matches!(*edge, (0, 1) | (1, 2) | (0, 2)) {
                        1
                    } else {
                        2
                    }
            })
        {
            return None;
        }
        let mut successors = vec![HashMap::<usize, usize>::new(); count];
        for [a, b, c] in &mesh.triangles {
            for (center, first, second) in [(*a, *b, *c), (*b, *c, *a), (*c, *a, *b)] {
                if center >= 3 && successors[center].insert(first, second).is_some() {
                    return None;
                }
            }
        }
        let mut rows = Vec::with_capacity(count - 3);
        for (center, successors) in successors.iter().enumerate().skip(3) {
            if successors.len() < 3 {
                return None;
            }
            let start = *successors.keys().min().unwrap();
            let mut neighbor = start;
            let mut order = Vec::with_capacity(successors.len());
            for _ in 0..successors.len() {
                order.push(neighbor);
                neighbor = *successors.get(&neighbor)?;
            }
            if neighbor != start
                || order
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != order.len()
            {
                return None;
            }
            let from_weights = mean_value_weights(center, &order, &mesh.from_vertices)?;
            let to_weights = mean_value_weights(center, &order, &mesh.to_vertices)?;
            rows.push(
                order
                    .into_iter()
                    .zip(from_weights.into_iter().zip(to_weights))
                    .map(|(vertex, (from_weight, to_weight))| BarycentricNeighbor {
                        vertex,
                        from_weight,
                        to_weight,
                    })
                    .collect(),
            );
        }
        Some(Self { mesh, rows })
    }

    pub fn mesh(&self) -> &EnclosedCompatibleMorphMesh {
        &self.mesh
    }

    pub(crate) fn is_valid(&self) -> bool {
        Self::from_mesh(self.mesh.clone()).is_some_and(|expected| expected.rows == self.rows)
    }

    /// Evaluate the compatible mesh at a normalized time in `[0, 1]`.
    /// Numerical failure is reported as `None`, rather than emitting a folded
    /// intermediate mesh.
    pub fn positions(&self, t: f64) -> Option<Vec<Point>> {
        if !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return None;
        }
        if t == 0.0 {
            return Some(self.mesh.from_vertices.clone());
        }
        if t == 1.0 {
            return Some(self.mesh.to_vertices.clone());
        }
        let count = self.rows.len();
        let mut matrix = vec![vec![0.0; count]; count];
        let mut rhs_x = vec![0.0; count];
        let mut rhs_y = vec![0.0; count];
        for (row, neighbors) in self.rows.iter().enumerate() {
            matrix[row][row] = 1.0;
            for neighbor in neighbors {
                let weight = (1.0 - t) * neighbor.from_weight + t * neighbor.to_weight;
                if !weight.is_finite() || weight <= 0.0 {
                    return None;
                }
                if neighbor.vertex < 3 {
                    let fixed = self.mesh.from_vertices[neighbor.vertex];
                    rhs_x[row] += weight * fixed.x;
                    rhs_y[row] += weight * fixed.y;
                } else {
                    matrix[row][neighbor.vertex - 3] -= weight;
                }
            }
        }
        let (solved_x, solved_y) = solve_two_rhs(matrix, rhs_x, rhs_y)?;
        let mut positions = self.mesh.from_vertices[..3].to_vec();
        positions.extend(
            solved_x
                .into_iter()
                .zip(solved_y)
                .map(|(x, y)| Point::new(x, y)),
        );
        if positions
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
            || self.mesh.triangles.iter().any(|[a, b, c]| {
                robust_turn(positions[*a], positions[*b], positions[*c])
                    != Orientation::CounterClockwise
            })
        {
            return None;
        }
        let outline = self
            .mesh
            .outline_vertices
            .iter()
            .map(|&index| positions[index])
            .collect::<Vec<_>>();
        (morph_check::closed_contour_winding(&outline) == Some(Ordering::Greater))
            .then_some(positions)
    }

    pub fn outline_path(&self, t: f64) -> Option<PathData> {
        let positions = self.positions(t)?;
        let mut outline = self
            .mesh
            .outline_vertices
            .iter()
            .map(|&index| positions[index])
            .collect::<Vec<_>>();
        if t > 0.0 && t < 1.0 {
            // The fixed outer triangle guarantees a simple outline, but its placement
            // otherwise pulls the intermediate shape toward unrelated mesh vertices.
            // An orientation-preserving similarity removes that global drift without
            // changing the solved shape's topology or its exact endpoints.
            self.align_outline(&mut outline, t)?;
        }
        super::path_from_sampled_contours(vec![(outline, true)]).ok()
    }

    fn align_outline(&self, outline: &mut [Point], t: f64) -> Option<()> {
        let from: Vec<_> = self
            .mesh
            .outline_vertices
            .iter()
            .map(|&i| self.mesh.from_vertices[i])
            .collect();
        let to: Vec<_> = self
            .mesh
            .outline_vertices
            .iter()
            .map(|&i| self.mesh.to_vertices[i])
            .collect();
        let from_center = super::contour_centroid(&from, true);
        let to_center = super::contour_centroid(&to, true);
        let center = super::contour_centroid(outline, true);
        let target = Point::new(
            (1.0 - t) * from_center.x + t * to_center.x,
            (1.0 - t) * from_center.y + t * to_center.y,
        );
        let rms = |points: &[Point], c: Point| {
            valle_draw::math::sqrt(
                points
                    .iter()
                    .map(|p| {
                        let dx = p.x - c.x;
                        let dy = p.y - c.y;
                        dx * dx + dy * dy
                    })
                    .sum::<f64>()
                    / points.len() as f64,
            )
        };
        let radius = rms(outline, center);
        let target_radius = (1.0 - t) * rms(&from, from_center) + t * rms(&to, to_center);
        if radius <= 0.0 || !radius.is_finite() || !target_radius.is_finite() {
            return None;
        }
        // Least-squares rotation toward the authored correspondence. Only use a
        // proper rotation; reflection would invalidate the no-fold guarantee.
        let mut dot = 0.0;
        let mut cross = 0.0;
        for ((p, a), b) in outline.iter().zip(&from).zip(&to) {
            let x = p.x - center.x;
            let y = p.y - center.y;
            let tx = (1.0 - t) * (a.x - from_center.x) + t * (b.x - to_center.x);
            let ty = (1.0 - t) * (a.y - from_center.y) + t * (b.y - to_center.y);
            dot += x * tx + y * ty;
            cross += x * ty - y * tx;
        }
        let norm = libm::hypot(dot, cross);
        let (cos, sin) = if norm > 0.0 {
            (dot / norm, cross / norm)
        } else {
            (1.0, 0.0)
        };
        let bounds = |points: &[Point]| {
            points.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |b, p| [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)],
            )
        };
        let a = bounds(&from);
        let b = bounds(&to);
        let envelope: [f64; 4] = std::array::from_fn(|i| (1.0 - t) * a[i] + t * b[i]);
        let mut scale = target_radius / radius;
        // A barycentric solve can also elongate a local feature. Keep the corrected
        // outline in the interpolated authored extent, using one positive scale so
        // no clipping or per-vertex distortion can invalidate its topology.
        for p in outline.iter() {
            let x = p.x - center.x;
            let y = p.y - center.y;
            let rotated = [cos * x - sin * y, sin * x + cos * y];
            for axis in 0..2 {
                let origin = [target.x, target.y][axis];
                if rotated[axis] > 0.0 {
                    scale = scale.min((envelope[axis + 2] - origin) / rotated[axis]);
                } else if rotated[axis] < 0.0 {
                    scale = scale.min((envelope[axis] - origin) / rotated[axis]);
                }
            }
        }
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        for p in outline.iter_mut() {
            let x = p.x - center.x;
            let y = p.y - center.y;
            *p = Point::new(
                target.x + scale * (cos * x - sin * y),
                target.y + scale * (sin * x + cos * y),
            );
        }
        (outline.iter().all(|p| p.x.is_finite() && p.y.is_finite())
            && morph_check::closed_contour_winding(outline) == Some(Ordering::Greater))
        .then_some(())
    }
}

fn mean_value_weights(center: usize, neighbors: &[usize], points: &[Point]) -> Option<Vec<f64>> {
    let origin = points[center];
    let vectors = neighbors
        .iter()
        .map(|&index| {
            let point = points[index];
            (point.x - origin.x, point.y - origin.y)
        })
        .collect::<Vec<_>>();
    let lengths = vectors
        .iter()
        .map(|&(x, y)| libm::hypot(x, y))
        .collect::<Vec<_>>();
    if lengths
        .iter()
        .any(|length| !length.is_finite() || *length <= 0.0)
    {
        return None;
    }
    let mut half_tangents = Vec::with_capacity(neighbors.len());
    for at in 0..neighbors.len() {
        let next = (at + 1) % neighbors.len();
        let (ax, ay) = vectors[at];
        let (bx, by) = vectors[next];
        let cross = ax * by - ay * bx;
        let dot = ax * bx + ay * by;
        if !cross.is_finite() || cross <= 0.0 || !dot.is_finite() {
            return None;
        }
        let tangent = libm::tan(libm::atan2(cross, dot) * 0.5);
        if !tangent.is_finite() || tangent <= 0.0 {
            return None;
        }
        half_tangents.push(tangent);
    }
    let mut weights = (0..neighbors.len())
        .map(|at| {
            (half_tangents[(at + neighbors.len() - 1) % neighbors.len()] + half_tangents[at])
                / lengths[at]
        })
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    for weight in &mut weights {
        *weight /= total;
    }
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        return None;
    }
    let residual_x = weights
        .iter()
        .zip(&vectors)
        .map(|(weight, (x, _))| weight * x)
        .sum::<f64>();
    let residual_y = weights
        .iter()
        .zip(&vectors)
        .map(|(weight, (_, y))| weight * y)
        .sum::<f64>();
    let scale = lengths.iter().copied().fold(0.0, f64::max);
    (libm::hypot(residual_x, residual_y) <= scale * 1e-9).then_some(weights)
}

fn solve_two_rhs(
    mut matrix: Vec<Vec<f64>>,
    mut rhs_x: Vec<f64>,
    mut rhs_y: Vec<f64>,
) -> Option<(Vec<f64>, Vec<f64>)> {
    let count = matrix.len();
    for column in 0..count {
        let pivot = (column..count)
            .max_by(|&a, &b| matrix[a][column].abs().total_cmp(&matrix[b][column].abs()))?;
        if matrix[pivot][column].abs() <= 1e-14 {
            return None;
        }
        matrix.swap(column, pivot);
        rhs_x.swap(column, pivot);
        rhs_y.swap(column, pivot);
        for row in column + 1..count {
            let factor = matrix[row][column] / matrix[column][column];
            matrix[row][column] = 0.0;
            for at in column + 1..count {
                matrix[row][at] -= factor * matrix[column][at];
            }
            rhs_x[row] -= factor * rhs_x[column];
            rhs_y[row] -= factor * rhs_y[column];
        }
    }
    let mut solved_x = vec![0.0; count];
    let mut solved_y = vec![0.0; count];
    for row in (0..count).rev() {
        let mut x = rhs_x[row];
        let mut y = rhs_y[row];
        for at in row + 1..count {
            x -= matrix[row][at] * solved_x[at];
            y -= matrix[row][at] * solved_y[at];
        }
        solved_x[row] = x / matrix[row][row];
        solved_y[row] = y / matrix[row][row];
    }
    Some((solved_x, solved_y))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ReferencePoint {
    x: BigRational,
    y: BigRational,
}

impl ReferencePoint {
    fn boundary(index: usize) -> Self {
        let x = BigInt::from(index);
        Self {
            x: BigRational::from_integer(x.clone()),
            y: BigRational::from_integer(&x * &x),
        }
    }
}

fn reference_turn(a: &ReferencePoint, b: &ReferencePoint, c: &ReferencePoint) -> BigRational {
    (&b.x - &a.x) * (&c.y - &a.y) - (&b.y - &a.y) * (&c.x - &a.x)
}

fn line_intersection(
    start: &ReferencePoint,
    end: &ReferencePoint,
    start_side: &BigRational,
    end_side: &BigRational,
) -> ReferencePoint {
    let denominator = start_side - end_side;
    ReferencePoint {
        x: (&end.x * start_side - &start.x * end_side) / &denominator,
        y: (&end.y * start_side - &start.y * end_side) / denominator,
    }
}

fn clip_to_triangle(
    triangle: &[ReferencePoint; 3],
    clip: &[ReferencePoint; 3],
) -> Vec<ReferencePoint> {
    let mut polygon = triangle.to_vec();
    for edge in 0..3 {
        let a = &clip[edge];
        let b = &clip[(edge + 1) % 3];
        let mut clipped = Vec::with_capacity(polygon.len() + 1);
        let mut start = polygon.last().unwrap();
        let mut start_side = reference_turn(a, b, start);
        for end in &polygon {
            let end_side = reference_turn(a, b, end);
            let start_inside = start_side >= BigRational::zero();
            let end_inside = end_side >= BigRational::zero();
            if start_inside != end_inside {
                clipped.push(line_intersection(start, end, &start_side, &end_side));
            }
            if end_inside {
                clipped.push(end.clone());
            }
            start = end;
            start_side = end_side;
        }
        clipped.dedup();
        if clipped.len() > 1 && clipped.first() == clipped.last() {
            clipped.pop();
        }
        polygon = clipped;
        if polygon.len() < 3 {
            break;
        }
    }
    polygon
}

fn map_reference_point(
    point: &ReferencePoint,
    triangle: [usize; 3],
    reference: &[ReferencePoint],
    polygon: &[Point],
) -> Option<Point> {
    let [a, b, c] = triangle;
    let denominator = reference_turn(&reference[a], &reference[b], &reference[c]);
    let weight_b = reference_turn(&reference[a], point, &reference[c]) / &denominator;
    let weight_c = reference_turn(&reference[a], &reference[b], point) / &denominator;
    let weight_a = BigRational::from_integer(BigInt::from(1)) - &weight_b - &weight_c;
    let (wa, wb, wc) = (weight_a.to_f64()?, weight_b.to_f64()?, weight_c.to_f64()?);
    let mapped = Point::new(
        polygon[a].x * wa + polygon[b].x * wb + polygon[c].x * wc,
        polygon[a].y * wa + polygon[b].y * wb + polygon[c].y * wc,
    );
    (mapped.x.is_finite() && mapped.y.is_finite()).then_some(mapped)
}

fn on_segment(a: Point, b: Point, point: Point) -> bool {
    robust_turn(a, b, point) == Orientation::Collinear
        && point.x >= a.x.min(b.x)
        && point.x <= a.x.max(b.x)
        && point.y >= a.y.min(b.y)
        && point.y <= a.y.max(b.y)
}

fn opposite(a: Orientation, b: Orientation) -> bool {
    matches!(
        (a, b),
        (Orientation::Clockwise, Orientation::CounterClockwise)
            | (Orientation::CounterClockwise, Orientation::Clockwise)
    )
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_c = robust_turn(a, b, c);
    let ab_d = robust_turn(a, b, d);
    let cd_a = robust_turn(c, d, a);
    let cd_b = robust_turn(c, d, b);
    (opposite(ab_c, ab_d) && opposite(cd_a, cd_b))
        || on_segment(a, b, c)
        || on_segment(a, b, d)
        || on_segment(c, d, a)
        || on_segment(c, d, b)
}

fn in_cone(points: &[Point], at: usize, target: usize) -> bool {
    let len = points.len();
    let origin = points[at];
    let next = points[(at + 1) % len];
    let prev = points[(at + len - 1) % len];
    let forward = robust_turn(origin, next, points[target]) == Orientation::CounterClockwise;
    let backward = robust_turn(origin, points[target], prev) == Orientation::CounterClockwise;
    if robust_turn(origin, next, prev) == Orientation::CounterClockwise {
        forward && backward
    } else {
        forward || backward
    }
}

fn diagonal_inside(points: &[Point], left: usize, right: usize) -> bool {
    if !in_cone(points, left, right) || !in_cone(points, right, left) {
        return false;
    }
    for edge in 0..points.len() {
        let next = (edge + 1) % points.len();
        if [edge, next].contains(&left) || [edge, next].contains(&right) {
            continue;
        }
        if segments_intersect(points[left], points[right], points[edge], points[next]) {
            return false;
        }
    }
    true
}

fn common_triangulation(from: &[Point], to: &[Point]) -> Option<Vec<[usize; 3]>> {
    let count = from.len();
    if count < 3 || count != to.len() || count > MAX_DIRECT_VERTICES {
        return None;
    }
    for points in [from, to] {
        if morph_check::closed_contour_winding(points) != Some(Ordering::Greater) {
            return None;
        }
    }
    let index = |left: usize, right: usize| left * count + right;
    let mut diagonal = vec![false; count * count];
    for left in 0..count {
        for right in left + 1..count {
            diagonal[index(left, right)] = right == left + 1
                || (left == 0 && right == count - 1)
                || (diagonal_inside(from, left, right) && diagonal_inside(to, left, right));
        }
    }
    let mut reachable = vec![false; count * count];
    let mut split = vec![None; count * count];
    for left in 0..count - 1 {
        reachable[index(left, left + 1)] = true;
    }
    for span in 2..count {
        for left in 0..count - span {
            let right = left + span;
            if !diagonal[index(left, right)] {
                continue;
            }
            for middle in left + 1..right {
                if reachable[index(left, middle)] && reachable[index(middle, right)] {
                    reachable[index(left, right)] = true;
                    split[index(left, right)] = Some(middle);
                    break;
                }
            }
        }
    }
    if !reachable[index(0, count - 1)] {
        return None;
    }
    fn collect(
        left: usize,
        right: usize,
        count: usize,
        split: &[Option<usize>],
        triangles: &mut Vec<[usize; 3]>,
    ) {
        if right == left + 1 {
            return;
        }
        let middle = split[left * count + right].unwrap();
        triangles.push([left, middle, right]);
        collect(left, middle, count, split, triangles);
        collect(middle, right, count, split, triangles);
    }
    let mut triangles = Vec::with_capacity(count - 2);
    collect(0, count - 1, count, &split, &mut triangles);
    if triangles.iter().any(|triangle| {
        [from, to].iter().any(|points| {
            robust_turn(
                points[triangle[0]],
                points[triangle[1]],
                points[triangle[2]],
            ) != Orientation::CounterClockwise
        })
    }) {
        return None;
    }
    Some(triangles)
}

fn single_closed(path: &PathData) -> Result<Option<FlattenedContour>, GeometryError> {
    let mut contours = adaptive_flatten(path)?;
    if contours.len() != 1 || !contours[0].closed {
        return Ok(None);
    }
    Ok(contours.pop())
}

fn normalized_pair(
    from: &PathData,
    to: &PathData,
) -> Result<Option<(Vec<Point>, Vec<Point>)>, GeometryError> {
    let (Some(mut from), Some(mut to)) = (single_closed(from)?, single_closed(to)?) else {
        return Ok(None);
    };
    let a_winding = morph_check::closed_contour_winding(&from.points);
    let b_winding = morph_check::closed_contour_winding(&to.points);
    if a_winding.is_none() || b_winding.is_none() {
        return Ok(None);
    }
    if a_winding == Some(Ordering::Less) {
        from.points[1..].reverse();
    }
    if b_winding == Some(Ordering::Less) {
        to.points[1..].reverse();
    }
    Ok(Some((from.points, to.points)))
}

pub(super) fn prepare_direct(
    from: &PathData,
    to: &PathData,
) -> Result<Option<CompatibleMorphMesh>, GeometryError> {
    let Some((from, to)) = normalized_pair(from, to)? else {
        return Ok(None);
    };
    let Some(triangles) = common_triangulation(&from, &to) else {
        return Ok(None);
    };
    Ok(Some(CompatibleMorphMesh {
        boundary_vertices: from.len(),
        from_vertices: from,
        to_vertices: to,
        triangles,
    }))
}

pub(super) fn prepare_refined(
    from: &PathData,
    to: &PathData,
) -> Result<Option<CompatibleMorphMesh>, GeometryError> {
    let Some((from, to)) = normalized_pair(from, to)? else {
        return Ok(None);
    };
    Ok(mesh_from_normalized(from, to))
}

fn mesh_from_normalized(from: Vec<Point>, to: Vec<Point>) -> Option<CompatibleMorphMesh> {
    let count = from.len();
    if count != to.len() {
        return None;
    }
    if let Some(triangles) = common_triangulation(&from, &to) {
        return Some(CompatibleMorphMesh {
            boundary_vertices: count,
            from_vertices: from,
            to_vertices: to,
            triangles,
        });
    }
    if count > MAX_REFINED_VERTICES {
        return None;
    }
    let (Some(from_triangles), Some(to_triangles)) = (
        common_triangulation(&from, &from),
        common_triangulation(&to, &to),
    ) else {
        return None;
    };
    overlay_triangulations(from, to, &from_triangles, &to_triangles)
}

fn overlay_triangulations(
    from: Vec<Point>,
    to: Vec<Point>,
    from_triangles: &[[usize; 3]],
    to_triangles: &[[usize; 3]],
) -> Option<CompatibleMorphMesh> {
    let count = from.len();
    if count != to.len()
        || count > MAX_REFINED_VERTICES
        || [(from_triangles, &from), (to_triangles, &to)]
            .iter()
            .any(|(triangles, points)| {
                triangles.iter().any(|triangle| {
                    triangle.iter().any(|&index| index >= count)
                        || robust_turn(
                            points[triangle[0]],
                            points[triangle[1]],
                            points[triangle[2]],
                        ) != Orientation::CounterClockwise
                })
            })
    {
        return None;
    }
    let reference = (0..count).map(ReferencePoint::boundary).collect::<Vec<_>>();
    if from_triangles.iter().chain(to_triangles).any(|triangle| {
        reference_turn(
            &reference[triangle[0]],
            &reference[triangle[1]],
            &reference[triangle[2]],
        ) <= BigRational::zero()
    }) {
        return None;
    }
    let mut vertex_index = reference
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, point)| (point, index))
        .collect::<HashMap<_, _>>();
    let mut mesh = CompatibleMorphMesh {
        boundary_vertices: count,
        from_vertices: from,
        to_vertices: to,
        triangles: Vec::new(),
    };
    for &from_triangle in from_triangles {
        let first = from_triangle.map(|index| reference[index].clone());
        for &to_triangle in to_triangles {
            // A positive-area intersection needs an overlap in x on the
            // parabola used for the convex reference boundary.
            let from_min = *from_triangle.iter().min().unwrap();
            let from_max = *from_triangle.iter().max().unwrap();
            let to_min = *to_triangle.iter().min().unwrap();
            let to_max = *to_triangle.iter().max().unwrap();
            if from_max <= to_min || to_max <= from_min {
                continue;
            }
            let second = to_triangle.map(|index| reference[index].clone());
            let clipped = clip_to_triangle(&first, &second);
            if clipped.len() < 3 {
                continue;
            }
            let mut indices = Vec::with_capacity(clipped.len());
            for point in &clipped {
                let index = if let Some(&index) = vertex_index.get(point) {
                    index
                } else {
                    if mesh.from_vertices.len() == super::MAX_PATH_POINTS {
                        return None;
                    }
                    let (Some(from_point), Some(to_point)) = (
                        map_reference_point(point, from_triangle, &reference, &mesh.from_vertices),
                        map_reference_point(point, to_triangle, &reference, &mesh.to_vertices),
                    ) else {
                        return None;
                    };
                    let index = mesh.from_vertices.len();
                    mesh.from_vertices.push(from_point);
                    mesh.to_vertices.push(to_point);
                    vertex_index.insert(point.clone(), index);
                    index
                };
                indices.push(index);
            }
            for at in 1..indices.len() - 1 {
                if reference_turn(&clipped[0], &clipped[at], &clipped[at + 1])
                    <= BigRational::zero()
                {
                    continue;
                }
                let triangle = [indices[0], indices[at], indices[at + 1]];
                if [&mesh.from_vertices, &mesh.to_vertices]
                    .iter()
                    .any(|points| {
                        robust_turn(
                            points[triangle[0]],
                            points[triangle[1]],
                            points[triangle[2]],
                        ) != Orientation::CounterClockwise
                    })
                {
                    return None;
                }
                mesh.triangles.push(triangle);
                if mesh.triangles.len() > MAX_MESH_TRIANGLES {
                    return None;
                }
            }
        }
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

/// A bridge from the leftmost hole vertex to the left outer vertex turns an
/// annulus into a disk. Both bridge endpoints occur twice on the cut boundary;
/// their copies are kept separate until after compatible refinement.
fn triangulate_cut_annulus(
    outer: [Point; 3],
    hole: &[Point],
) -> Option<(Vec<Point>, Vec<[usize; 3]>)> {
    if hole.len() < 3
        || morph_check::closed_contour_winding(hole) != Some(Ordering::Greater)
        || hole.iter().any(|point| {
            point.x < hole[0].x
                || (0..3).any(|edge| {
                    robust_turn(outer[edge], outer[(edge + 1) % 3], *point)
                        != Orientation::CounterClockwise
                })
        })
        || outer[0].x >= hole[0].x
    {
        return None;
    }
    let mut cut = Vec::with_capacity(hole.len() + 5);
    cut.extend([outer[0], hole[0]]);
    cut.extend(hole[1..].iter().rev().copied());
    cut.extend([hole[0], outer[0], outer[1], outer[2]]);
    let ring = geo::LineString::from(
        cut.iter()
            .chain(std::iter::once(&outer[0]))
            .map(|point| (point.x, point.y))
            .collect::<Vec<_>>(),
    );
    let raw = geo::Polygon::new(ring, vec![]).earcut_triangles_raw();
    if raw.vertices.len() != cut.len() || raw.triangle_indices.len() != (cut.len() - 2) * 3 {
        return None;
    }
    let mut triangles = Vec::with_capacity(cut.len() - 2);
    for indices in raw.triangle_indices.chunks_exact(3) {
        let [a, mut b, mut c] = [indices[0], indices[1], indices[2]];
        if [a, b, c].iter().any(|&index| index >= cut.len()) {
            return None;
        }
        match robust_turn(cut[a], cut[b], cut[c]) {
            Orientation::CounterClockwise => {}
            Orientation::Clockwise => std::mem::swap(&mut b, &mut c),
            Orientation::Collinear => return None,
        }
        triangles.push([a, b, c]);
    }
    Some((cut, triangles))
}

fn rotate_to_leftmost(points: &mut [Point]) {
    let leftmost = points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
        .unwrap()
        .0;
    points.rotate_left(leftmost);
}

fn subdivide_boundary_to(points: &mut Vec<Point>, target: usize) -> Option<()> {
    while points.len() < target {
        let mut longest = None;
        for at in 0..points.len() {
            let next = (at + 1) % points.len();
            let a = points[at];
            let b = points[next];
            let midpoint = Point::new((a.x * 0.5) + (b.x * 0.5), (a.y * 0.5) + (b.y * 0.5));
            let length = libm::hypot(b.x - a.x, b.y - a.y);
            if !length.is_finite()
                || !midpoint.x.is_finite()
                || !midpoint.y.is_finite()
                || midpoint == a
                || midpoint == b
            {
                continue;
            }
            if longest.is_none_or(|(_, best_length)| length > best_length) {
                longest = Some((at, length));
            }
        }
        let (at, _) = longest?;
        let next = (at + 1) % points.len();
        let a = points[at];
        let b = points[next];
        points.insert(
            at + 1,
            Point::new((a.x * 0.5) + (b.x * 0.5), (a.y * 0.5) + (b.y * 0.5)),
        );
    }
    Some(())
}

fn enclosing_triangle(from: &[Point], to: &[Point]) -> Option<[Point; 3]> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in from.iter().chain(to) {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    let span = (max_x - min_x).max(max_y - min_y).max(1.0);
    let margin = span * 4.0;
    let outer = [
        Point::new(min_x - margin, min_y + (max_y - min_y) * 0.5),
        Point::new(max_x + margin, min_y - margin),
        Point::new(max_x + margin, max_y + margin),
    ];
    outer
        .iter()
        .all(|point| point.x.is_finite() && point.y.is_finite())
        .then_some(outer)
}

pub(super) fn prepare_enclosed(
    from: &PathData,
    to: &PathData,
) -> Result<Option<EnclosedCompatibleMorphMesh>, GeometryError> {
    let Some((mut from, mut to)) = normalized_pair(from, to)? else {
        return Ok(None);
    };
    let target = from.len().max(to.len());
    if target + 5 > MAX_REFINED_VERTICES
        || subdivide_boundary_to(&mut from, target).is_none()
        || subdivide_boundary_to(&mut to, target).is_none()
    {
        return Ok(None);
    }
    rotate_to_leftmost(&mut from);
    rotate_to_leftmost(&mut to);
    let Some(outer) = enclosing_triangle(&from, &to) else {
        return Ok(None);
    };
    let (Some(interior), Some((cut_from, from_triangles)), Some((cut_to, to_triangles))) = (
        mesh_from_normalized(from.clone(), to.clone()),
        triangulate_cut_annulus(outer, &from),
        triangulate_cut_annulus(outer, &to),
    ) else {
        return Ok(None);
    };
    let Some(annulus) = overlay_triangulations(cut_from, cut_to, &from_triangles, &to_triangles)
    else {
        return Ok(None);
    };
    let outline_count = from.len();
    let cut_count = outline_count + 5;
    let mut from_vertices = Vec::with_capacity(
        3 + interior.from_vertices.len() + annulus.from_vertices.len() - cut_count,
    );
    let mut to_vertices = Vec::with_capacity(from_vertices.capacity());
    from_vertices.extend(outer);
    to_vertices.extend(outer);
    from_vertices.extend(&interior.from_vertices);
    to_vertices.extend(&interior.to_vertices);
    let mut cut_to_combined = Vec::with_capacity(annulus.from_vertices.len());
    for at in 0..annulus.from_vertices.len() {
        let combined = match at {
            0 => 0,
            1 => 3,
            index if index <= outline_count => 3 + outline_count + 1 - index,
            index if index == outline_count + 1 => 3,
            index if index == outline_count + 2 => 0,
            index if index == outline_count + 3 => 1,
            index if index == outline_count + 4 => 2,
            _ => {
                let index = from_vertices.len();
                from_vertices.push(annulus.from_vertices[at]);
                to_vertices.push(annulus.to_vertices[at]);
                index
            }
        };
        cut_to_combined.push(combined);
    }
    let mut triangles = interior
        .triangles
        .iter()
        .map(|triangle| triangle.map(|index| index + 3))
        .collect::<Vec<_>>();
    triangles.extend(
        annulus
            .triangles
            .iter()
            .map(|triangle| triangle.map(|index| cut_to_combined[index])),
    );
    if from_vertices.len() > super::MAX_PATH_POINTS || triangles.len() > MAX_MESH_TRIANGLES {
        return Ok(None);
    }
    let mut edges = HashMap::<(usize, usize), usize>::new();
    for triangle in &triangles {
        if [&from_vertices, &to_vertices].iter().any(|points| {
            robust_turn(
                points[triangle[0]],
                points[triangle[1]],
                points[triangle[2]],
            ) != Orientation::CounterClockwise
        }) {
            return Ok(None);
        }
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    for outer_edge in [(0, 1), (1, 2), (0, 2)] {
        if edges.get(&outer_edge) != Some(&1) {
            return Ok(None);
        }
    }
    if edges.iter().any(|(edge, uses)| {
        let outer_edge = matches!(*edge, (0, 1) | (1, 2) | (0, 2));
        *uses != if outer_edge { 1 } else { 2 }
    }) {
        return Ok(None);
    }
    Ok(Some(EnclosedCompatibleMorphMesh {
        from_vertices,
        to_vertices,
        outer_vertices: [0, 1, 2],
        outline_vertices: (3..outline_count + 3).collect(),
        triangles,
    }))
}

pub(super) fn prepare_sequence(
    paths: &[PathData],
) -> Result<Option<Vec<CompatibleBarycentricMorph>>, GeometryError> {
    if paths.len() < 2 {
        return Ok(None);
    }
    let mut contours = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(mut contour) = single_closed(path)? else {
            return Ok(None);
        };
        match morph_check::closed_contour_winding(&contour.points) {
            Some(Ordering::Greater) => {}
            Some(Ordering::Less) => contour.points[1..].reverse(),
            _ => return Ok(None),
        }
        contours.push(contour.points);
    }
    let count = contours.iter().map(Vec::len).max().unwrap();
    if count + 5 > MAX_REFINED_VERTICES {
        return Ok(None);
    }
    let mut normalized = Vec::with_capacity(paths.len());
    for mut contour in contours {
        if subdivide_boundary_to(&mut contour, count).is_none() {
            return Ok(None);
        }
        normalized.push(super::path_from_sampled_contours(vec![(contour, true)])?);
    }
    let mut segments = Vec::with_capacity(paths.len() - 1);
    for pair in normalized.windows(2) {
        let Some(mesh) = prepare_enclosed(&pair[0], &pair[1])? else {
            return Ok(None);
        };
        let Some(morph) = CompatibleBarycentricMorph::from_mesh(mesh) else {
            return Ok(None);
        };
        segments.push(morph);
    }
    Ok(Some(segments))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[(f64, f64)]) -> PathData {
        super::super::path_from_sampled_contours(vec![(
            points.iter().map(|(x, y)| Point::new(*x, *y)).collect(),
            true,
        )])
        .unwrap()
    }

    fn assert_covers_both_polygons(mesh: &CompatibleMorphMesh) {
        let mut edge_counts = HashMap::<(usize, usize), usize>::new();
        for [a, b, c] in &mesh.triangles {
            for (left, right) in [(*a, *b), (*b, *c), (*c, *a)] {
                let edge = (left.min(right), left.max(right));
                *edge_counts.entry(edge).or_default() += 1;
            }
        }
        for at in 0..mesh.boundary_vertices {
            let next = (at + 1) % mesh.boundary_vertices;
            assert_eq!(edge_counts.get(&(at.min(next), at.max(next))), Some(&1));
        }
        for ((left, right), uses) in edge_counts {
            let boundary = left < mesh.boundary_vertices
                && right < mesh.boundary_vertices
                && (left + 1 == right || (left == 0 && right + 1 == mesh.boundary_vertices));
            assert_eq!(uses, if boundary { 1 } else { 2 }, "edge {left} {right}");
        }
        assert_eq!(mesh.from_vertices.len(), mesh.to_vertices.len());
        for points in [&mesh.from_vertices, &mesh.to_vertices] {
            let boundary = &points[..mesh.boundary_vertices];
            let polygon_area = boundary
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let next = boundary[(index + 1) % boundary.len()];
                    point.x * next.y - point.y * next.x
                })
                .sum::<f64>()
                * 0.5;
            let mesh_area = mesh
                .triangles
                .iter()
                .map(|[a, b, c]| {
                    let (a, b, c) = (points[*a], points[*b], points[*c]);
                    assert_eq!(robust_turn(a, b, c), Orientation::CounterClockwise);
                    ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5
                })
                .sum::<f64>();
            assert!(
                (mesh_area - polygon_area).abs() < 1e-9,
                "{mesh_area} {polygon_area}"
            );
        }
    }

    #[test]
    fn direct_mesh_triangulates_corresponding_concave_polygons() {
        let a = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 2.0), (0.0, 4.0)]);
        let b = polygon(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (2.0, 1.0), (0.0, 5.0)]);
        let mesh = a.direct_compatible_morph_mesh(&b).unwrap().unwrap();
        assert_eq!(mesh.triangles.len(), 3);
        assert_eq!(mesh.from_vertices.len(), mesh.to_vertices.len());
        assert_eq!(mesh.boundary_vertices, 5);
        for triangle in &mesh.triangles {
            for points in [&mesh.from_vertices, &mesh.to_vertices] {
                assert_eq!(
                    robust_turn(
                        points[triangle[0]],
                        points[triangle[1]],
                        points[triangle[2]],
                    ),
                    Orientation::CounterClockwise
                );
            }
        }
        let reversed = a
            .reverse()
            .unwrap()
            .direct_compatible_morph_mesh(&b.reverse().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(reversed.triangles.len(), 3);
        assert_eq!(
            morph_check::closed_contour_winding(&reversed.from_vertices),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn direct_mesh_rejects_bad_input_without_pretending_it_added_steiner_vertices() {
        let square = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let triangle = polygon(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]);
        assert!(
            square
                .direct_compatible_morph_mesh(&triangle)
                .unwrap()
                .is_none()
        );
        let bow_tie = polygon(&[(0.0, 0.0), (4.0, 4.0), (0.0, 4.0), (4.0, 0.0)]);
        assert!(
            square
                .direct_compatible_morph_mesh(&bow_tie)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn compatible_mesh_normalizes_opposite_winding_and_collinear_boundary_points() {
        let a = polygon(&[(0.0, 0.0), (2.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let b = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 5.0), (0.0, 4.0)]);
        let mesh = a
            .compatible_morph_mesh(&b.reverse().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(mesh.boundary_vertices, 5);
        assert_covers_both_polygons(&mesh);
    }

    #[test]
    fn an_exterior_bridge_opens_a_polygonal_annulus_into_a_triangulable_disk() {
        let outer = [
            Point::new(-10.0, 0.0),
            Point::new(10.0, -10.0),
            Point::new(10.0, 10.0),
        ];
        let hole = [
            Point::new(0.0, 0.0),
            Point::new(3.0, 0.0),
            Point::new(3.0, 3.0),
            Point::new(1.5, 1.5),
            Point::new(0.0, 3.0),
        ];
        let (cut, triangles) = triangulate_cut_annulus(outer, &hole).unwrap();
        assert_eq!(triangles.len(), cut.len() - 2);
        let area = triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = [cut[triangle[0]], cut[triangle[1]], cut[triangle[2]]];
                assert_eq!(robust_turn(a, b, c), Orientation::CounterClockwise);
                ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5
            })
            .sum::<f64>();
        let hole_area = hole
            .iter()
            .enumerate()
            .map(|(at, point)| {
                let next = hole[(at + 1) % hole.len()];
                point.x * next.y - point.y * next.x
            })
            .sum::<f64>()
            * 0.5;
        assert!((area - (200.0 - hole_area)).abs() < 1e-9);
    }

    #[test]
    fn cut_annulus_triangulations_share_a_refined_mesh() {
        let outer = [
            Point::new(-10.0, 0.0),
            Point::new(10.0, -10.0),
            Point::new(10.0, 10.0),
        ];
        let a = [
            Point::new(0.0, 0.0),
            Point::new(3.0, 0.0),
            Point::new(3.0, 3.0),
            Point::new(1.5, 1.5),
            Point::new(0.0, 3.0),
        ];
        let b = [
            Point::new(0.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(3.0, 2.0),
            Point::new(4.0, 4.0),
            Point::new(0.0, 4.0),
        ];
        let (cut_a, triangles_a) = triangulate_cut_annulus(outer, &a).unwrap();
        let (cut_b, triangles_b) = triangulate_cut_annulus(outer, &b).unwrap();
        let mesh = overlay_triangulations(cut_a, cut_b, &triangles_a, &triangles_b).unwrap();
        assert_eq!(mesh.boundary_vertices, a.len() + 5);
        assert_eq!(mesh.from_vertices.len(), mesh.to_vertices.len());
        for points in [&mesh.from_vertices, &mesh.to_vertices] {
            assert_eq!(points[0], points[a.len() + 2]);
            assert_eq!(points[1], points[a.len() + 1]);
            assert!(mesh.triangles.iter().all(|[a, b, c]| {
                robust_turn(points[*a], points[*b], points[*c]) == Orientation::CounterClockwise
            }));
        }
    }

    #[test]
    fn enclosed_mesh_glues_the_cut_annulus_to_the_shape_interior() {
        let a = polygon(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (1.5, 1.5), (0.0, 3.0)]);
        let b = polygon(&[(0.0, 0.0), (4.0, 0.0), (3.0, 2.0), (4.0, 4.0), (0.0, 4.0)]);
        let mesh = a.enclosed_compatible_morph_mesh(&b).unwrap().unwrap();
        assert_eq!(mesh.outer_vertices, [0, 1, 2]);
        assert_eq!(mesh.outline_vertices, vec![3, 4, 5, 6, 7]);
        assert_eq!(&mesh.from_vertices[..3], &mesh.to_vertices[..3]);
        for points in [&mesh.from_vertices, &mesh.to_vertices] {
            let [left, bottom, top] = mesh.outer_vertices.map(|index| points[index]);
            let outer_area = ((bottom.x - left.x) * (top.y - left.y)
                - (bottom.y - left.y) * (top.x - left.x))
                * 0.5;
            let triangle_area = mesh
                .triangles
                .iter()
                .map(|[a, b, c]| {
                    let [a, b, c] = [points[*a], points[*b], points[*c]];
                    assert_eq!(robust_turn(a, b, c), Orientation::CounterClockwise);
                    ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5
                })
                .sum::<f64>();
            assert!((outer_area - triangle_area).abs() < 1e-9);
        }
        let morph = a.prepare_compatible_barycentric_morph(&b).unwrap().unwrap();
        assert_eq!(morph.positions(0.0).unwrap(), morph.mesh().from_vertices);
        assert_eq!(morph.positions(1.0).unwrap(), morph.mesh().to_vertices);
        for step in 1..20 {
            let t = step as f64 / 20.0;
            let positions = morph
                .positions(t)
                .expect("barycentric morph must stay valid");
            assert_eq!(&positions[..3], &mesh.from_vertices[..3]);
            assert!(morph.outline_path(t).is_some());
        }
    }

    #[test]
    fn enclosed_mesh_accepts_an_inserted_collinear_boundary_vertex() {
        let square = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let triangle = polygon(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]);
        let morph = square
            .prepare_compatible_barycentric_morph(&triangle)
            .unwrap()
            .expect("adding a flat vertex preserves the triangle's authored boundary");
        assert_eq!(morph.mesh().outline_vertices.len(), 4);
        assert!(morph.outline_path(0.5).is_some());
    }

    #[test]
    fn compatible_star_to_concave_shape_preserves_authored_position_and_scale() {
        let star: Vec<_> = (0..10)
            .map(|i| {
                let angle = i as f64 * core::f64::consts::PI / 5.0 - core::f64::consts::FRAC_PI_2;
                let radius = if i % 2 == 0 { 75.0 } else { 30.0 };
                (
                    100.0 + radius * libm::cos(angle),
                    100.0 + radius * libm::sin(angle),
                )
            })
            .collect();
        let a = polygon(&star);
        let b = polygon(&[
            (30., 25.),
            (160., 25.),
            (160., 55.),
            (65., 55.),
            (65., 125.),
            (160., 125.),
            (160., 155.),
            (30., 155.),
        ]);
        let morph = a.prepare_compatible_barycentric_morph(&b).unwrap().unwrap();
        let start = morph.outline_path(0.0).unwrap();
        let end = morph.outline_path(1.0).unwrap();
        let ca = super::super::contour_centroid(&start.points, true);
        let cb = super::super::contour_centroid(&end.points, true);
        for frame in 1..30 {
            let t = frame as f64 / 30.0;
            let path = morph.outline_path(t).unwrap();
            let center = super::super::contour_centroid(&path.points, true);
            assert!((center.x - ((1.0 - t) * ca.x + t * cb.x)).abs() < 1e-7);
            assert!((center.y - ((1.0 - t) * ca.y + t * cb.y)).abs() < 1e-7);
            assert!(
                path.points
                    .iter()
                    .all(|p| p.x > 0.0 && p.x < 200.0 && p.y > 0.0 && p.y < 200.0),
                "frame {frame}: {:?}",
                path.points
            );
            assert_eq!(
                morph_check::closed_contour_winding(&path.points),
                Some(Ordering::Greater)
            );
        }
        // The endpoint mesh remains unchanged; the appearance correction acts only
        // on intermediate outlines and cannot weaken its no-fold validation.
        assert_eq!(morph.positions(0.0).unwrap(), morph.mesh().from_vertices);
        assert_eq!(morph.positions(1.0).unwrap(), morph.mesh().to_vertices);
    }

    #[test]
    fn direct_mesh_covers_random_simple_polygons_without_overlap() {
        let mut seed = 0x3ca6_94f2_712e_d083_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed as u32 as f64) / (u32::MAX as f64)
        };
        let mut accepted = 0;
        let mut needs_steiner = 0;
        for _ in 0..64 {
            let mut shapes = Vec::new();
            for _ in 0..2 {
                let points = (0..10)
                    .map(|index| {
                        let angle = std::f64::consts::TAU * index as f64 / 10.0;
                        let radius = 0.4 + 1.6 * next();
                        Point::new(radius * angle.cos(), radius * angle.sin())
                    })
                    .collect::<Vec<_>>();
                shapes
                    .push(super::super::path_from_sampled_contours(vec![(points, true)]).unwrap());
            }
            let enclosed = shapes[0]
                .enclosed_compatible_morph_mesh(&shapes[1])
                .unwrap()
                .expect("simple polygons must embed in a fixed outer triangle");
            assert_eq!(&enclosed.from_vertices[..3], &enclosed.to_vertices[..3]);
            for points in [&enclosed.from_vertices, &enclosed.to_vertices] {
                let [a, b, c] = enclosed.outer_vertices.map(|index| points[index]);
                let outer_area = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5;
                let mesh_area = enclosed
                    .triangles
                    .iter()
                    .map(|[a, b, c]| {
                        let [a, b, c] = [points[*a], points[*b], points[*c]];
                        ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5
                    })
                    .sum::<f64>();
                assert!((outer_area - mesh_area).abs() < 1e-9);
            }
            let barycentric = CompatibleBarycentricMorph::from_mesh(enclosed)
                .expect("positive neighbor weights must exist in the enclosing mesh");
            for t in [0.2, 0.5, 0.8] {
                assert!(
                    barycentric.positions(t).is_some(),
                    "barycentric solve failed at t={t}"
                );
            }
            let Some(mesh) = shapes[0].direct_compatible_morph_mesh(&shapes[1]).unwrap() else {
                needs_steiner += 1;
                let refined = shapes[0]
                    .compatible_morph_mesh(&shapes[1])
                    .unwrap()
                    .expect("simple polygons must have a refined compatible mesh");
                assert!(refined.from_vertices.len() > 10);
                assert_covers_both_polygons(&refined);
                if needs_steiner == 1 {
                    assert_eq!(
                        refined,
                        shapes[0]
                            .compatible_morph_mesh(&shapes[1])
                            .unwrap()
                            .unwrap()
                    );
                }
                continue;
            };
            accepted += 1;
            assert_covers_both_polygons(&mesh);
        }
        assert!(accepted > 0);
        assert!(needs_steiner > 0);
    }
}
