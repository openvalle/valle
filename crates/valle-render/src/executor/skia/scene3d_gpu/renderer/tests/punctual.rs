use super::*;

#[test]
#[ignore = "requires real GPU; checks animated point/spot lights on all materials and projections"]
fn punctual_lights_match_cpu_materials_and_random_seek() {
    let (mut rasterizer, mut context) = renderer();
    for kind in [
        MaterialKind::Unlit,
        MaterialKind::Lambert,
        MaterialKind::Pbr,
    ] {
        let (mut scene, mut frame, resources) = fixture_spec(true, false, false);
        scene.meshes[0].material.kind = Some(kind);
        scene.lights.extend([LightKind::Point, LightKind::Spot]);
        frame.lights.extend([
            LightFrameState::Point {
                color: Color4([1.0, 0.4, 0.2, 1.0]),
                intensity: 4.0,
                position: Vec3::new(2.0, 1.0, 2.0),
                range: 6.0,
            },
            LightFrameState::Spot {
                color: Color4([0.2, 0.4, 1.0, 1.0]),
                intensity: 8.0,
                position: Vec3::new(-2.0, 1.0, 2.0),
                target: Vec3::ZERO,
                range: 8.0,
                angle: 35.0,
                penumbra: 0.4,
            },
        ]);
        frame.meshes[0].material.clearcoat = Some(0.6);
        frame.meshes[0].material.clearcoat_roughness = Some(0.2);
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        let mut seen = BTreeMap::new();
        for sample in [0, 1, 2, 3, 1, 0] {
            frame.camera.orthographic_height = if sample % 2 == 0 { None } else { Some(3.0) };
            if let LightFrameState::Point {
                position, range, ..
            } = &mut frame.lights[scene.lights.len() - 2]
            {
                position.0[0] = sample as f32 * 0.6;
                *range = if sample == 3 { 0.0 } else { 6.0 };
            }
            if let LightFrameState::Spot {
                angle, penumbra, ..
            } = &mut frame.lights[scene.lights.len() - 1]
            {
                *angle = 20.0 + sample as f32 * 10.0;
                *penumbra = if sample == 2 { 0.0 } else { 0.4 };
            }
            let reference = render_scene(&prepared, &frame).unwrap();
            let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let maximum = actual
                .as_chunks::<2>()
                .0
                .iter()
                .zip(&reference.premul_rgba16f)
                .map(|(a, b)| (half(u16::from_le_bytes(*a)) - half(*b)).abs())
                .fold(0.0_f32, f32::max);
            eprintln!("punctual {kind:?} sample={sample} max_f16={maximum}");
            assert!(maximum <= 0.001);
            if let Some(previous) = seen.insert(sample, actual.clone()) {
                assert_eq!(previous, actual);
            }
        }
        if kind != MaterialKind::Unlit {
            assert_ne!(seen[&0], seen[&2]);
        }
    }
}
