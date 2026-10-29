use super::*;
use valle_engine::motion::particles3d::ParticleSystem3D;

#[test]
#[ignore = "requires a real GPU; shared particle/mesh depth, tint, alpha and seeking"]
fn particles_match_cpu_depth_tint_and_random_seek() {
    let (mut rasterizer, mut context) = renderer();
    for textured in [false, true] {
        let (mut scene, mut frame, mut resources) = fixture_spec(false, false, false);
        if !textured {
            scene.meshes[0].material.textures = [
                MaterialTextureSlot::BaseColor,
                MaterialTextureSlot::MetallicRoughness,
                MaterialTextureSlot::Normal,
                MaterialTextureSlot::Occlusion,
                MaterialTextureSlot::Emissive,
            ]
            .into_iter()
            .map(|s| (s, None))
            .collect();
        }
        let mut emitter = scene.meshes[0].clone();
        emitter.key = "particles".into();
        emitter.model_control = None;
        emitter.material = MaterialSpec {
            kind: Some(MaterialKind::Unlit),
            alpha_mode: Some(AlphaMode::Blend),
            ..Default::default()
        };
        if textured {
            emitter.material.kind = Some(MaterialKind::Pbr);
            resources.textures.insert(
                ("sprite".into(), TextureRole::Color),
                Arc::new(resources.models["model"].images()[0].clone()),
            );
            emitter.material.textures.insert(
                MaterialTextureSlot::BaseColor,
                Some(MaterialTexture {
                    control: "sprite".into(),
                    wrap_u: TextureWrap::Clamp,
                    wrap_v: TextureWrap::Clamp,
                    min_filter: TextureFilter::Linear,
                    mag_filter: TextureFilter::Linear,
                    mipmap: MipmapFilter::Linear,
                }),
            );
        }
        emitter.particles = Some(Particles3DSpec {
            motion: ParticleSystem3D {
                seed: 3,
                count: 32,
                min: [-0.8, -0.6, -0.5],
                max: [0.8, 0.6, 0.5],
                velocity: [[-0.1, 0.1], [0.1, 0.3], [-0.1, 0.1]],
                gravity: [0., -0.1, 0.],
                birth_interval: 0.,
                lifetime: 2.,
                drag: 0.3,
                looping: true,
            },
            shape: ParticleShape3D::Circle,
            sizes: [0.3, 0.1],
            colors: [Color4([1., 0.3, 0.1, 1.]), Color4([0.2, 0.4, 1., 1.])],
            opacities: [0.8, 0.2],
            rotations: [0., 90.],
        });
        scene.meshes.push(emitter);
        let mut state = frame.meshes[0].clone();
        state.key = "particles".into();
        state.particle_time = Some(0.);
        state.material = MaterialFrameState::default();
        frame.meshes.push(state);
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        let mut seen = BTreeMap::new();
        for sample in [0, 1, 3, 2, 1, 0] {
            frame.meshes[1].particle_time = Some(sample as f64 * 0.5);
            frame.meshes[1].parents = vec![Transform3D {
                rotation_degrees: Vec3::new(0., sample as f32 * 20., 0.),
                scale: Vec3::new(0.9, 1.1, 1.),
                ..Default::default()
            }];
            frame.camera.orthographic_height = if sample % 2 == 0 { None } else { Some(3.) };
            let reference = render_scene(&prepared, &frame).unwrap();
            let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            assert_eq!(
                geometry.vertex_stride(),
                if textured { 31 * 4 } else { 17 * 4 }
            );
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let max = actual
                .as_chunks::<2>()
                .0
                .iter()
                .zip(&reference.premul_rgba16f)
                .map(|(a, b)| (half(u16::from_le_bytes(*a)) - half(*b)).abs())
                .fold(0.0_f32, f32::max);
            eprintln!("particles textured={textured} sample={sample} max_f16={max}");
            assert!(max <= 0.002);
            if let Some(previous) = seen.insert(sample, actual.clone()) {
                assert_eq!(previous, actual);
            }
        }
        assert_ne!(seen[&0], seen[&2]);
    }
}
