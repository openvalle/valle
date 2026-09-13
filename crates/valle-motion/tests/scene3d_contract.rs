use valle_motion::scene3d::{
    AnchorSpec, BudgetUsage, CameraFrameState, Color4, Frame3DState, LightFrameState, LightKind,
    MAX_ABS_POSITION, MAX_ABS_ROTATION_DEGREES, MAX_ANCHORS, MAX_CAMERA_FAR,
    MAX_DIRECTIONAL_LIGHTS, MAX_FRAME_BUFFER_BYTES, MAX_FRAME_SCALARS, MAX_LAYER_EDGE,
    MAX_LAYER_PIXELS, MAX_MATERIALS, MAX_MESHES, MAX_MODEL_BYTES, MAX_SCALE, MAX_TEXTURE_PIXELS,
    MAX_TEXTURE_STORAGE_BYTES, MAX_TEXTURES, MAX_TRIANGLES, MAX_VERTICES, MaterialFrameState,
    MaterialKind, MaterialSpec, MaterialTexture, MaterialTextureSlot, MeshFrameState, MeshSpec,
    MipmapFilter, Scene3DSpec, TextureFilter, TextureWrap, Transform3D, Vec3,
};

fn spec() -> Scene3DSpec {
    Scene3DSpec {
        pbr: Default::default(),
        meshes: vec![MeshSpec {
            key: "product".into(),
            model_control: "productModel".into(),
            material: MaterialSpec {
                kind: Some(MaterialKind::Lambert),
                textures: [(
                    MaterialTextureSlot::BaseColor,
                    Some(MaterialTexture {
                        control: "productTexture".into(),
                        wrap_u: TextureWrap::Clamp,
                        wrap_v: TextureWrap::Clamp,
                        min_filter: TextureFilter::Nearest,
                        mag_filter: TextureFilter::Nearest,
                        mipmap: MipmapFilter::None,
                    }),
                )]
                .into(),
                ..MaterialSpec::default()
            },
            material_overrides: Vec::new(),
            node_ids: Vec::new(),
        }],
        lights: vec![LightKind::Ambient, LightKind::Directional],
        anchors: vec![AnchorSpec {
            key: "feature-anchor".into(),
            parent: "product".into(),
            position: Vec3::new(0.82, 0.95, 0.28),
        }],
    }
}

fn frame() -> Frame3DState {
    Frame3DState {
        exposure: 1.0,
        environment_intensity: 1.0,
        environment_rotation_degrees: 0.0,
        camera: CameraFrameState {
            position: Vec3::new(0.0, 1.2, 4.5),
            target: Vec3::new(0.0, 0.4, 0.0),
            fov_y_degrees: 38.0,
            near: 0.1,
            far: 100.0,
        }
        .with_orbit(26.0, 0.0, Some(4.5))
        .unwrap(),
        meshes: vec![MeshFrameState {
            key: "product".into(),
            material: MaterialFrameState {
                color: Some(Color4([0.8, 0.95, 1.0, 1.0])),
                ..MaterialFrameState::default()
            },
            material_overrides: Vec::new(),
            transform: Transform3D {
                rotation_degrees: Vec3::new(0.0, 10.92, 0.0),
                ..Transform3D::default()
            },
            nodes: Vec::new(),
        }],
        lights: vec![
            LightFrameState::Ambient {
                color: Color4([1.0; 4]),
                intensity: 0.18,
            },
            LightFrameState::Directional {
                color: Color4([1.0; 4]),
                direction: Vec3::new(-0.7, 0.9, 0.55),
                intensity: 1.0,
            },
        ],
    }
}

#[test]
fn scene_contract_wire_is_exact_strict_and_wasm_clean() {
    let scene = spec();
    assert_eq!(scene.frame_scalar_count(), 48);
    scene.validate().unwrap();
    let json = serde_json::to_value(&scene).unwrap();
    assert_eq!(
        json["lights"],
        serde_json::json!(["ambient", "directional"])
    );
    assert!(json.get("camera").is_none());
    assert_eq!(
        serde_json::from_value::<Scene3DSpec>(json.clone()).unwrap(),
        scene
    );
    let mut unknown = json;
    unknown["mutableRuntime"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<Scene3DSpec>(unknown)
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
    changed.lights.push(LightKind::Ambient);
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
}

#[test]
fn frame_contract_carries_complete_camera_lights_and_static_mesh_order() {
    let scene = spec();
    frame()
        .validate_for(&scene)
        .expect("valid frame scalar table");

    let mut changed = frame();
    changed.meshes[0].key = "other".into();
    changed.meshes[0].transform.scale.0[1] = 0.0;
    changed.camera.position = changed.camera.target;
    changed.lights.clear();
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
            .any(|error| error.message.contains("differ from position"))
    );
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("light kinds"))
    );
}

