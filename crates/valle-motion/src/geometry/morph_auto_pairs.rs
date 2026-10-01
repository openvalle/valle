//! Deterministic hierarchy-aware contour matching before the checked morph preparer.

use super::{
    FlattenedContour, GeometryError, PathData, Point, adaptive_flatten, morph_anchor, morph_check,
};
use num_rational::BigRational;
use num_traits::Zero;
use std::collections::HashMap;

#[derive(Clone)]
struct Descriptor {
    x: BigRational,
    y: BigRational,
    width: BigRational,
    height: BigRational,
}

fn exact(value: f64) -> BigRational {
    BigRational::from_float(value).expect("validated finite contour coordinate")
}

fn descriptors(
    contours: &[FlattenedContour],
    hierarchy: &morph_check::ContourHierarchy,
) -> Vec<Descriptor> {
    let boxes = contours
        .iter()
        .map(|contour| {
            let (mut min_x, mut max_x, mut min_y, mut max_y) = (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            );
            for point in &contour.points {
                min_x = min_x.min(point.x);
                max_x = max_x.max(point.x);
                min_y = min_y.min(point.y);
                max_y = max_y.max(point.y);
            }
            (exact(min_x), exact(max_x), exact(min_y), exact(max_y))
        })
        .collect::<Vec<_>>();
    boxes
        .iter()
        .enumerate()
        .map(|(index, (min_x, max_x, min_y, max_y))| {
            let two = BigRational::from_integer(2.into());
            let mut x = (min_x + max_x) / &two;
            let mut y = (min_y + max_y) / &two;
            if let Some(parent) = hierarchy.parent[index] {
                let (px0, px1, py0, py1) = &boxes[parent];
                x -= (px0 + px1) / &two;
                y -= (py0 + py1) / &two;
            }
            Descriptor {
                x,
                y,
                width: max_x - min_x,
                height: max_y - min_y,
            }
        })
        .collect()
}

fn squared(value: BigRational) -> BigRational {
    &value * &value
}

fn cost(left: &Descriptor, right: &Descriptor) -> BigRational {
    let center = squared(&left.x - &right.x) + squared(&left.y - &right.y);
    let extent = squared(&left.width - &right.width) + squared(&left.height - &right.height);
    center * BigRational::from_integer(16.into()) + extent
}

fn choose_assignment(
    row: usize,
    used: u16,
    scores: &[Vec<BigRational>],
    memo: &mut HashMap<(usize, u16), (BigRational, Vec<usize>)>,
) -> (BigRational, Vec<usize>) {
    if row == scores.len() {
        return (BigRational::zero(), Vec::new());
    }
    if let Some(cached) = memo.get(&(row, used)) {
        return cached.clone();
    }
    let mut best: Option<(BigRational, Vec<usize>)> = None;
    for column in 0..scores[row].len() {
        if used & (1 << column) != 0 {
            continue;
        }
        let (tail_cost, tail) = choose_assignment(row + 1, used | (1 << column), scores, memo);
        let mut picks = Vec::with_capacity(tail.len() + 1);
        picks.push(column);
        picks.extend(tail);
        let candidate = (&scores[row][column] + tail_cost, picks);
        if best.as_ref().is_none_or(|current| candidate < *current) {
            best = Some(candidate);
        }
    }
    let best = best.unwrap();
    memo.insert((row, used), best.clone());
    best
}

