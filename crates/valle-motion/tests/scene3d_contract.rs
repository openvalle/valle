use valle_motion::scene3d::{
    AnchorSpec, BudgetUsage, CameraFrameState, CameraSpec, Color4, Frame3DState, LightSpec,
    MAX_ABS_POSITION, MAX_ABS_ROTATION_DEGREES, MAX_ANCHORS, MAX_CAMERA_FAR,
    MAX_DIRECTIONAL_LIGHTS, MAX_FRAME_BUFFER_BYTES, MAX_FRAME_SCALARS, MAX_LAYER_EDGE,
    MAX_LAYER_PIXELS, MAX_MATERIALS, MAX_MESHES, MAX_MODEL_BYTES, MAX_SCALE, MAX_TEXTURE_PIXELS,
    MAX_TEXTURES, MAX_TRIANGLES, MAX_VERTICES, MaterialKind, MaterialSpec, MeshFrameState,
    MeshSpec, Scene3DSpec, Transform3D, Vec3,
};

const GOLDEN: &str = include_str!("golden/scene3d-spec.json");

fn spec() -> Scene3DSpec {
    Scene3DSpec {
        camera: CameraSpec {
            position: Vec3::new(0.0, 1.2, 4.5),
            target: Vec3::new(0.0, 0.4, 0.0),
            fov_y_degrees: 38.0,
            near: 0.1,
            far: 100.0,
        },
        meshes: vec![MeshSpec {
            key: "product".into(),
            model_control: "productModel".into(),
            material: MaterialSpec {
                kind: MaterialKind::Lambert,
                color: Color4([82.0 / 255.0, 220.0 / 255.0, 1.0, 0.96]),
                texture_control: Some("productTexture".into()),
            },
            transform: Transform3D::default(),
        }],
        lights: vec![
            LightSpec::Ambient { intensity: 0.18 },
            LightSpec::Directional {
                direction: Vec3::new(-0.7, 0.9, 0.55),
                intensity: 1.0,
            },
        ],
        anchors: vec![AnchorSpec {
            key: "feature-anchor".into(),
            parent: "product".into(),
            position: Vec3::new(0.82, 0.95, 0.28),
        }],
    }
}

fn frame() -> Frame3DState {
    Frame3DState {
        camera: CameraFrameState {
            orbit_yaw_degrees: 26.0,
            orbit_pitch_degrees: 0.0,
            distance: 4.5,
            fov_y_degrees: 38.0,
        },
        meshes: vec![MeshFrameState {
            key: "product".into(),
            translation_x: 0.0,
            translation_y: 0.0,
            translation_z: 0.0,
            rotation_x_degrees: 0.0,
            rotation_y_degrees: 10.92,
            rotation_z_degrees: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            scale_z: 1.0,
        }],
        light_intensities: vec![0.18, 1.0],
    }
}

#[test]
fn scene_contract_wire_is_exact_strict_and_wasm_clean() {
    let scene = spec();
    assert_eq!(scene.frame_scalar_count(), 15);
    scene.validate().expect("valid fixed Scene3D contract");
    let json = serde_json::to_string_pretty(&scene).unwrap() + "\n";
    assert_eq!(json, GOLDEN);
    let decoded: Scene3DSpec = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(decoded, scene);

    let unknown = GOLDEN.replacen(
        "\"camera\": {",
        "\"mutableRuntime\": true,\n  \"camera\": {",
        1,
    );
    assert!(
        serde_json::from_str::<Scene3DSpec>(&unknown)
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );
}

#[test]
fn fixed_topology_keys_lights_materials_and_anchors_fail_closed() {
    let mut changed = spec();
    changed.meshes.push(changed.meshes[0].clone());
    changed.meshes[1].key = "product".into();
    changed.anchors[0].parent = "missing".into();
    changed.lights.push(LightSpec::Ambient { intensity: 0.2 });
    changed.lights.push(LightSpec::Directional {
        direction: Vec3::new(0.0, 0.0, 0.0),
        intensity: 8.0,
    });
    let errors = changed.validate().unwrap_err();
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("unique"))
    );
    assert!(errors.0.iter().any(|error| error.path.ends_with("/parent")));
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("ambient"))
    );
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("non-zero"))
    );
    assert!(errors.0.iter().any(|error| error.message.contains("0..=4")));
}

