use super::*;

#[test]
#[ignore = "requires real GPU; checks exact far-plane coverage and coplanar tie order"]
fn gpu_mesh_clip_planes_masks_and_coplanar_order() {
    let (mut rasterizer, mut context) = renderer();
    let (mut scene, mut frame, resources) = fixture_spec(false, false, false);
    scene.meshes[0].material.kind = Some(MaterialKind::Unlit);
    scene.meshes[0].material.alpha_mode = Some(AlphaMode::Mask);
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
    let mut second = scene.meshes[0].clone();
    second.key = "second".into();
    scene.meshes.push(second);
    frame.meshes[0].material = MaterialFrameState {
        color: Some(Color4([0.8, 0.1, 0.05, 1.0])),
        emissive_intensity: Some(0.0),
        alpha_cutoff: Some(0.5),
        ..Default::default()
    };
    let mut second = frame.meshes[0].clone();
    second.key = "second".into();
    second.material.color = Some(Color4([0.05, 0.1, 0.8, 1.0]));
    frame.meshes.push(second);
    let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
    for height in [None, Some(3.0)] {
        frame.camera.orthographic_height = height;
        for (near, far) in [(0.1, 100.0), (0.1, 3.0), (3.0, 100.0), (0.1, 2.9)] {
            frame.camera.near = near;
            frame.camera.far = far;
            for alpha in [0.4, 0.5, 1.0] {
                frame.meshes[0].material.color.as_mut().unwrap().0[3] = alpha;
                let reference = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
                let actual = prepare_gpu_mesh_geometry(&prepared, &frame)
                    .unwrap()
                    .unwrap();
                let mut outputs = Vec::new();
                for geometry in [reference, actual] {
                    let input =
                        Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone()))
                            .unwrap();
                    let result = rasterizer.render(&input, &mut context).unwrap().unwrap();
                    // Constant projected Unlit is already working Rec.2020; shaded packets
                    // carry sRGB. Compare after conversion to the same tagged color space.
                    let info = ImageInfo::new(
                        (384, 256),
                        ColorType::RGBAF16,
                        AlphaType::Premul,
                        ColorSpace::new_srgb(),
                    );
                    let mut pixels = vec![0; info.compute_byte_size(info.min_row_bytes())];
                    assert!(result.image.read_pixels_with_context(
                        &mut context,
                        &info,
                        &mut pixels,
                        info.min_row_bytes(),
                        (0, 0),
                        skia_safe::image::CachingHint::Disallow
                    ));
                    outputs.push(pixels);
                }
                let mut visible = 0;
                for (a, b) in outputs[1]
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .zip(outputs[0].as_chunks::<8>().0)
                {
                    assert_eq!(
                        a[6..],
                        b[6..],
                        "near={near} far={far} alpha={alpha} coverage"
                    );
                    visible += usize::from(b[6..] != [0, 0]);
                    for channel in 0..3 {
                        let value = |p: &[u8; 8]| {
                            half(u16::from_le_bytes(
                                p[channel * 2..channel * 2 + 2].try_into().unwrap(),
                            ))
                        };
                        assert!(
                            (value(a) - value(b)).abs() <= 1.0 / 512.0,
                            "near={near} far={far} alpha={alpha} color/order channel={channel}: {} vs {}",
                            value(a),
                            value(b)
                        );
                    }
                }
                assert_eq!(visible > 100, far >= 3.0);
            }
        }
    }
}