fn minimum_cost_pairs(
    from: &[usize],
    to: &[usize],
    from_description: &[Descriptor],
    to_description: &[Descriptor],
) -> Vec<(usize, usize)> {
    if from.is_empty() || to.is_empty() {
        return Vec::new();
    }
    let from_is_shorter = from.len() <= to.len();
    let (rows, columns) = if from_is_shorter {
        (from, to)
    } else {
        (to, from)
    };
    let scores = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| {
                    if from_is_shorter {
                        cost(&from_description[*row], &to_description[*column])
                    } else {
                        cost(&from_description[*column], &to_description[*row])
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let (_, picks) = choose_assignment(0, 0, &scores, &mut HashMap::new());
    rows.iter()
        .zip(picks)
        .map(|(row, column)| {
            if from_is_shorter {
                (*row, columns[column])
            } else {
                (columns[column], *row)
            }
        })
        .collect()
}

fn pin(
    from: usize,
    to: usize,
    from_parent: &[Option<usize>],
    to_parent: &[Option<usize>],
    pinned: &mut HashMap<usize, usize>,
) -> Result<(), GeometryError> {
    if pinned.get(&from).is_some_and(|previous| *previous != to)
        || pinned
            .iter()
            .any(|(other, target)| *other != from && *target == to)
    {
        return Err(GeometryError::InvalidMorphAnchors);
    }
    pinned.insert(from, to);
    match (from_parent[from], to_parent[to]) {
        (None, None) => Ok(()),
        (Some(left), Some(right)) => pin(left, right, from_parent, to_parent, pinned),
        _ => Err(GeometryError::InvalidMorphAnchors),
    }
}

fn match_level(
    from_parent: Option<usize>,
    to_parent: Option<usize>,
    from_hierarchy: &morph_check::ContourHierarchy,
    to_hierarchy: &morph_check::ContourHierarchy,
    from_description: &[Descriptor],
    to_description: &[Descriptor],
    pinned: &HashMap<usize, usize>,
    output: &mut Vec<(usize, usize)>,
) -> Result<(), GeometryError> {
    let from = from_hierarchy
        .parent
        .iter()
        .enumerate()
        .filter_map(|(index, parent)| (*parent == from_parent).then_some(index))
        .collect::<Vec<_>>();
    let to = to_hierarchy
        .parent
        .iter()
        .enumerate()
        .filter_map(|(index, parent)| (*parent == to_parent).then_some(index))
        .collect::<Vec<_>>();
    let fixed = from
        .iter()
        .filter_map(|source| pinned.get(source).map(|target| (*source, *target)))
        .collect::<Vec<_>>();
    if fixed.iter().any(|(_, target)| !to.contains(target)) {
        return Err(GeometryError::InvalidMorphAnchors);
    }
    let remaining_from = from
        .iter()
        .filter(|source| !fixed.iter().any(|(current, _)| current == *source))
        .copied()
        .collect::<Vec<_>>();
    let remaining_to = to
        .iter()
        .filter(|target| !fixed.iter().any(|(_, current)| current == *target))
        .copied()
        .collect::<Vec<_>>();
    let mut matched = fixed;
    matched.extend(minimum_cost_pairs(
        &remaining_from,
        &remaining_to,
        from_description,
        to_description,
    ));
    matched.sort_unstable();
    for (source, target) in matched {
        output.push((source, target));
        match_level(
            Some(source),
            Some(target),
            from_hierarchy,
            to_hierarchy,
            from_description,
            to_description,
            pinned,
            output,
        )?;
    }
    Ok(())
}

pub(super) fn infer(
    paths: &[PathData],
    anchors: &[Vec<Point>],
) -> Result<Vec<Vec<Option<usize>>>, GeometryError> {
    if !(2..=16).contains(&paths.len()) {
        return Err(GeometryError::TooComplex);
    }
    if anchors.len() > 32 {
        return Err(GeometryError::InvalidMorphAnchors);
    }
    let contours = paths
        .iter()
        .map(adaptive_flatten)
        .collect::<Result<Vec<_>, _>>()?;
    if contours.iter().any(|path| path.len() > 8) {
        return Err(GeometryError::TooComplex);
    }
    let hierarchy = contours
        .iter()
        .map(|path| {
            if path.iter().any(|contour| !contour.closed) {
                return None;
            }
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
    let descriptions = contours
        .iter()
        .zip(&hierarchy)
        .map(|(path, nesting)| descriptors(path, nesting))
        .collect::<Vec<_>>();
    let mut pins = vec![HashMap::new(); paths.len() - 1];
    for anchor_row in anchors {
        if anchor_row.len() != paths.len() {
            return Err(GeometryError::InvalidMorphAnchors);
        }
        let indices = anchor_row
            .iter()
            .enumerate()
            .map(|(key, anchor)| {
                let matching = contours[key]
                    .iter()
                    .enumerate()
                    .filter(|(_, contour)| morph_anchor::anchor_matches_contour(contour, *anchor))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                (matching.len() == 1)
                    .then(|| matching[0])
                    .ok_or(GeometryError::InvalidMorphAnchors)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for key in 0..paths.len() - 1 {
            pin(
                indices[key],
                indices[key + 1],
                &hierarchy[key].parent,
                &hierarchy[key + 1].parent,
                &mut pins[key],
            )?;
        }
    }
    let mut transition = Vec::with_capacity(paths.len() - 1);
    for key in 0..paths.len() - 1 {
        let mut matching = Vec::new();
        match_level(
            None,
            None,
            &hierarchy[key],
            &hierarchy[key + 1],
            &descriptions[key],
            &descriptions[key + 1],
            &pins[key],
            &mut matching,
        )?;
        transition.push(matching);
    }
    let mut rows = (0..contours[0].len())
        .map(|index| {
            let mut row = vec![None; paths.len()];
            row[0] = Some(index);
            row
        })
        .collect::<Vec<_>>();
    let mut row_parent = hierarchy[0].parent.clone();
    let mut current = (0..contours[0].len()).collect::<Vec<_>>();
    for key in 0..paths.len() - 1 {
        let mut next = vec![None; contours[key + 1].len()];
        for (source, target) in &transition[key] {
            let row = current[*source];
            rows[row][key + 1] = Some(*target);
            next[*target] = Some(row);
        }
        let mut births = (0..next.len())
            .filter(|index| next[*index].is_none())
            .collect::<Vec<_>>();
        births.sort_by_key(|index| (hierarchy[key + 1].depth[*index], *index));
        for index in births {
            let parent = hierarchy[key + 1].parent[index].map(|outer| next[outer].unwrap());
            let resumed = (0..rows.len())
                .filter(|row| {
                    rows[*row][key].is_none()
                        && rows[*row][key + 1].is_none()
                        && row_parent[*row] == parent
                        && rows[*row].iter().take(key + 1).any(Option::is_some)
                })
                .map(|row| {
                    let last_key = (0..=key).rev().find(|at| rows[row][*at].is_some()).unwrap();
                    let old = rows[row][last_key].unwrap();
                    (
                        cost(&descriptions[last_key][old], &descriptions[key + 1][index]),
                        row,
                    )
                })
                .min();
            let row = if let Some((_, row)) = resumed {
                row
            } else {
                let row = rows.len();
                rows.push(vec![None; paths.len()]);
                row_parent.push(parent);
                row
            };
            rows[row][key + 1] = Some(index);
            next[index] = Some(row);
        }
        current = next.into_iter().map(Option::unwrap).collect();
    }
    if rows.is_empty() || rows.len() > 8 {
        return Err(GeometryError::TooComplex);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{MorphMethod, path_from_sampled_contours};

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
    fn automatically_matches_reordered_outers_and_respects_anchor_pins() {
        let from = shape(vec![square(0.0, 0.0, 10.0), square(30.0, 0.0, 10.0)]);
        let reversed = shape(vec![square(35.0, 0.0, 10.0), square(5.0, 0.0, 10.0)]);
        let paths = [from, reversed];
        assert_eq!(
            infer(&paths, &[]).unwrap(),
            vec![vec![Some(0), Some(1)], vec![Some(1), Some(0)]]
        );
        let aligned = PathData::automatic_multi_morph_sequence_with_options(
            &paths,
            &[],
            MorphMethod::Auto,
            false,
        )
        .unwrap();
        assert!(aligned[0].has_same_topology(&aligned[1]));
        let pinned = [vec![Point::new(10.0, 5.0), Point::new(45.0, 5.0)]];
        assert_eq!(
            infer(&paths, &pinned).unwrap(),
            vec![vec![Some(0), Some(0)], vec![Some(1), Some(1)]]
        );
    }

    #[test]
    fn automatically_tracks_holes_through_birth_death_and_reappearance() {
        let outer = square(0.0, 0.0, 100.0);
        let hole = square(40.0, 40.0, 20.0);
        let plain = shape(vec![outer.clone()]);
        let holed = shape(vec![outer.clone(), hole.clone()]);
        let birth = [plain.clone(), holed.clone(), plain.clone()];
        assert_eq!(
            infer(&birth, &[]).unwrap(),
            vec![vec![Some(0), Some(0), Some(0)], vec![None, Some(1), None]]
        );
        assert!(
            PathData::automatic_multi_morph_sequence_with_options(
                &birth,
                &[],
                MorphMethod::Auto,
                false,
            )
            .is_ok()
        );
        let reappear = [holed.clone(), plain, holed];
        assert_eq!(
            infer(&reappear, &[]).unwrap(),
            vec![
                vec![Some(0), Some(0), Some(0)],
                vec![Some(1), None, Some(1)]
            ]
        );
        assert!(
            PathData::automatic_multi_morph_sequence_with_options(
                &reappear,
                &[],
                MorphMethod::Auto,
                false,
            )
            .is_ok()
        );
    }

    #[test]
    fn hole_matching_stays_with_its_parent_and_not_input_order() {
        let outer = square(0.0, 0.0, 100.0);
        let left_hole = square(20.0, 40.0, 10.0);
        let right_hole = square(70.0, 40.0, 10.0);
        let paths = [
            shape(vec![outer.clone(), left_hole.clone(), right_hole.clone()]),
            shape(vec![outer, right_hole, left_hole]),
        ];
        assert_eq!(
            infer(&paths, &[]).unwrap(),
            vec![
                vec![Some(0), Some(0)],
                vec![Some(1), Some(2)],
                vec![Some(2), Some(1)],
            ]
        );
        assert!(
            PathData::automatic_multi_morph_sequence_with_options(
                &paths,
                &[],
                MorphMethod::Auto,
                false,
            )
            .is_ok()
        );
    }
}
