use super::*;

fn model(extensions: serde_json::Value, metallic: f32) -> Vec<u8> {
    let bytes = material_glb(
        json!({
            "pbrMetallicRoughness": {"baseColorFactor":[0.1,0.2,0.3,1],
                "metallicFactor":metallic,"roughnessFactor":0.35},
            "extensions":extensions,
        }),
        &[],
    );
    edit_glb(&bytes, |root, _| {
        root["extensionsUsed"] = json!(extensions.as_object().unwrap().keys().collect::<Vec<_>>());
    })
}

#[test]
fn imported_physical_factors_match_authored_materials() {
    let extensions = json!({
        "KHR_materials_ior":{"ior":2.2},
        "KHR_materials_specular":{"specularFactor":0.4},
        "KHR_materials_clearcoat":{"clearcoatFactor":0.7,"clearcoatRoughnessFactor":0.2},
    });
    let a = prepare_scene(&pbr_scene(), 96, 96, &resources(&model(extensions, 0.1))).unwrap();
    let b = prepare_scene(&pbr_scene(), 96, 96, &resources(&model(json!({}), 0.1))).unwrap();
    let mut frame = pbr_frame();
    frame.lights = test_lights(0.2, 2.0);
    let imported = render_scene(&a, &frame).unwrap();
    frame.meshes[0].material = MaterialFrameState {
        ior: Some(2.2),
        specular_intensity: Some(0.4),
        clearcoat: Some(0.7),
        clearcoat_roughness: Some(0.2),
        ..Default::default()
    };
    assert_frame_bits_eq(&imported, &render_scene(&b, &frame).unwrap());
    assert_frame_bits_eq(
        &imported,
        &render_scene_with_workers(&b, &frame, 4).unwrap(),
    );
}

#[test]
fn specular_controls_change_dielectrics_but_not_metals() {
    let scene = pbr_scene();
    for metallic in [0.0, 1.0] {
        let p = prepare_scene(&scene, 64, 64, &resources(&model(json!({}), metallic))).unwrap();
        let mut f = pbr_frame();
        f.lights = test_lights(0.0, 1.0);
        let original = render_scene(&p, &f).unwrap();
        f.meshes[0].material.specular_intensity = Some(0.0);
        f.meshes[0].material.ior = Some(2.4);
        let changed = render_scene(&p, &f).unwrap();
        if metallic == 1.0 {
            assert_frame_bits_eq(&original, &changed);
        } else {
            assert_ne!(original.premul_rgba16f, changed.premul_rgba16f);
        }
        f.meshes[0].material.clearcoat = Some(1.0);
        f.meshes[0].material.clearcoat_roughness = Some(0.1);
        assert_ne!(
            changed.premul_rgba16f,
            render_scene(&p, &f).unwrap().premul_rgba16f
        );
    }
}

#[test]
fn imported_volume_thickness_follows_node_and_host_scale() {
    let mut scene: Scene3DSpec =
        serde_json::from_str(include_str!("../fixtures/scene3d/transmission-scene.json")).unwrap();
    let mut frame: Frame3DState =
        serde_json::from_str(include_str!("../fixtures/scene3d/transmission-frame.json")).unwrap();
    scene.meshes[2].model_control = Some("volume".into());
    let mut r = SceneResources::default();
    for (key, bytes) in [
        (
            "plane",
            include_bytes!("../fixtures/scene3d/plane.glb").as_slice(),
        ),
        (
            "volume",
            include_bytes!("../fixtures/scene3d/volume-plane.glb").as_slice(),
        ),
    ] {
        r.models
            .insert(key.into(), Arc::new(admit_glb(bytes).unwrap()));
    }
    let p = prepare_scene(&scene, 160, 120, &r).unwrap();
    for scale in [1.0, 2.0, -2.0, 0.5] {
        frame.meshes[2].transform.scale = Vec3::new(scale, 1.4, 1.0);
        frame.meshes[2].material.thickness = None;
        let imported = render_scene(&p, &frame).unwrap();
        // A local Z normal rotated 45 degrees then scaled on world X has inverse-transpose
        // length sqrt((0.5/sx^2+0.5)/1.5^2). Rotation after that preserves its length.
        frame.meshes[2].material.thickness = Some(0.4 * 1.5 / (0.5 / (scale * scale) + 0.5).sqrt());
        let explicit = render_scene(&p, &frame).unwrap();
        let max = imported
            .premul_rgba16f
            .iter()
            .zip(&explicit.premul_rgba16f)
            .map(|(&a, &b)| (half_to_f32(a) - half_to_f32(b)).abs())
            .fold(0.0_f32, f32::max);
        assert!(max < 0.002, "scale={scale},max={max}");
        assert_eq!(imported.depth, explicit.depth);
    }
}

#[test]
fn imported_transmission_uses_the_transparent_gpu_path() {
    let bytes = model(
        json!({
            "KHR_materials_transmission":{"transmissionFactor":0.85},
            "KHR_materials_volume":{"thicknessFactor":0.2,"attenuationColor":[0.4,0.6,0.9],"attenuationDistance":2},
        }),
        0.0,
    );
    let model = admit_glb(&bytes).unwrap();
    let m = &model.materials()[0];
    assert_eq!(m.transmission, 0.85);
    assert_eq!(m.thickness, 0.2);
    assert_eq!(m.attenuation_distance, Some(2.0));
    let p = prepare_scene(&pbr_scene(), 64, 64, &resources(&bytes)).unwrap();
    let f = pbr_frame();
    assert!(
        valle_motion::scene3d::prepare_gpu_mesh_geometry(&p, &f)
            .unwrap()
            .is_none()
    );
    assert!(prepare_gpu_geometry(&p, &f).unwrap().is_some());
    let image = render_scene(&p, &f).unwrap();
    assert!(
        image
            .premul_rgba16f
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0 && half_to_f32(p[3]) < 0.5)
    );
}

#[test]
fn physical_extension_validation_rejects_invalid_factors() {
    for extension in [
        json!({"KHR_materials_ior":{"ior":0.5}}),
        json!({"KHR_materials_specular":{"specularFactor":1.1}}),
        json!({"KHR_materials_clearcoat":{"clearcoatFactor":-0.1}}),
        json!({"KHR_materials_clearcoat":{"clearcoatRoughnessFactor":2}}),
        json!({"KHR_materials_volume":{"thicknessFactor":-1}}),
        json!({"KHR_materials_volume":{"attenuationDistance":0}}),
        json!({"KHR_materials_transmission":{"transmissionFactor":2}}),
        json!({"KHR_materials_unlit":{},"KHR_materials_specular":{}}),
    ] {
        assert!(
            admit_glb(&model(extension.clone(), 0.0)).is_err(),
            "{extension}"
        );
    }
}
