#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::PathVerb;
use valle_motion::{Expr, ExprId, MotionValue, PathData};

fn constant_path(artifact: &valle_motion::SceneArtifact, id: ExprId) -> &PathData {
    match &artifact.exprs[id.0 as usize] {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => path,
        _ => panic!("paired correspondence must be prepared once"),
    }
}

#[test]
fn explicit_pairs_prepare_multi_contour_morph_and_null_hole_sequence() {
    let source = include_str!("fixtures/motion/composition/paired-contours.motion.tsx");
    let artifact = compile_motion(source)
        .expect("paired contours compile")
        .artifact;
    artifact.validate().unwrap();

    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let (from, to) = (constant_path(&artifact, from), constant_path(&artifact, to));
    assert!(from.has_same_topology(to));
    assert_eq!(
        from.verbs
            .iter()
            .filter(|verb| **verb == PathVerb::Move)
            .count(),
        2
    );
    assert_eq!((from.points[0].x, to.points[0].x), (40.0, 80.0));
    assert_eq!((from.points[4].x, to.points[4].x), (240.0, 280.0));

    let stops = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::Interpolate { stops, .. }
                if stops.len() == 3
                    && stops
                        .iter()
                        .all(|stop| matches!(stop.output, MotionValue::PathData(_))) =>
            {
                Some(stops)
            }
            _ => None,
        })
        .unwrap();
    let keys = stops
        .iter()
        .map(|stop| match &stop.output {
            MotionValue::PathData(path) => path,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    assert!(
        keys.windows(2)
            .all(|pair| pair[0].has_same_topology(pair[1]))
    );
    assert_eq!(keys[0].points.len(), keys[1].points.len());
    assert!(
        keys[0].points[4..]
            .iter()
            .all(|point| *point == keys[0].points[4])
    );
    assert!(
        keys[2].points[4..]
            .iter()
            .all(|point| *point == keys[2].points[4])
    );
    let hole = &keys[1].points[4..];
    let signed_area = (0..hole.len())
        .map(|index| {
            hole[index].x * hole[(index + 1) % hole.len()].y
                - hole[index].y * hole[(index + 1) % hole.len()].x
        })
        .sum::<f64>();
    assert!(
        signed_area < 0.0,
        "nested hole must wind opposite its outer contour"
    );
}

#[test]
fn pair_indices_and_cross_contour_motion_are_checked() {
    let source = include_str!("fixtures/motion/composition/paired-contours.motion.tsx");
    for invalid in [
        source.replace("pairs: [[0, 1], [1, 0]]", "pairs: [[0, 0], [1, 0]]"),
        source.replace("pairs: [[0, 1], [1, 0]]", "pairs: [[0, 1], [1, 2]]"),
        source.replace("pairs: [[0, 1], [1, 0]]", "pairs: [[0, 1], [1, null]]"),
        source.replace("[null, 1, null]", "[null, 1, 0]"),
        source.replace("pairs: [[0, 1], [1, 0]]", "pairs: [[0.5, 1], [1, 0]]"),
    ] {
        assert!(compile_motion(&invalid).is_err());
    }
    let crossing = source.replace("pairs: [[0, 1], [1, 0]]", "pairs: [[0, 0], [1, 1]]");
    assert!(compile_motion(&crossing).is_err());
    assert!(
        compile_motion(&crossing.replace(
            "pairs: [[0, 0], [1, 1]]",
            "pairs: [[0, 0], [1, 1]], allowSelfIntersection: true"
        ))
        .is_ok()
    );
}

#[test]
fn authored_anchors_are_assigned_to_paired_contours_in_pair_and_sequence() {
    let source = include_str!("fixtures/motion/composition/paired-anchors.motion.tsx");
    let artifact = compile_motion(source)
        .expect("paired anchors compile")
        .artifact;
    artifact.validate().unwrap();
    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let (from, to) = (constant_path(&artifact, from), constant_path(&artifact, to));
    assert!(from.has_same_topology(to));
    assert_eq!(from.points.len(), 512);
    for (index, source, target) in [
        (0, (120.0, 70.0), (160.0, 70.0)),
        (256, (320.0, 70.0), (360.0, 70.0)),
    ] {
        assert_eq!((from.points[index].x, from.points[index].y), source);
        assert_eq!((to.points[index].x, to.points[index].y), target);
    }
    let stops = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::Interpolate { stops, .. }
                if stops.len() == 3
                    && stops
                        .iter()
                        .all(|stop| matches!(stop.output, MotionValue::PathData(_))) =>
            {
                Some(stops)
            }
            _ => None,
        })
        .unwrap();
    for stop in stops {
        let MotionValue::PathData(path) = &stop.output else {
            unreachable!()
        };
        assert_eq!((path.points[0].x, path.points[0].y), (520.0, 260.0));
        assert_eq!(path.points.len(), 512);
    }
    for invalid in [
        source.replace("point(160, 70)", "point(360, 70)"),
        source.replace("point(160, 70)", "point(200, 70)"),
        source.replace(
            "point(520, 260), point(520, 260), point(520, 260)",
            "point(520, 260), point(480, 260), point(520, 260)",
        ),
    ] {
        assert!(compile_motion(&invalid).is_err());
    }
}

#[test]
fn multi_contours_infer_pairs_and_use_anchors_as_matching_constraints() {
    let explicit = include_str!("fixtures/motion/composition/paired-contours.motion.tsx");
    let automatic = explicit
        .replace("pairs: [[0, 1], [1, 0]],", "")
        .replace("pairs: [[0, 0, 0], [null, 1, null]],", "");
    assert_ne!(automatic, explicit);
    let artifact = compile_motion(&automatic)
        .expect("multiple contours infer correspondence")
        .artifact;
    artifact.validate().unwrap();
    let (from, to) = artifact
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let (from, to) = (constant_path(&artifact, from), constant_path(&artifact, to));
    assert!(from.has_same_topology(to));
    assert_eq!((from.points[0].x, to.points[0].x), (40.0, 80.0));
    assert_eq!((from.points[4].x, to.points[4].x), (240.0, 280.0));

    let explicit_anchors = include_str!("fixtures/motion/composition/paired-anchors.motion.tsx");
    let automatic_anchors = explicit_anchors
        .replace("pairs: [[0, 1], [1, 0]],", "")
        .replace("pairs: [[0, 0, 0], [null, 1, null]],", "");
    let anchored = compile_motion(&automatic_anchors)
        .expect("anchors constrain inferred contour matching")
        .artifact;
    anchored.validate().unwrap();
    let (from, to) = anchored
        .exprs
        .iter()
        .find_map(|expr| match expr {
            Expr::PathMorph { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .unwrap();
    let (from, to) = (constant_path(&anchored, from), constant_path(&anchored, to));
    assert_eq!(from.points.len(), 512);
    assert_eq!((from.points[0].x, to.points[0].x), (120.0, 160.0));
    assert_eq!((from.points[256].x, to.points[256].x), (320.0, 360.0));
}
