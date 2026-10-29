use super::*;

#[test]
#[ignore = "requires real GPU; verifies LTC rectangles, physical coating and both geometry paths"]
fn rectangle_area_lights_match_cpu_materials() {
    let (mut rasterizer, mut context) = renderer();
    for kind in [MaterialKind::Lambert, MaterialKind::Pbr] {
        let retained = kind == MaterialKind::Lambert;
        let (mut scene, mut frame, resources) = fixture_spec(false, false, false);
        scene.meshes[0].material.kind = Some(kind);
        if retained {
            scene.meshes[0].material.textures = [
                MaterialTextureSlot::BaseColor,
                MaterialTextureSlot::MetallicRoughness,
                MaterialTextureSlot::Normal,
                MaterialTextureSlot::Occlusion,
                MaterialTextureSlot::Emissive,
            ]
            .into_iter()
            .map(|slot| (slot, None))
            .collect();
        }
        scene.lights = vec![LightKind::RectArea];
        frame.lights = vec![LightFrameState::RectArea {
            color: Color4([1.0, 0.7, 0.4, 1.0]),
            intensity: 4.0,
            position: Vec3::new(1.0, 1.0, 3.0),
            target: Vec3::ZERO,
            width: 2.0,
            height: 1.0,
            roll: 0.0,
        }];
        frame.meshes[0].material.clearcoat = Some(0.7);
        frame.meshes[0].material.clearcoat_roughness = Some(0.1);
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        let mut seen = BTreeMap::new();
        for sample in [0, 1, 2, 3, 1, 0] {
            frame.camera.orthographic_height = if sample % 2 == 0 { None } else { Some(3.0) };
            frame.meshes[0].material.roughness = Some(0.1 + sample as f32 * 0.2);
            if let LightFrameState::RectArea {
                position,
                width,
                roll,
                ..
            } = &mut frame.lights[0]
            {
                position.0[0] = sample as f32 * 0.3;
                *width = 1.0 + sample as f32 * 0.3;
                *roll = sample as f32 * 20.0;
            }
            let reference = render_scene(&prepared, &frame).unwrap();
            let geometry = if retained {
                prepare_gpu_mesh_geometry(&prepared, &frame)
            } else {
                prepare_gpu_geometry(&prepared, &frame)
            }
            .unwrap()
            .unwrap();
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
            eprintln!("area {kind:?} sample={sample} max_f16={max}");
            assert!(max <= 0.002);
            if let Some(previous) = seen.insert(sample, actual.clone()) {
                assert_eq!(previous, actual);
            }
        }
        assert_ne!(seen[&0], seen[&2]);
    }
}
