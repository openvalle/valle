use super::*;
use valle_draw::PathVerb;
use valle_engine::motion::geometry::PathData;

#[test]
#[ignore = "requires real GPU; compares rounded geometry and smooth normals on both paths"]
fn rounded_extrusion_matches_cpu_on_both_geometry_paths() {
    let (mut rasterizer, mut context) = renderer();
    for segments in [1, 4, 12] {
        let (mut scene, mut frame, resources) = fixture_spec(false, false, false);
        scene.meshes[0].model_control = None;
        scene.meshes[0].geometry = Some(ProceduralGeometry::Extrude {
            path: PathData::new(
                vec![
                    PathVerb::Move,
                    PathVerb::Line,
                    PathVerb::Line,
                    PathVerb::Line,
                    PathVerb::Close,
                ],
                [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                    .map(|p| valle_draw::Point::new(p[0], p[1]))
                    .to_vec(),
            )
            .unwrap(),
            depth: 0.8,
            bevel: 0.25,
            bevel_segments: segments,
        });
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
        frame.meshes[0].transform.rotation_degrees = Vec3::new(15.0, 25.0, 5.0);
        frame.meshes[0].material.clearcoat = Some(0.6);
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        let reference = render_scene(&prepared, &frame).unwrap();
        for retained in [false, true] {
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
            eprintln!("bevel segments={segments} retained={retained} max_f16={max}");
            assert!(max <= 0.002);
        }
    }
}
