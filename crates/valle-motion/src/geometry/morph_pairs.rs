//! Explicit multi-contour correspondence for path morphing.

use super::{
    FlattenedContour, GeometryError, MorphContact, MorphMethod, POLAR_MORPH_POINTS, PathData,
    Point, adaptive_flatten, morph_anchor, morph_check, path_from_sampled_contours,
};
use core::cmp::Ordering;
use std::collections::BinaryHeap;
use valle_draw::PathVerb;

struct PreparedRow {
    paths: Vec<Option<Vec<Point>>>,
    parent: Option<usize>,
    depth: usize,
    needs_check: bool,
}

fn invalid(reason: impl Into<String>) -> GeometryError {
    GeometryError::InvalidMorphPairs(reason.into())
}

fn contour_path(contour: &FlattenedContour) -> Result<PathData, GeometryError> {
    let mut verbs = vec![PathVerb::Move];
    verbs.extend(std::iter::repeat_n(
        PathVerb::Line,
        contour.points.len().saturating_sub(1),
    ));
    verbs.push(PathVerb::Close);
    PathData::new(verbs, contour.points.clone())
}

fn prepared_winding(points: &[Point]) -> Option<Ordering> {
    let mut distinct = Vec::with_capacity(points.len());
    for point in points {
        if distinct.last() != Some(point) {
            distinct.push(*point);
        }
    }
    if distinct.first() == distinct.last() {
        distinct.pop();
    }
    morph_check::closed_contour_winding(&distinct)
}

