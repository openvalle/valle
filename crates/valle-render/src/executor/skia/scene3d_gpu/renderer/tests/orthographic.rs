use super::*;

#[test]
#[ignore = "requires real GPU; verifies orthographic color, depth and replay on both geometry paths"]
fn orthographic_projection_matches_cpu_depth_and_materials() {
    let (mut rasterizer, mut context) = renderer();
    for retained in [false, true] {
        let (mut scene, mut frame, resources) = fixture_spec(!retained, !retained, false);
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
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        // Request a depth attachment without changing the raster color under comparison.
        frame.camera.depth_of_field = Some(DepthOfFieldState {
            focus_distance: 3.0,
            max_blur_radius: 1.0,
        });
        let mut seen = BTreeMap::new();
        for sample in [0, 1, 2, 1, 0] {
            frame.camera.orthographic_height = Some(3.0 + sample as f32 * 0.4);
            frame.camera.position.0[0] = sample as f32 * 0.15;
            frame.meshes[0].transform.rotation_degrees =
                Vec3::new(0.0, 13.0 + sample as f32 * 7.0, 4.0);
            let reference = render_scene_without_depth_of_field(&prepared, &frame).unwrap();
            assert!(reference.object_ids.iter().filter(|id| **id != 0).count() > 1000);
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
            let pixels = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let depth = read_color(
                &mut context,
                output.depth_image.as_ref().unwrap(),
                384,
                256,
                ColorType::RGBA8888,
            );
            let mut maximum = 0.0_f32;
            let mut edge_pixels = 0;
            for (i, (pixel, expected)) in pixels
                .as_chunks::<8>()
                .0
                .iter()
                .zip(reference.premul_rgba16f.as_chunks::<4>().0)
                .enumerate()
            {
                if half(u16::from_le_bytes([pixel[6], pixel[7]])) != half(expected[3]) {
                    edge_pixels += 1;
                    continue;
                }
                for c in 0..4 {
                    maximum = maximum.max(
                        (half(u16::from_le_bytes([pixel[c * 2], pixel[c * 2 + 1]]))
                            - half(expected[c]))
                        .abs(),
                    );
                }
                let actual = f32::from_le_bytes(depth[i * 4..i * 4 + 4].try_into().unwrap());
                let expected = reference.depth[i];
                if expected == f32::MAX {
                    assert_eq!(actual, expected);
                } else {
                    assert!(
                        (actual - expected).abs() / expected.max(1.0) <= 1e-4,
                        "retained={retained} sample={sample} depth {actual}/{expected}"
                    );
                }
            }
            eprintln!(
                "orthographic retained={retained} sample={sample} max_f16={maximum} edges={edge_pixels}"
            );
            assert!(maximum <= 1.0 / 512.0);
            assert!(edge_pixels <= 4);
            if let Some(previous) = seen.insert(sample, pixels.clone()) {
                assert_eq!(previous, pixels);
            }
        }
    }
}
