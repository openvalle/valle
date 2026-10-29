use super::*;
use crate::scene3d::{MaterialFrameState, MeshFrameState};

fn state(parents: Vec<Transform3D>, transform: Transform3D) -> MeshFrameState {
    MeshFrameState {
        particle_time: None,
        parents,
        transform,
        key: "subject".into(),
        material: MaterialFrameState::default(),
        material_overrides: vec![],
        animation_time: None,
        nodes: vec![],
    }
}

#[test]
fn group_order_and_mirror_follow_parent_space() {
    let mesh = state(
        vec![
            Transform3D {
                translation: Vec3::new(10.0, 0.0, 0.0),
                rotation_degrees: Vec3::new(0.0, 0.0, 90.0),
                scale: Vec3::new(2.0, 1.0, 1.0),
            },
            Transform3D {
                translation: Vec3::new(1.0, 0.0, 0.0),
                scale: Vec3::new(-1.0, 3.0, 1.0),
                ..Default::default()
            },
        ],
        Transform3D {
            translation: Vec3::new(1.0, 1.0, 0.0),
            ..Default::default()
        },
    );
    let transform = MeshTransform::from_state(&mesh).unwrap();
    let p = transform.point(V3::new(0.0, 0.0, 0.0));
    assert!((p.x - 7.0).abs() < 1e-5 && p.y.abs() < 1e-5 && p.z.abs() < 1e-5);
    assert!(transform.mirrored());
}

#[test]
fn normals_remain_perpendicular_under_nested_nonuniform_scales() {
    let mesh = state(
        vec![
            Transform3D {
                rotation_degrees: Vec3::new(13.0, 27.0, 9.0),
                scale: Vec3::new(2.0, 0.5, -1.0),
                ..Default::default()
            },
            Transform3D {
                rotation_degrees: Vec3::new(0.0, 39.0, 18.0),
                scale: Vec3::new(0.8, 1.6, 1.0),
                ..Default::default()
            },
        ],
        Transform3D {
            rotation_degrees: Vec3::new(19.0, 11.0, 7.0),
            ..Default::default()
        },
    );
    let transform = MeshTransform::from_state(&mesh).unwrap();
    let origin = transform.point(V3::new(0.0, 0.0, 0.0));
    let n = transform.normal(V3::new(0.0, 0.0, 1.0));
    assert!(
        n.dot(transform.point(V3::new(1.0, 0.0, 0.0)) - origin)
            .abs()
            < 1e-5
    );
    assert!(
        n.dot(transform.point(V3::new(0.0, 1.0, 0.0)) - origin)
            .abs()
            < 1e-5
    );
    assert!((n.dot(n) - 1.0).abs() < 1e-6);
}

#[test]
fn group_motion_updates_anchors_and_replays_without_stale_geometry() {
    let (prepared, mut frame) = geometry_reuse_tests::fixture(
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
        true,
    );
    let baseline = render_scene(&prepared, &frame).unwrap();
    frame.meshes[0].parents = vec![Transform3D {
        translation: Vec3::new(1.0, 0.0, 0.0),
        ..Default::default()
    }];
    let shifted = render_scene_with_workers(&prepared, &frame, 4).unwrap();
    assert!(shifted.anchors[0].screen[0] > baseline.anchors[0].screen[0]);
    assert_eq!(shifted.anchors[0].depth, baseline.anchors[0].depth);
    assert_ne!(shifted.premul_rgba16f, baseline.premul_rgba16f);
    frame.meshes[0].parents.clear();
    assert_eq!(render_scene(&prepared, &frame).unwrap(), baseline);
}

#[test]
fn invalid_group_chains_are_rejected_before_rendering() {
    let (prepared, mut frame) = geometry_reuse_tests::fixture(
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
        false,
    );
    frame.meshes[0].parents = vec![Transform3D::default(); super::super::MAX_NODE_DEPTH + 1];
    assert!(frame.validate_for(&prepared.scene).is_err());
    frame.meshes[0].parents = vec![Transform3D {
        scale: Vec3::new(1.0, 0.0, 1.0),
        ..Default::default()
    }];
    assert!(frame.validate_for(&prepared.scene).is_err());
}
