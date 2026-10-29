use super::*;
use crate::{particles3d::ParticleSystem3D, scene3d::*};

fn fixture() -> (Scene3DSpec, Frame3DState) {
    let (p, mut frame) = geometry_reuse_tests::fixture(
        include_bytes!("../../../tests/fixtures/scene3d/animated-morph-cube.glb"),
        false,
    );
    let mut scene = p.scene.clone();
    scene.anchors.clear();
    scene.meshes[0].model_control = None;
    scene.meshes[0].animation_clip = None;
    scene.meshes[0].material = MaterialSpec {
        kind: Some(MaterialKind::Unlit),
        alpha_mode: Some(AlphaMode::Blend),
        ..Default::default()
    };
    scene.meshes[0].particles = Some(Particles3DSpec {
        motion: ParticleSystem3D {
            seed: 1,
            count: 1,
            min: [0.; 3],
            max: [0.; 3],
            velocity: [[0.; 2]; 3],
            gravity: [0.; 3],
            birth_interval: 0.,
            lifetime: 2.,
            drag: 0.,
            looping: false,
        },
        shape: ParticleShape3D::Quad,
        sizes: [1.; 2],
        colors: [Color4([1., 0., 0., 1.]); 2],
        opacities: [1.; 2],
        rotations: [0.; 2],
    });
    frame.camera.position = Vec3::new(0., 0., 5.);
    frame.camera.orthographic_height = Some(4.);
    frame.meshes[0].animation_time = None;
    frame.meshes[0].particle_time = Some(0.);
    frame.meshes[0].material = MaterialFrameState::default();
    (scene, frame)
}
fn prepare(scene: &Scene3DSpec) -> PreparedScene {
    prepare_scene(scene, 128, 96, &SceneResources::default()).unwrap()
}
#[test]
fn particles_share_mesh_depth_and_restore_after_seeking() {
    let (mut scene, mut frame) = fixture();
    let mut wall = scene.meshes[0].clone();
    wall.key = "wall".into();
    wall.particles = None;
    // Use the admitted model fixture as the opaque occluder.
    wall.geometry = None;
    wall.model_control = Some("wall".into());
    wall.material.alpha_mode = Some(AlphaMode::Opaque);
    scene.meshes.push(wall);
    let mut wall_frame = frame.meshes[0].clone();
    wall_frame.key = "wall".into();
    wall_frame.particle_time = None;
    wall_frame.material.color = Some(Color4([0., 0., 1., 1.]));
    frame.meshes.push(wall_frame);
    let resources = SceneResources {
        models: [(
            "wall".into(),
            Arc::new(
                admit_glb(include_bytes!(
                    "../../../tests/fixtures/scene3d/animated-morph-cube.glb"
                ))
                .unwrap(),
            ),
        )]
        .into(),
        ..Default::default()
    };
    let prepared = prepare_scene(&scene, 128, 96, &resources).unwrap();
    let center = (48 * 128 + 64) * 4;
    frame.meshes[0].transform.translation.0[2] = -2.;
    let behind = render_scene(&prepared, &frame).unwrap();
    assert_eq!(behind.premul_rgba16f[center], 0);
    assert!(behind.premul_rgba16f[center + 2] > 0);
    frame.meshes[0].transform.translation.0[2] = 2.;
    let front = render_scene(&prepared, &frame).unwrap();
    assert!(front.premul_rgba16f[center] > 0);
    assert_eq!(front.premul_rgba16f[center + 2], 0);
    frame.meshes[0].particle_time = Some(2.);
    assert_ne!(front, render_scene(&prepared, &frame).unwrap());
    frame.meshes[0].particle_time = Some(0.);
    assert_eq!(
        front,
        render_scene_with_workers(&prepared, &frame, 4).unwrap()
    );
}
#[test]
fn particle_billboards_follow_camera_groups_and_lifetime() {
    let (mut scene, mut frame) = fixture();
    scene.meshes[0].particles.as_mut().unwrap().opacities = [0.8, 0.2];
    let prepared = prepare(&scene);
    let original = render_scene(&prepared, &frame).unwrap();
    assert!(original.premul_rgba16f.iter().any(|v| *v != 0));
    frame.meshes[0].particle_time = Some(1.);
    assert_ne!(original, render_scene(&prepared, &frame).unwrap());
    frame.meshes[0].particle_time = Some(0.);
    frame.camera.position = Vec3::new(5., 0., 0.);
    assert_eq!(original, render_scene(&prepared, &frame).unwrap());
    frame.meshes[0].parents = vec![Transform3D {
        translation: Vec3::new(0., 1., 0.),
        scale: Vec3::new(2., 2., 2.),
        ..Default::default()
    }];
    let shifted = render_scene(&prepared, &frame).unwrap();
    assert_ne!(original.premul_rgba16f, shifted.premul_rgba16f);
    frame.meshes[0].particle_time = Some(-1.);
    assert!(
        render_scene(&prepared, &frame)
            .unwrap()
            .premul_rgba16f
            .iter()
            .all(|v| *v == 0)
    );
}
#[test]
fn particle_budget_and_gpu_tint_packet_are_bounded() {
    let (mut scene, frame) = fixture();
    let prepared = prepare(&scene);
    assert_eq!(prepared.budget().triangles, 2);
    assert!(
        prepare_gpu_mesh_geometry(&prepared, &frame)
            .unwrap()
            .is_none()
    );
    let gpu = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    assert_eq!(gpu.vertex_stride(), 17 * 4);
    assert!(gpu.web_packet(&frame).is_some());
    assert_eq!(
        gpu.shading
            .as_ref()
            .unwrap()
            .particle_tints
            .as_ref()
            .unwrap()
            .len(),
        gpu.vertices.len()
    );
    let spec = scene.meshes[0].particles.as_mut().unwrap();
    spec.motion.count = 8193;
    spec.shape = ParticleShape3D::Circle;
    assert!(scene.validate().is_err());
    let (mut scene, _) = fixture();
    scene.meshes[0].particles.as_mut().unwrap().motion.count = 16384;
    for key in ["two", "three", "four", "five"] {
        let mut emitter = scene.meshes[0].clone();
        emitter.key = key.into();
        scene.meshes.push(emitter);
    }
    assert!(prepare_scene(&scene, 128, 96, &SceneResources::default()).is_err());
}

#[test]
fn particle_transparency_is_independent_of_emitter_order() {
    let (mut scene, mut frame) = fixture();
    scene.meshes[0].particles.as_mut().unwrap().opacities = [0.4; 2];
    let mut other = scene.meshes[0].clone();
    other.key = "other".into();
    other.particles.as_mut().unwrap().colors = [Color4([0., 0., 1., 1.]); 2];
    scene.meshes.push(other);
    let mut state = frame.meshes[0].clone();
    state.key = "other".into();
    state.transform.translation.0[2] = 0.1;
    frame.meshes.push(state);
    let a = render_scene(&prepare(&scene), &frame).unwrap();
    scene.meshes.reverse();
    frame.meshes.reverse();
    let b = render_scene(&prepare(&scene), &frame).unwrap();
    assert_eq!(a.premul_rgba16f, b.premul_rgba16f);
    let pixel = (48 * 128 + 64) * 4;
    assert!(a.premul_rgba16f[pixel] > 0 && a.premul_rgba16f[pixel + 2] > 0);
}