#[test]
fn camera_projection_orbit_and_light_values_validate_the_complete_frame() {
    let scene = spec();
    let camera = frame().camera;
    assert!(camera.with_orbit(0.0, 0.0, None).is_ok());
    for (yaw, pitch, distance) in [
        (f32::NAN, 0.0, None),
        (0.0, 90.0, None),
        (0.0, 0.0, Some(-1.0)),
    ] {
        assert!(camera.with_orbit(yaw, pitch, distance).is_err());
    }
    let mut pole = camera;
    pole.position = Vec3::new(0.0, 1.0, 1.0);
    pole.target = Vec3::ZERO;
    assert!(
        pole.with_orbit(0.0, -45.0, None).is_err(),
        "validate the final orientation, not just the pitch input"
    );
    for bad in 0..6 {
        let mut state = frame();
        match bad {
            0 => state.camera.near = state.camera.far,
            1 => state.camera.target.0[0] = f32::NAN,
            2 => state.exposure = 17.0,
            3 => {
                state.lights[0] = LightFrameState::Ambient {
                    color: Color4([1.0, 0.0, 0.0, 0.5]),
                    intensity: 1.0,
                }
            }
            4 => {
                state.lights[1] = LightFrameState::Directional {
                    color: Color4([1.0; 4]),
                    direction: Vec3::ZERO,
                    intensity: 1.0,
                }
            }
            _ => state.lights.swap(0, 1),
        }
        assert!(
            state.validate_for(&scene).is_err(),
            "invalid frame case {bad}"
        );
    }
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
fn constants_lock_the_bounded_pbr_product_budget() {
    assert_eq!(MAX_LAYER_EDGE, 2_048);
    assert_eq!(MAX_LAYER_PIXELS, 2_000_000);
    assert_eq!(MAX_FRAME_BUFFER_BYTES, 20_000_000);
    assert_eq!(MAX_MODEL_BYTES, 16_777_216);
    assert_eq!(MAX_VERTICES, 65_535);
    assert_eq!(MAX_TRIANGLES, 20_000);
    assert_eq!(MAX_MESHES, 8);
    assert_eq!(MAX_MATERIALS, 8);
    assert_eq!(MAX_TEXTURES, 8);
    assert_eq!(MAX_TEXTURE_PIXELS, 25_165_824);
    assert_eq!(MAX_TEXTURE_STORAGE_BYTES, 134_217_728);
    assert_eq!(MAX_ANCHORS, 32);
    assert_eq!(MAX_DIRECTIONAL_LIGHTS, 2);
    assert_eq!(MAX_FRAME_SCALARS, 19_557);
    assert_eq!(MAX_ABS_POSITION, 10_000.0);
    assert_eq!(MAX_SCALE, 1_000.0);
    assert_eq!(MAX_ABS_ROTATION_DEGREES, 1_000_000.0);
    assert_eq!(MAX_CAMERA_FAR, 100_000.0);
}

#[test]
fn node_bindings_have_stable_distinct_ids_and_complete_bounded_frame_transforms() {
    use valle_motion::scene3d::NodeFrameState;
    let mut scene = spec();
    scene.meshes[0].node_ids = vec![2, 0];
    let mut state = frame();
    state.meshes[0].nodes = [2, 0]
        .map(|id| NodeFrameState {
            id,
            transform: Transform3D::default(),
        })
        .to_vec();
    state.meshes[0].nodes[0].transform.scale.0[0] = -2.0;
    state.validate_for(&scene).unwrap();
    assert_eq!(scene.frame_scalar_count(), 66);
    state.meshes[0].nodes.swap(0, 1);
    assert!(state.validate_for(&scene).is_err());
    state.meshes[0].nodes.swap(0, 1);
    for invalid in [0.0, f32::NAN, 0.0000001, -1001.0] {
        state.meshes[0].nodes[0].transform.scale.0[0] = invalid;
        assert!(state.validate_for(&scene).is_err());
    }
    for ids in [vec![2, 2], vec![256]] {
        scene.meshes[0].node_ids = ids;
        assert!(scene.validate().is_err());
    }
}