fn align(
    paths: &[PathData],
    anchors: &[Vec<Point>],
    method: MorphMethod,
) -> Result<(Vec<PathData>, bool), GeometryError> {
    if !anchors.is_empty() {
        if !matches!(method, MorphMethod::Auto | MorphMethod::ArcLength) {
            return Err(GeometryError::InvalidMorphAnchors);
        }
        return Ok((
            PathData::arc_length_morph_sequence_with_options(paths, anchors, true)?,
            true,
        ));
    }
    if paths.len() == 1 {
        let prepared = match method {
            MorphMethod::Convex => paths[0].convex_morph_pair(&paths[0])?.0,
            MorphMethod::Polar => paths[0].polar_morph_pair(&paths[0])?.0,
            MorphMethod::Auto | MorphMethod::ArcLength => paths[0].resample(POLAR_MORPH_POINTS)?,
        };
        return Ok((vec![prepared], false));
    }
    match method {
        MorphMethod::Convex => Ok((PathData::convex_morph_sequence(paths)?, false)),
        MorphMethod::Polar => Ok((PathData::polar_morph_sequence(paths)?, false)),
        MorphMethod::ArcLength => Ok((
            PathData::arc_length_morph_sequence_with_options(paths, &[], true)?,
            true,
        )),
        MorphMethod::Auto => {
            if let Ok(aligned) = PathData::convex_morph_sequence(paths) {
                return Ok((aligned, false));
            }
            if let Ok(aligned) = PathData::polar_morph_sequence(paths) {
                return Ok((aligned, false));
            }
            Ok((
                PathData::arc_length_morph_sequence_with_options(paths, &[], true)?,
                true,
            ))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointPosition {
    Outside,
    Boundary,
    Inside,
}

fn point_position(contour: &[Point], point: Point) -> PointPosition {
    let mut inside = false;
    for edge in 0..contour.len() {
        let a = contour[edge];
        let b = contour[(edge + 1) % contour.len()];
        let turn = super::robust_turn(a, b, point);
        if turn == super::Orientation::Collinear
            && point.x >= a.x.min(b.x)
            && point.x <= a.x.max(b.x)
            && point.y >= a.y.min(b.y)
            && point.y <= a.y.max(b.y)
        {
            return PointPosition::Boundary;
        }
        if (a.y > point.y) != (b.y > point.y)
            && ((b.y > a.y && turn == super::Orientation::CounterClockwise)
                || (b.y < a.y && turn == super::Orientation::Clockwise))
        {
            inside = !inside;
        }
    }
    if inside {
        PointPosition::Inside
    } else {
        PointPosition::Outside
    }
}

fn region_contains(outer: &[Point], holes: &[&[Point]], point: Point) -> bool {
    point_position(outer, point) == PointPosition::Inside
        && holes
            .iter()
            .all(|hole| point_position(hole, point) == PointPosition::Outside)
}

fn clearance(contour: &[Point], point: Point) -> f64 {
    contour
        .iter()
        .enumerate()
        .map(|(edge, a)| {
            let b = contour[(edge + 1) % contour.len()];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length_squared = dx * dx + dy * dy;
            let along = if length_squared > 0.0 {
                ((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared
            } else {
                0.0
            }
            .clamp(0.0, 1.0);
            valle_draw::math::hypot(point.x - a.x - along * dx, point.y - a.y - along * dy)
        })
        .fold(f64::INFINITY, f64::min)
}

fn signed_clearance(outer: &[Point], holes: &[&[Point]], point: Point) -> f64 {
    let distance = holes.iter().fold(clearance(outer, point), |current, hole| {
        current.min(clearance(hole, point))
    });
    if region_contains(outer, holes, point) {
        distance
    } else {
        -distance
    }
}

fn consider_interior(
    point: Point,
    distance: f64,
    preferred: Option<Point>,
    precision: f64,
    best: &mut Option<(Point, f64)>,
) {
    if point.x.is_finite() && point.y.is_finite() {
        let preferred_distance = |candidate: Point| {
            preferred.map_or(0.0, |target| {
                valle_draw::math::hypot(candidate.x - target.x, candidate.y - target.y)
            })
        };
        if distance > 0.0
            && best.is_none_or(|(current, previous)| {
                distance > previous + precision
                    || ((distance - previous).abs() <= precision
                        && preferred.is_some()
                        && preferred_distance(point) < preferred_distance(current))
            })
        {
            *best = Some((point, distance));
        }
    }
}

#[derive(Clone, Copy)]
struct InteriorCell {
    center: Point,
    half: f64,
    distance: f64,
    upper: f64,
}

impl InteriorCell {
    fn new(outer: &[Point], holes: &[&[Point]], center: Point, half: f64) -> Self {
        let distance = signed_clearance(outer, holes, center);
        Self {
            center,
            half,
            distance,
            upper: distance + half * std::f64::consts::SQRT_2,
        }
    }
}

impl PartialEq for InteriorCell {
    fn eq(&self, other: &Self) -> bool {
        self.upper.to_bits() == other.upper.to_bits()
            && self.center.x.to_bits() == other.center.x.to_bits()
            && self.center.y.to_bits() == other.center.y.to_bits()
            && self.half.to_bits() == other.half.to_bits()
    }
}

impl Eq for InteriorCell {}

impl PartialOrd for InteriorCell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InteriorCell {
    fn cmp(&self, other: &Self) -> Ordering {
        self.upper
            .total_cmp(&other.upper)
            .then_with(|| self.center.x.total_cmp(&other.center.x))
            .then_with(|| self.center.y.total_cmp(&other.center.y))
            .then_with(|| self.half.total_cmp(&other.half))
    }
}

fn interior_point(
    contour: &[Point],
    holes: &[&[Point]],
    preferred: Option<Point>,
) -> Result<Point, GeometryError> {
    if contour.iter().all(|point| *point == contour[0]) {
        return Ok(contour[0]);
    }
    let winding = prepared_winding(contour).ok_or(GeometryError::DegenerateContour)?;
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in contour {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    let span = width.max(height);
    if !span.is_finite() || span <= 0.0 {
        return Err(GeometryError::DegenerateContour);
    }
    let precision = (width.min(height) * 1e-5).max(span * f64::EPSILON * 16.0);
    let mut best: Option<(Point, f64)> = None;
    if let Some(preferred) = preferred {
        consider_interior(
            preferred,
            signed_clearance(contour, holes, preferred),
            Some(preferred),
            precision,
            &mut best,
        );
    }
    let midpoint = Point::new(min_x + width * 0.5, min_y + height * 0.5);
    consider_interior(
        midpoint,
        signed_clearance(contour, holes, midpoint),
        preferred,
        precision,
        &mut best,
    );
    for edge in 0..contour.len() {
        let a = contour[edge];
        let b = contour[(edge + 1) % contour.len()];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length = valle_draw::math::hypot(dx, dy);
        if length == 0.0 {
            continue;
        }
        let side = if winding == Ordering::Greater {
            1.0
        } else {
            -1.0
        };
        let midpoint = Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
        for power in 0..48 {
            let distance = span * valle_draw::math::pow(2.0, f64::from(-power - 2));
            let candidate = Point::new(
                midpoint.x - side * dy / length * distance,
                midpoint.y + side * dx / length * distance,
            );
            consider_interior(
                candidate,
                signed_clearance(contour, holes, candidate),
                preferred,
                precision,
                &mut best,
            );
            if best.is_some() {
                break;
            }
        }
        if best.is_some() {
            break;
        }
    }
    // The signed distance is 1-Lipschitz. A cell center plus its half-diagonal
    // bounds the best clearance attainable anywhere in that cell.
    let mut queue = BinaryHeap::from([InteriorCell::new(
        contour,
        holes,
        Point::new(min_x + width * 0.5, min_y + height * 0.5),
        span * 0.5,
    )]);
    for _ in 0..50_000 {
        let Some(cell) = queue.pop() else { break };
        consider_interior(cell.center, cell.distance, preferred, precision, &mut best);
        if cell.upper <= best.map_or(0.0, |(_, distance)| distance) + precision
            || cell.half <= precision * 0.5
        {
            continue;
        }
        let half = cell.half * 0.5;
        for dy in [-half, half] {
            for dx in [-half, half] {
                queue.push(InteriorCell::new(
                    contour,
                    holes,
                    Point::new(cell.center.x + dx, cell.center.y + dy),
                    half,
                ));
            }
        }
    }
    best.map(|(point, _)| point)
        .filter(|point| {
            morph_check::strictly_inside_contour(contour, *point)
                && holes
                    .iter()
                    .all(|hole| point_position(hole, *point) == PointPosition::Outside)
        })
        .ok_or(GeometryError::DegenerateContour)
}

fn contour_center(contour: &[Point]) -> Point {
    let (min_x, max_x, min_y, max_y) = contour.iter().fold(
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ),
        |(min_x, max_x, min_y, max_y), point| {
            (
                min_x.min(point.x),
                max_x.max(point.x),
                min_y.min(point.y),
                max_y.max(point.y),
            )
        },
    );
    Point::new(min_x + (max_x - min_x) * 0.5, min_y + (max_y - min_y) * 0.5)
}

pub(super) fn prepare(
    paths: &[PathData],
    pairs: &[Vec<Option<usize>>],
    anchors: &[Vec<Point>],
    method: MorphMethod,
    allow_self_intersection: bool,
) -> Result<Vec<PathData>, GeometryError> {
    prepare_with_contacts(paths, pairs, anchors, method, allow_self_intersection, true)
        .map(|(paths, _)| paths)
}

pub(super) fn prepare_with_contacts(
    paths: &[PathData],
    pairs: &[Vec<Option<usize>>],
    anchors: &[Vec<Point>],
    method: MorphMethod,
    allow_self_intersection: bool,
    contact_is_error: bool,
) -> Result<(Vec<PathData>, Vec<MorphContact>), GeometryError> {
    if !(2..=16).contains(&paths.len()) || pairs.is_empty() || pairs.len() > 8 {
        return Err(GeometryError::TooComplex);
    }
    if anchors.len() > 32 {
        return Err(GeometryError::InvalidMorphAnchors);
    }
    let contours = paths
        .iter()
        .map(adaptive_flatten)
        .collect::<Result<Vec<_>, _>>()?;
    for path in &contours {
        if path.iter().any(|contour| !contour.closed) {
            return Err(GeometryError::ClosedContourRequired);
        }
    }
    let hierarchy = contours
        .iter()
        .map(|path| {
            if path.is_empty() {
                Some(morph_check::ContourHierarchy {
                    parent: Vec::new(),
                    depth: Vec::new(),
                })
            } else {
                morph_check::contour_hierarchy(
                    &path
                        .iter()
                        .map(|contour| contour.points.clone())
                        .collect::<Vec<_>>(),
                )
            }
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(GeometryError::DegenerateContour)?;
    let mut contour_rows = contours
        .iter()
        .map(|path| vec![None; path.len()])
        .collect::<Vec<_>>();
    for (row, cells) in pairs.iter().enumerate() {
        if cells.len() != paths.len() || cells.iter().all(Option::is_none) {
            return Err(invalid(
                "each row needs one index or null per key shape and at least one index",
            ));
        }
        for (key, cell) in cells.iter().enumerate() {
            if let Some(index) = cell {
                let Some(entry) = contour_rows[key].get_mut(*index) else {
                    return Err(invalid(format!("key shape {key} has no contour {index}")));
                };
                if entry.replace(row).is_some() {
                    return Err(invalid(format!("key shape {key} repeats contour {index}")));
                }
            }
        }
    }
    if contour_rows
        .iter()
        .any(|rows| rows.iter().any(Option::is_none))
    {
        return Err(invalid("every contour must appear exactly once in pairs"));
    }
    let mut anchors_by_row = vec![Vec::new(); pairs.len()];
    for (anchor_index, anchor_row) in anchors.iter().enumerate() {
        if anchor_row.len() != paths.len() {
            return Err(invalid(format!(
                "anchor row {anchor_index} needs one point per key shape"
            )));
        }
        let mut assigned = None;
        for (key, anchor) in anchor_row.iter().enumerate() {
            let matches = contours[key]
                .iter()
                .enumerate()
                .filter(|(_, contour)| morph_anchor::anchor_matches_contour(contour, *anchor))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(invalid(format!(
                    "anchor row {anchor_index} at key shape {key} must belong to exactly one contour"
                )));
            }
            let pair = contour_rows[key][matches[0]].unwrap();
            if assigned.is_some_and(|previous| previous != pair) {
                return Err(invalid(format!(
                    "anchor row {anchor_index} crosses contour pairs"
                )));
            }
            assigned = Some(pair);
        }
        anchors_by_row[assigned.unwrap()].push(anchor_row.clone());
    }
    let mut rows = Vec::with_capacity(pairs.len());
    for (pair_index, cells) in pairs.iter().enumerate() {
        let mut parent = None;
        let mut depth = None;
        let mut existing = Vec::new();
        for (key, cell) in cells.iter().enumerate() {
            if let Some(index) = cell {
                let current_parent = hierarchy[key].parent[*index]
                    .map(|contour| contour_rows[key][contour].unwrap());
                let current_depth = hierarchy[key].depth[*index];
                if let Some(previous) = parent {
                    if previous != current_parent {
                        return Err(invalid(
                            "a contour changes its containing pair between key shapes",
                        ));
                    }
                } else {
                    parent = Some(current_parent);
                }
                if depth.is_some_and(|previous| previous != current_depth) {
                    return Err(invalid(
                        "a pair mixes outer contours, holes, or nesting levels",
                    ));
                }
                depth = Some(current_depth);
                existing.push((key, contour_path(&contours[key][*index])?));
            }
        }
        let source = existing
            .iter()
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        let (aligned, needs_check) = align(&source, &anchors_by_row[pair_index], method)?;
        let mut prepared = vec![None; paths.len()];
        for ((key, _), mut path) in existing.into_iter().zip(aligned) {
            let winding = prepared_winding(&path.points).ok_or(GeometryError::DegenerateContour)?;
            if (winding == Ordering::Greater) != (depth.unwrap() % 2 == 0) {
                path = path.reverse()?;
            }
            prepared[key] = Some(path.points);
        }
        rows.push(PreparedRow {
            paths: prepared,
            parent: parent.unwrap(),
            depth: depth.unwrap(),
            needs_check,
        });
    }
    let mut by_depth = (0..rows.len()).collect::<Vec<_>>();
    by_depth.sort_by_key(|row| rows[*row].depth);
    for row in by_depth {
        for key in 0..paths.len() {
            if rows[row].paths[key].is_some() {
                continue;
            }
            let nearest = (0..paths.len())
                .filter(|key| rows[row].paths[*key].is_some())
                .min_by_key(|other| other.abs_diff(key))
                .unwrap();
            let preferred = contour_center(rows[row].paths[nearest].as_ref().unwrap());
            let center = if let Some(parent) = rows[row].parent {
                let siblings = rows
                    .iter()
                    .enumerate()
                    .filter(|(other, data)| {
                        *other != row && data.parent == Some(parent) && pairs[*other][key].is_some()
                    })
                    .map(|(_, data)| data.paths[key].as_ref().unwrap().as_slice())
                    .collect::<Vec<_>>();
                interior_point(
                    rows[parent].paths[key].as_ref().unwrap(),
                    &siblings,
                    Some(preferred),
                )?
            } else {
                interior_point(
                    rows[row].paths[nearest].as_ref().unwrap(),
                    &[],
                    Some(preferred),
                )?
            };
            let count = rows[row].paths.iter().flatten().next().unwrap().len();
            rows[row].paths[key] = Some(vec![center; count]);
        }
    }
    let mut contacts = Vec::new();
    if !allow_self_intersection {
        for key in 0..paths.len() - 1 {
            for (row, data) in rows.iter().enumerate() {
                if data.needs_check && pairs[row][key].is_some() && pairs[row][key + 1].is_some() {
                    let from =
                        path_from_sampled_contours(vec![(data.paths[key].clone().unwrap(), true)])?;
                    let to = path_from_sampled_contours(vec![(
                        data.paths[key + 1].clone().unwrap(),
                        true,
                    )])?;
                    if let Some(issue) =
                        morph_check::scan_linear_morph(&from, &to, &[], &[], contact_is_error)
                            .map_err(|issue| GeometryError::UnsafeMorph(issue.to_string()))?
                    {
                        contacts.push(MorphContact {
                            segment: key,
                            detail: format!("contour row {row}: {issue}"),
                        });
                    }
                }
            }
            for left in 0..rows.len() {
                for right in left + 1..rows.len() {
                    if let Some(issue) = morph_check::scan_contour_pair_morph(
                        rows[left].paths[key].as_ref().unwrap(),
                        rows[left].paths[key + 1].as_ref().unwrap(),
                        rows[right].paths[key].as_ref().unwrap(),
                        rows[right].paths[key + 1].as_ref().unwrap(),
                        [pairs[left][key].is_none(), pairs[left][key + 1].is_none()],
                        [pairs[right][key].is_none(), pairs[right][key + 1].is_none()],
                        contact_is_error,
                    )
                    .map_err(|issue| GeometryError::UnsafeMorph(issue.to_string()))?
                    {
                        contacts.push(MorphContact {
                            segment: key,
                            detail: format!("contour rows {left} and {right}: {issue}"),
                        });
                    }
                }
            }
        }
    }
    let aligned = (0..paths.len())
        .map(|key| {
            path_from_sampled_contours(
                rows.iter()
                    .map(|row| (row.paths[key].clone().unwrap(), true))
                    .collect(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((aligned, contacts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, size: f64) -> Vec<Point> {
        vec![
            Point::new(x, y),
            Point::new(x + size, y),
            Point::new(x + size, y + size),
            Point::new(x, y + size),
        ]
    }

    fn shape(contours: Vec<Vec<Point>>) -> PathData {
        path_from_sampled_contours(contours.into_iter().map(|points| (points, true)).collect())
            .unwrap()
    }

    #[test]
    fn pairs_reorder_disjoint_contours_and_reject_collisions() {
        let from = shape(vec![square(0.0, 0.0, 10.0), square(30.0, 0.0, 10.0)]);
        let reversed = shape(vec![square(35.0, 0.0, 10.0), square(5.0, 0.0, 10.0)]);
        let aligned = prepare(
            &[from.clone(), reversed.clone()],
            &[vec![Some(0), Some(1)], vec![Some(1), Some(0)]],
            &[],
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        assert!(aligned[0].has_same_topology(&aligned[1]));
        let middle = aligned[0].morph(&aligned[1], 0.5).unwrap();
        assert_eq!(adaptive_flatten(&middle).unwrap().len(), 2);
        assert!(matches!(
            prepare(
                &[from.clone(), reversed.clone()],
                &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
                &[],
                MorphMethod::Auto,
                false,
            ),
            Err(GeometryError::UnsafeMorph(_))
        ));
        assert!(
            prepare(
                &[from, reversed],
                &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
                &[],
                MorphMethod::Auto,
                true,
            )
            .is_ok()
        );
    }

    #[test]
    fn null_pair_grows_and_shrinks_a_hole_inside_its_parent() {
        let outer = square(0.0, 0.0, 100.0);
        let hole = square(40.0, 40.0, 20.0);
        let plain = shape(vec![outer.clone()]);
        let perforated = shape(vec![outer.clone(), hole]);
        let keys = prepare(
            &[plain.clone(), perforated, plain],
            &[vec![Some(0), Some(0), Some(0)], vec![None, Some(1), None]],
            &[],
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        assert!(
            keys.windows(2)
                .all(|pair| pair[0].has_same_topology(&pair[1]))
        );
        for key in [0, 2] {
            let contours = adaptive_flatten(&keys[key]).unwrap();
            assert!(
                contours[1]
                    .points
                    .iter()
                    .all(|point| *point == contours[1].points[0])
            );
            assert!(morph_check::strictly_inside_contour(
                &contours[0].points,
                contours[1].points[0]
            ));
        }
        for middle in [
            keys[0].morph(&keys[1], 0.5).unwrap(),
            keys[1].morph(&keys[2], 0.5).unwrap(),
        ] {
            let contours = adaptive_flatten(&middle).unwrap();
            let nesting = morph_check::contour_hierarchy(
                &contours
                    .iter()
                    .map(|contour| contour.points.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            assert_eq!(nesting.parent, vec![None, Some(0)]);
            assert_eq!(
                morph_check::closed_contour_winding(&contours[1].points),
                Some(Ordering::Less)
            );
        }
    }

    #[test]
    fn hole_birth_chooses_filled_space_outside_existing_sibling_holes() {
        let outer = square(0.0, 0.0, 100.0);
        let existing = square(10.0, 10.0, 80.0);
        let center = interior_point(&outer, &[&existing], Some(Point::new(5.0, 50.0))).unwrap();
        assert!(region_contains(&outer, &[&existing], center));
        assert!(signed_clearance(&outer, &[&existing], center) >= 5.8);

        let small = vec![
            Point::new(2.0, 40.0),
            Point::new(8.0, 40.0),
            Point::new(8.0, 60.0),
            Point::new(2.0, 60.0),
        ];
        let from = shape(vec![outer.clone(), existing.clone()]);
        let to = shape(vec![outer, existing, small]);
        let keys = prepare(
            &[from, to],
            &[
                vec![Some(0), Some(0)],
                vec![Some(1), Some(1)],
                vec![None, Some(2)],
            ],
            &[],
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        let born = adaptive_flatten(&keys[0]).unwrap();
        let birth = born[2].points[0];
        assert!(region_contains(&born[0].points, &[&born[1].points], birth));
        assert!(signed_clearance(&born[0].points, &[&born[1].points], birth) >= 5.8);
    }

    #[test]
    fn null_pair_shrinks_an_outer_contour_and_bad_pair_maps_are_rejected() {
        let from = shape(vec![square(0.0, 0.0, 10.0), square(30.0, 0.0, 10.0)]);
        let to = shape(vec![square(5.0, 0.0, 10.0)]);
        let keys = prepare(
            &[from.clone(), to],
            &[vec![Some(0), Some(0)], vec![Some(1), None]],
            &[],
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        assert!(keys[0].has_same_topology(&keys[1]));
        assert!(
            adaptive_flatten(&keys[1]).unwrap()[1]
                .points
                .iter()
                .all(|point| *point == adaptive_flatten(&keys[1]).unwrap()[1].points[0])
        );
        assert!(matches!(
            prepare(
                &[from.clone(), from.clone()],
                &[vec![Some(0), Some(0)], vec![Some(0), Some(1)]],
                &[],
                MorphMethod::Auto,
                false,
            ),
            Err(GeometryError::InvalidMorphPairs(_))
        ));
        let holed = shape(vec![square(0.0, 0.0, 100.0), square(40.0, 40.0, 20.0)]);
        assert!(matches!(
            prepare(
                &[holed, from],
                &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
                &[],
                MorphMethod::Auto,
                false,
            ),
            Err(GeometryError::InvalidMorphPairs(_))
        ));
    }

    #[test]
    fn anchor_rows_follow_paired_contours_and_keep_hole_winding() {
        let from = shape(vec![square(0.0, 0.0, 10.0), square(30.0, 0.0, 10.0)]);
        let reversed = shape(vec![square(35.0, 0.0, 10.0), square(5.0, 0.0, 10.0)]);
        let pairs = [vec![Some(0), Some(1)], vec![Some(1), Some(0)]];
        let anchors = [
            vec![Point::new(10.0, 5.0), Point::new(15.0, 5.0)],
            vec![Point::new(40.0, 5.0), Point::new(45.0, 5.0)],
        ];
        let aligned = prepare(
            &[from.clone(), reversed.clone()],
            &pairs,
            &anchors,
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        assert_eq!(aligned[0].points.len(), 2 * POLAR_MORPH_POINTS);
        for (index, source, target) in [
            (0, anchors[0][0], anchors[0][1]),
            (POLAR_MORPH_POINTS, anchors[1][0], anchors[1][1]),
        ] {
            assert_eq!(aligned[0].points[index], source);
            assert_eq!(aligned[1].points[index], target);
        }
        assert!(matches!(
            prepare(
                &[from.clone(), reversed.clone()],
                &pairs,
                &[vec![anchors[0][0], anchors[1][1]]],
                MorphMethod::Auto,
                false,
            ),
            Err(GeometryError::InvalidMorphPairs(_))
        ));

        let holed_from = shape(vec![square(0.0, 0.0, 100.0), square(40.0, 40.0, 20.0)]);
        let holed_to = shape(vec![square(10.0, 0.0, 100.0), square(50.0, 40.0, 20.0)]);
        let hole_anchor = [vec![Point::new(60.0, 50.0), Point::new(70.0, 50.0)]];
        let holes = prepare(
            &[holed_from.clone(), holed_to.clone()],
            &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
            &hole_anchor,
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        let contours = adaptive_flatten(&holes[0]).unwrap();
        assert_eq!(
            morph_check::closed_contour_winding(&contours[1].points),
            Some(Ordering::Less)
        );
        let index = holes[0]
            .points
            .iter()
            .position(|point| *point == hole_anchor[0][0])
            .unwrap();
        assert_eq!(holes[1].points[index], hole_anchor[0][1]);
        assert!(matches!(
            prepare(
                &[holed_from, holed_to],
                &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
                &[vec![hole_anchor[0][0], Point::new(110.0, 50.0)]],
                MorphMethod::Auto,
                false,
            ),
            Err(GeometryError::InvalidMorphPairs(_))
        ));
    }

    #[test]
    fn contours_report_a_tangent_contact_without_crossing() {
        let first_from = vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, -1.0),
        ];
        let first_to = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 0.0),
        ];
        let second_from = vec![
            Point::new(0.75, 0.25),
            Point::new(0.75, 1.25),
            Point::new(-0.25, 0.25),
        ];
        let second_to = vec![
            Point::new(-0.25, 0.25),
            Point::new(-0.25, 1.25),
            Point::new(-1.25, 0.25),
        ];
        let anchors = first_from
            .iter()
            .zip(&first_to)
            .chain(second_from.iter().zip(&second_to))
            .map(|(left, right)| vec![*left, *right])
            .collect::<Vec<_>>();
        let from = shape(vec![first_from, second_from]);
        let to = shape(vec![first_to, second_to]);
        let result = prepare_with_contacts(
            &[from, to],
            &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
            &anchors,
            MorphMethod::ArcLength,
            false,
            false,
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(!result.unwrap().1.is_empty());
    }

    #[test]
    fn convex_multi_contour_alignment_keeps_constructed_zero_edges() {
        let from = shape(vec![
            vec![
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(0.0, -1.0),
            ],
            square(20.0, 20.0, 2.0),
        ]);
        let to = shape(vec![
            vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 1.0),
                Point::new(1.0, 0.0),
            ],
            square(20.0, 20.0, 2.0),
        ]);
        let aligned = prepare(
            &[from, to],
            &[vec![Some(0), Some(0)], vec![Some(1), Some(1)]],
            &[],
            MorphMethod::Convex,
            false,
        )
        .unwrap();
        assert!(aligned[0].has_same_topology(&aligned[1]));
        assert!(aligned[0].points.windows(2).any(|pair| pair[0] == pair[1]));
    }
}
