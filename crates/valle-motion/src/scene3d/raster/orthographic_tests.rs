use super::*;

#[test]
fn orthographic_projection_preserves_size_across_depth_and_changes_with_height() {
    let (prepared, mut frame) = geometry_reuse_tests::fixture(
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
        false,
    );
    frame.camera.orthographic_height = Some(4.0);
    let camera = Camera::new(&frame, 128, 96).unwrap();
    for z in [0.0, 8.0, -8.0] {
        let p = camera.project_world(V3::new(1.0, 1.0, z)).unwrap();
        assert_eq!((p.x, p.y, p.view_z), (88.0, 24.0, 15.0 - z));
    }
    let before = render_scene(&prepared, &frame).unwrap();
    frame.meshes[0].transform.translation.0[2] = 5.0;
    let after = render_scene(&prepared, &frame).unwrap();
    assert_eq!(before.object_ids, after.object_ids);
    assert_eq!(before.premul_rgba16f, after.premul_rgba16f);
    let mut visible = 0;
    for (a, b) in before.depth.iter().zip(&after.depth) {
        if *a != CLEAR_DEPTH {
            assert!((a - b - 5.0).abs() < 1e-5);
            visible += 1;
        }
    }
    assert!(visible > 100);
    frame.camera.orthographic_height = Some(8.0);
    let p = Camera::new(&frame, 128, 96)
        .unwrap()
        .project_world(V3::new(1.0, 1.0, 0.0))
        .unwrap();
    assert_eq!((p.x, p.y), (76.0, 36.0));
}

#[test]
fn orthographic_clipping_and_plane_interpolation_are_linear() {
    let (_, mut frame) = geometry_reuse_tests::fixture(
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
        false,
    );
    frame.camera.orthographic_height = Some(4.0);
    let camera = Camera::new(&frame, 128, 96).unwrap();
    let vertex = |x, y, z| WorldVertex {
        position: V3::new(x, y, z),
        normal: V3::Y,
        uv: V2 { x, y },
    };
    // At this height the right side is x = 8/3, irrespective of distance.
    assert!(
        camera
            .project_vertex(vertex(2.6, 0.0, 0.0))
            .screen
            .is_some()
    );
    assert!(
        camera
            .project_vertex(vertex(2.7, 0.0, -10.0))
            .screen
            .is_none()
    );
    let triangle = [
        vertex(-1.0, -1.0, 0.0),
        vertex(1.0, -1.0, 4.0),
        vertex(0.0, 1.0, 8.0),
    ];
    let mut projected = vec![];
    project_world_triangle(triangle, &camera, |p| projected.push(p));
    assert_eq!(projected.len(), 1);
    let vertices = projected[0];
    assert_eq!(vertices.map(|v| v.inv_z), [1.0; 3]);
    let weights = [0.25, 0.25, 0.5];
    let depth = (0..3).map(|i| weights[i] * vertices[i].view_z).sum::<f32>();
    assert_eq!(depth, 10.0);
    let uv = interpolated_uv(vertices, weights);
    assert_eq!((uv.x, uv.y), (0.0, 0.0));
    for height in [0.0, -1.0, f32::NAN, f32::INFINITY, 100_001.0] {
        frame.camera.orthographic_height = Some(height);
        assert!(frame.camera.validate().is_err());
    }
}