#[test]
#[ignore = "requires real GPU; compares vertex deformation, clipping and retained uploads"]
fn gpu_mesh_transforms_morph_skin_and_clipping() {
    let (mut rasterizer, mut context) = renderer();
    for (name, bytes, camera_z, times) in [
        (
            "rigid",
            include_bytes!(
                "../../../../../../../valle-motion/tests/fixtures/scene3d/animated-morph-cube.glb"
            )
            .as_slice(),
            5.0,
            [0.0, 0.0, 0.0],
        ),
        (
            "morph",
            include_bytes!(
                "../../../../../../../valle-motion/tests/fixtures/scene3d/animated-morph-cube.glb"
            )
            .as_slice(),
            5.0,
            [0.0, 1.5, 3.5],
        ),
        (
            "quantized",
            include_bytes!(
                "../../../../../../../valle-motion/tests/fixtures/scene3d/quantized-sparse.glb"
            )
            .as_slice(),
            5.0,
            [0.0, 1.0, 2.0],
        ),
        (
            "skin",
            include_bytes!(
                "../../../../../../../valle-motion/tests/fixtures/scene3d/rigged-simple.glb"
            )
            .as_slice(),
            15.0,
            [0.0, 1.0, 2.083333],
        ),
    ] {
        let (mut scene, mut frame, mut resources) = fixture_spec(false, false, false);
        scene.meshes[0].animation_clip = (name != "rigid").then_some(0);
        resources
            .models
            .insert("model".into(), Arc::new(admit_glb(bytes).unwrap()));
        frame.camera.position = Vec3::new(0.0, 0.0, camera_z);
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        let mut seen = BTreeMap::new();
        let mut source = None;
        for sample in [0, 1, 2, 3, 4, 5, 6, 7, 8, 2, 0] {
            frame.meshes[0].animation_time =
                scene.meshes[0].animation_clip.map(|_| times[sample % 3]);
            frame.meshes[0].transform.rotation_degrees =
                Vec3::new(13.0, sample as f32 * 13.0, -9.0);
            frame.meshes[0].transform.scale =
                Vec3::new(if sample == 4 { -0.9 } else { 0.9 }, 1.1, 0.8);
            frame.meshes[0].parents = if sample == 0 {
                vec![]
            } else {
                vec![
                    Transform3D {
                        translation: Vec3::new(0.2, 0.1, 0.0),
                        rotation_degrees: Vec3::new(7.0, 11.0, 19.0),
                        scale: Vec3::new(if sample == 4 { -0.8 } else { 0.8 }, 1.1, 0.9),
                    },
                    Transform3D {
                        rotation_degrees: Vec3::new(3.0, sample as f32 * 2.0, -7.0),
                        ..Default::default()
                    },
                ]
            };
            frame.camera.near = if sample == 6 { camera_z } else { 0.1 };
            frame.camera.far = if sample == 7 { camera_z } else { 100.0 };
            if sample == 8 {
                frame.camera.position = Vec3::new(0.0, 0.0, 0.1);
            } else {
                frame.camera.position = Vec3::new(0.0, 0.0, camera_z);
            }
            let geometry = prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .expect(name);
            assert!(geometry.vertices.is_empty());
            let mesh = geometry.mesh.as_ref().unwrap();
            if let Some(previous) = &source {
                assert!(Arc::ptr_eq(previous, mesh));
            }
            let cold = rasterizer.mesh.as_ref().is_none_or(|m| m.key != mesh.key);
            source = Some(mesh.clone());
            let reference = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            let expected_input =
                Scene3dGpuInput::new(reference, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let expected = rasterizer
                .render(&expected_input, &mut context)
                .unwrap()
                .unwrap();
            let expected = read_color(&mut context, &expected.image, 384, 256, ColorType::RGBAF16);
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            assert_eq!(
                output.upload_bytes,
                input.upload_bytes().unwrap()
                    + if cold {
                        source.as_ref().unwrap().bytes().len() as u64
                    } else {
                        0
                    }
            );
            let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let a = actual.as_chunks::<8>().0;
            let b = expected.as_chunks::<8>().0;
            let mut maximum = 0.0f32;
            let mut edge_pixels = 0;
            for y in 0usize..256 {
                for x in 0usize..384 {
                    let i = y * 384 + x;
                    if a[i][6..8] != b[i][6..8] {
                        // Existing edge=1 contract: a coverage change must touch a reference edge.
                        let alpha = &a[i][6..8];
                        assert!(
                            (y.saturating_sub(1)..=(y + 1).min(255))
                                .any(|ny| (x.saturating_sub(1)..=(x + 1).min(383))
                                    .any(|nx| &b[ny * 384 + nx][6..8] == alpha)),
                            "{name} sample={sample} coverage outside edge at {x},{y}"
                        );
                        edge_pixels += 1;
                        continue;
                    }
                    for channel in 0..3 {
                        let read = |p: &[u8; 8]| {
                            half(u16::from_le_bytes(
                                p[channel * 2..channel * 2 + 2].try_into().unwrap(),
                            ))
                        };
                        maximum = maximum.max((read(&a[i]) - read(&b[i])).abs());
                    }
                }
            }
            eprintln!(
                "{name} sample={sample} max_f16={maximum} edge_pixels={edge_pixels} upload={}",
                output.upload_bytes
            );
            assert!(
                maximum <= 1.0 / 512.0,
                "{name} sample={sample} F16 error {maximum}"
            );
            if let Some(old) = seen.insert(sample, actual.clone()) {
                assert_eq!(old, actual, "random seek");
            }
        }
    }
}