#[test]
fn frame_contract_has_only_explicit_scalars_and_static_mesh_order() {
    let scene = spec();
    frame()
        .validate_for(&scene)
        .expect("valid frame scalar table");

    let mut changed = frame();
    changed.meshes[0].key = "other".into();
    changed.meshes[0].scale_y = 0.0;
    changed.camera.orbit_pitch_degrees = 90.0;
    changed.light_intensities.clear();
    let errors = changed.validate_for(&scene).unwrap_err();
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("static order"))
    );
    assert!(errors.0.iter().any(|error| error.message.contains("scale")));
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("orbit pitch"))
    );
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("all Scene3D lights"))
    );
}

#[test]
fn one_aggregate_budget_unit_is_shared_by_all_three_admission_layers() {
    let exact = BudgetUsage {
        width: 1_250,
        height: 1_600,
        model_bytes: MAX_MODEL_BYTES,
        vertices: MAX_VERTICES,
        triangles: MAX_TRIANGLES,
        meshes: MAX_MESHES as u32,
        materials: MAX_MATERIALS as u32,
        textures: MAX_TEXTURES,
        texture_pixels: MAX_TEXTURE_PIXELS,
        anchors: MAX_ANCHORS as u32,
        frame_scalars: MAX_FRAME_SCALARS as u32,
    };
    assert_eq!(exact.layer_pixels(), Some(MAX_LAYER_PIXELS));
    assert_eq!(exact.frame_buffer_bytes(), Some(MAX_FRAME_BUFFER_BYTES));
    exact.validate().expect("every exact maximum is admitted");

    for changed in [
        BudgetUsage {
            width: MAX_LAYER_EDGE + 1,
            ..exact
        },
        BudgetUsage {
            width: 1_401,
            height: 1_429,
            ..exact
        },
        BudgetUsage {
            model_bytes: MAX_MODEL_BYTES + 1,
            ..exact
        },
        BudgetUsage {
            vertices: MAX_VERTICES + 1,
            ..exact
        },
        BudgetUsage {
            triangles: MAX_TRIANGLES + 1,
            ..exact
        },
        BudgetUsage {
            meshes: MAX_MESHES as u32 + 1,
            ..exact
        },
        BudgetUsage {
            materials: MAX_MATERIALS as u32 + 1,
            ..exact
        },
        BudgetUsage {
            textures: MAX_TEXTURES + 1,
            ..exact
        },
        BudgetUsage {
            texture_pixels: MAX_TEXTURE_PIXELS + 1,
            ..exact
        },
        BudgetUsage {
            anchors: MAX_ANCHORS as u32 + 1,
            ..exact
        },
        BudgetUsage {
            frame_scalars: MAX_FRAME_SCALARS as u32 + 1,
            ..exact
        },
    ] {
        assert!(
            changed.validate().is_err(),
            "over-budget usage was admitted: {changed:?}"
        );
    }
}

#[test]
fn object_addresses_are_two_keys_not_an_ambiguous_flat_string() {
    assert_eq!(
        Scene3DSpec::semantic_address("product-stage", "product").unwrap(),
        "product-stage::product"
    );
    assert!(Scene3DSpec::semantic_address("bad::scene", "product").is_err());
    assert!(Scene3DSpec::semantic_address("scene", "bad::object").is_err());
}

#[test]
fn constants_lock_the_first_product_budget() {
    assert_eq!(MAX_LAYER_EDGE, 2_048);
    assert_eq!(MAX_LAYER_PIXELS, 2_000_000);
    assert_eq!(MAX_FRAME_BUFFER_BYTES, 16_000_000);
    assert_eq!(MAX_MODEL_BYTES, 16_777_216);
    assert_eq!(MAX_VERTICES, 65_535);
    assert_eq!(MAX_TRIANGLES, 20_000);
    assert_eq!(MAX_MESHES, 8);
    assert_eq!(MAX_MATERIALS, 8);
    assert_eq!(MAX_TEXTURES, 4);
    assert_eq!(MAX_TEXTURE_PIXELS, 4_194_304);
    assert_eq!(MAX_ANCHORS, 32);
    assert_eq!(MAX_DIRECTIONAL_LIGHTS, 2);
    assert_eq!(MAX_FRAME_SCALARS, 79);
    assert_eq!(MAX_ABS_POSITION, 10_000.0);
    assert_eq!(MAX_SCALE, 1_000.0);
    assert_eq!(MAX_ABS_ROTATION_DEGREES, 1_000_000.0);
    assert_eq!(MAX_CAMERA_FAR, 100_000.0);
}
