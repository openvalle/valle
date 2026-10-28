use super::*;
use crate::scene3d::{
    AnchorSpec, CameraFrameState, LightKind, MaterialFrameState, MeshFrameState, MeshSpec,
};

fn without_normals(bytes: &[u8]) -> Vec<u8> {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    for mesh in json["meshes"].as_array_mut().unwrap() {
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            primitive["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("NORMAL");
            primitive["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("TANGENT");
            if let Some(targets) = primitive.get_mut("targets").and_then(|v| v.as_array_mut()) {
                for target in targets {
                    target.as_object_mut().unwrap().remove("NORMAL");
                    target.as_object_mut().unwrap().remove("TANGENT");
                }
            }
        }
    }
    let mut json = serde_json::to_vec(&json).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut result = bytes[..12].to_vec();
    result.extend_from_slice(&(json.len() as u32).to_le_bytes());
    result.extend_from_slice(b"JSON");
    result.extend_from_slice(&json);
    result.extend_from_slice(&bytes[20 + length..]);
    let total = result.len() as u32;
    result[8..12].copy_from_slice(&total.to_le_bytes());
    result
}

pub(super) fn fixture(bytes: &[u8], shadows: bool) -> (PreparedScene, Frame3DState) {
    let mut scene = Scene3DSpec {
        pbr: Default::default(),
        meshes: vec![MeshSpec {
            key: "subject".into(),
            model_control: Some("model".into()),
            geometry: None,
            material: MaterialSpec {
                kind: Some(MaterialKind::Lambert),
                ..Default::default()
            },
            material_overrides: Vec::new(),
            animation_clip: Some(0),
            node_ids: Vec::new(),
        }],
        lights: vec![LightKind::Ambient, LightKind::Directional],
        anchors: vec![AnchorSpec {
            key: "label".into(),
            parent: "subject".into(),
            position: Vec3::new(0.0, 0.0, 0.0),
        }],
    };
    scene.pbr.shadows = shadows;
    let resources = SceneResources {
        models: [(
            "model".into(),
            Arc::new(super::super::admit_glb(bytes).unwrap()),
        )]
        .into(),
        ..Default::default()
    };
    let prepared = prepare_scene(&scene, 128, 96, &resources).unwrap();
    let frame = Frame3DState {
        exposure: 1.0,
        environment_intensity: 1.0,
        environment_rotation_degrees: 0.0,
        camera: CameraFrameState {
            position: Vec3::new(0.0, 0.0, 15.0),
            target: Vec3::new(0.0, 0.0, 0.0),
            fov_y_degrees: 45.0,
            near: 0.1,
            far: 40.0,
            depth_of_field: None,
        },
        meshes: vec![MeshFrameState {
            key: "subject".into(),
            material: MaterialFrameState {
                color: Some(Color4([0.3, 0.6, 0.9, 1.0])),
                ..Default::default()
            },
            material_overrides: Vec::new(),
            animation_time: Some(0.0),
            transform: Transform3D::default(),
            nodes: Vec::new(),
        }],
        lights: vec![
            LightFrameState::Ambient {
                color: Color4([1.0; 4]),
                intensity: 0.3,
            },
            LightFrameState::Directional {
                color: Color4([1.0; 4]),
                direction: Vec3::new(0.3, 0.8, 1.0),
                intensity: 0.7,
            },
        ],
    };
    (prepared, frame)
}

#[test]
fn indexed_deformation_and_projection_match_triangle_reference_bits() {
    let rigged = include_bytes!("../../../tests/fixtures/scene3d/rigged-simple.glb");
    let morph = include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb");
    let flat = without_normals(morph);
    let mut visible = 0;
    for bytes in [rigged.as_slice(), morph.as_slice(), flat.as_slice()] {
        for shadows in [false, true] {
            let (prepared, mut frame) = fixture(bytes, shadows);
            for (time, mirror, close) in [
                (1.0, false, false),
                (0.0, true, false),
                (0.5, false, true),
                (1.0, false, false),
            ] {
                frame.meshes[0].animation_time = Some(time);
                frame.meshes[0].transform.rotation_degrees = Vec3::new(7.0, 19.0, -3.0);
                frame.meshes[0].transform.scale.0[0] = if mirror { -1.0 } else { 1.0 };
                frame.camera.position.0[2] = if close { 1.0 } else { 15.0 };
                let expected =
                    render_scene_with_geometry_reuse(&prepared, &frame, None, 1, true, false)
                        .unwrap();
                let actual =
                    render_scene_with_geometry_reuse(&prepared, &frame, None, 4, true, true)
                        .unwrap();
                assert_eq!(
                    actual, expected,
                    "shadows={shadows}, time={time}, mirror={mirror}, close={close}"
                );
                assert_eq!(actual.depth_f32_le_bytes(), expected.depth_f32_le_bytes());
                for (a, b) in actual.anchors.iter().zip(&expected.anchors) {
                    assert_eq!(a.screen.map(f32::to_bits), b.screen.map(f32::to_bits));
                    assert_eq!(a.depth.to_bits(), b.depth.to_bits());
                }
                visible += usize::from(
                    actual
                        .object_ids
                        .iter()
                        .any(|&id| id != BACKGROUND_OBJECT_ID),
                );
            }
        }
    }
    assert!(
        visible >= 12,
        "deformation cases must render visible triangles"
    );
}

#[test]
#[ignore = "manual Scene3D geometry preparation throughput comparison"]
fn indexed_geometry_preparation_benchmark() {
    use std::{hint::black_box, time::Instant};
    for bytes in [
        include_bytes!("../../../tests/fixtures/scene3d/rigged-simple.glb").as_slice(),
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb").as_slice(),
    ] {
        let (prepared, mut frame) = fixture(bytes, false);
        frame.meshes[0].animation_time = Some(0.7);
        let camera = Camera::new(&frame, prepared.width, prepared.height).unwrap();
        for reuse in [false, true] {
            for _ in 0..50 {
                black_box(collect_geometry_reusing(&prepared, &frame, &camera, reuse).unwrap());
            }
        }
        let mut rows = Vec::new();
        for reuse in [false, true, true, false] {
            let start = Instant::now();
            for _ in 0..1000 {
                black_box(collect_geometry_reusing(&prepared, &frame, &camera, reuse).unwrap());
            }
            rows.push((reuse, start.elapsed().as_secs_f64()));
        }
        eprintln!(
            "{} source vertices, 1000 preparations: {rows:?}",
            prepared.meshes[0].model.vertices.len()
        );
    }
}
