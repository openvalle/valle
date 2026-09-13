use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use sha2::{Digest, Sha256};
use valle_motion::scene3d::{
    AnchorSpec, CameraFrameState, Color4, Frame3DState, LightFrameState, LightKind,
    MaterialFrameState, MaterialImage, MaterialKind, MaterialSpec, MaterialTexture,
    MaterialTextureSlot, MeshFrameState, MeshSpec, MipmapFilter, Scene3DSpec, SceneResources,
    TextureFilter, TextureRole, TextureWrap, Transform3D, Vec3, admit_glb, prepare_cache_key,
    prepare_scene, project_anchors, render_scene, render_scene_reusing,
};

const RASTER_GOLDEN: &str = include_str!("golden/software-raster.json");

#[derive(Clone)]
struct Triangle {
    positions: [[f32; 3]; 3],
    normals: [[f32; 3]; 3],
    uvs: [[f32; 2]; 3],
    indices: [u16; 3],
}

fn triangle(z: f32, normal: [f32; 3]) -> Triangle {
    Triangle {
        positions: [[-0.9, -0.72, z], [0.9, -0.72, z], [0.0, 0.94, z]],
        normals: [normal; 3],
        uvs: [[0.0, 1.0], [1.0, 1.0], [0.5, 0.0]],
        indices: [0, 1, 2],
    }
}

fn glb(triangles: &[Triangle]) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    for triangle in triangles {
        let position = push_f32_view(&mut bin, &mut views, triangle.positions.iter().flatten());
        let normal = push_f32_view(&mut bin, &mut views, triangle.normals.iter().flatten());
        let uv = push_f32_view(&mut bin, &mut views, triangle.uvs.iter().flatten());
        let indices = push_u16_view(&mut bin, &mut views, &triangle.indices);
        let base = accessors.len();
        accessors.extend([
            json!({"bufferView": position, "componentType": 5126, "count": 3, "type": "VEC3"}),
            json!({"bufferView": normal, "componentType": 5126, "count": 3, "type": "VEC3"}),
            json!({"bufferView": uv, "componentType": 5126, "count": 3, "type": "VEC2"}),
            json!({"bufferView": indices, "componentType": 5123, "count": 3, "type": "SCALAR"}),
        ]);
        primitives.push(json!({
            "attributes": {"POSITION": base, "NORMAL": base + 1, "TEXCOORD_0": base + 2},
            "indices": base + 3,
            "mode": 4
        }));
    }
    let json = json!({
        "asset": {"version": "2.0", "generator": "valle-3d-test"},
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
        "meshes": [{"name": "product", "primitives": primitives}],
        "nodes": [{"name": "product", "mesh": 0}],
        "scenes": [{"name": "default", "nodes": [0]}],
        "scene": 0
    });
    encode_glb(serde_json::to_vec(&json).unwrap(), bin)
}

fn push_f32_view<'a>(
    bin: &mut Vec<u8>,
    views: &mut Vec<serde_json::Value>,
    values: impl Iterator<Item = &'a f32>,
) -> usize {
    align4(bin, 0);
    let offset = bin.len();
    for value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let index = views.len();
    views.push(json!({
        "buffer": 0,
        "byteOffset": offset,
        "byteLength": bin.len() - offset,
        "target": 34962
    }));
    index
}

fn push_u16_view(bin: &mut Vec<u8>, views: &mut Vec<serde_json::Value>, values: &[u16]) -> usize {
    align4(bin, 0);
    let offset = bin.len();
    for value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let index = views.len();
    views.push(json!({
        "buffer": 0,
        "byteOffset": offset,
        "byteLength": bin.len() - offset,
        "target": 34963
    }));
    index
}

fn encode_glb(mut json: Vec<u8>, mut bin: Vec<u8>) -> Vec<u8> {
    align4(&mut json, b' ');
    let declared_bin_length = bin.len();
    align4(&mut bin, 0);
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&0x4654_6c67_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4e4f_534a_u32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x004e_4942_u32.to_le_bytes());
    bytes.extend_from_slice(&bin);
    assert!(bin.len() - declared_bin_length <= 3);
    bytes
}

fn align4(bytes: &mut Vec<u8>, padding: u8) {
    while bytes.len() % 4 != 0 {
        bytes.push(padding);
    }
}

fn scene() -> Scene3DSpec {
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
            key: "label".into(),
            parent: "product".into(),
            position: Vec3::new(0.62, 0.54, 0.5),
        }],
    }
}

fn frame() -> Frame3DState {
    Frame3DState {
        exposure: 1.0,
        environment_intensity: 1.0,
        environment_rotation_degrees: 0.0,
        camera: CameraFrameState {
            position: Vec3::new(0.0, 0.35, 4.5),
            target: Vec3::new(0.0, 0.1, 0.0),
            fov_y_degrees: 38.0,
            near: 0.1,
            far: 20.0,
        }
        .with_orbit(0.0, 0.0, Some(4.5))
        .unwrap(),
        meshes: vec![MeshFrameState {
            key: "product".into(),
            material: MaterialFrameState {
                color: Some(Color4([0.8, 0.95, 1.0, 1.0])),
                ..MaterialFrameState::default()
            },
            material_overrides: Vec::new(),
            transform: Transform3D {
                rotation_degrees: Vec3::new(0.0, 0.0, 0.0),
                ..Transform3D::default()
            },
            nodes: Vec::new(),
        }],
        lights: test_lights(0.12, 0.88),
    }
}

fn test_lights(ambient: f32, directional: f32) -> Vec<LightFrameState> {
    vec![
        LightFrameState::Ambient {
            color: Color4([1.0; 4]),
            intensity: ambient,
        },
        LightFrameState::Directional {
            color: Color4([1.0; 4]),
            direction: Vec3::new(0.0, 0.0, 1.0),
            intensity: directional,
        },
    ]
}

fn texture_image(width: u32, height: u32, rgba: &[u8], role: TextureRole) -> MaterialImage {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    MaterialImage::from_encoded(&png, role).unwrap()
}

fn resources(model_bytes: &[u8]) -> SceneResources {
    let texture_bytes = vec![
        255, 80, 30, 255, 40, 180, 255, 255, 24, 24, 24, 64, 150, 150, 150, 180,
    ];
    SceneResources {
        environments: BTreeMap::new(),
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(model_bytes).unwrap()),
        )]),
        textures: BTreeMap::from([(
            ("productTexture".into(), TextureRole::Color),
            Arc::new(texture_image(2, 2, &texture_bytes, TextureRole::Color)),
        )]),
    }
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn u16_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[test]
fn exact_glb_subset_admits_to_content_addressed_geometry() {
    let bytes = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let model = admit_glb(&bytes).expect("narrow GLB subset");
    assert_eq!(model.source_bytes(), bytes.len() as u64);
    assert_eq!(model.content_digest().as_hex(), hash(&bytes));
    assert_eq!(model.vertex_count(), 3);
    assert_eq!(model.indices(), [0, 1, 2]);
    assert_eq!(model.triangle_count(), 1);
    assert_eq!(model.primitives().len(), 1);
}

#[test]
fn external_uri_extensions_unsupported_materials_and_bad_indices_fail_closed() {
    let valid = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    for (needle, replacement, expected) in [
        (
            "\"byteLength\":102",
            "\"byteLength\":102,\"uri\":\"https://x/model.bin\"",
            "no URI",
        ),
        (
            "\"asset\":{",
            "\"extensionsUsed\":[\"KHR_draco_mesh_compression\"],\"asset\":{",
            "extensions",
        ),
        (
            "\"asset\":{",
            "\"materials\":[{\"alphaMode\":\"BLEND\"}],\"asset\":{",
            "OPAQUE",
        ),
    ] {
        let changed = rewrite_json_chunk(&valid, needle, replacement);
        let error = admit_glb(&changed).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    let mut bad_index = triangle(0.0, [0.0, 0.0, 1.0]);
    bad_index.indices[2] = 9;
    assert!(
        admit_glb(&glb(&[bad_index]))
            .unwrap_err()
            .to_string()
            .contains("outside this primitive")
    );
}

/// Enforce aggregate model budgets incrementally. A later NaN primitive must never be decoded after the vertex budget is exceeded.
#[test]
fn aggregate_vertex_budget_stops_decoding_at_the_offending_primitive() {
    const CHUNK: usize = 30_000; // One primitive is valid, but three exceed MAX_VERTICES=65535.
    let mut bin = Vec::new();
    let mut views = Vec::new();

    let positions = vec![[0.1f32, 0.2, 0.3]; CHUNK];
    let normals = vec![[0.0f32, 0.0, 1.0]; CHUNK];
    let uvs = vec![[0.5f32, 0.5]; CHUNK];
    let position = push_f32_view(&mut bin, &mut views, positions.iter().flatten());
    let normal = push_f32_view(&mut bin, &mut views, normals.iter().flatten());
    let uv = push_f32_view(&mut bin, &mut views, uvs.iter().flatten());
    let index_view = push_u16_view(&mut bin, &mut views, &[0u16, 1, 2]);

    let poison_positions = [f32::NAN, 0.0, 0.0, f32::NAN, 0.0, 0.0, f32::NAN, 0.0, 0.0];
    let poison_normals = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let poison_uvs = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0];
    let poison_position = push_f32_view(&mut bin, &mut views, poison_positions.iter());
    let poison_normal = push_f32_view(&mut bin, &mut views, poison_normals.iter());
    let poison_uv = push_f32_view(&mut bin, &mut views, poison_uvs.iter());

    let accessors = json!([
        {"bufferView": position, "componentType": 5126, "count": CHUNK, "type": "VEC3"},
        {"bufferView": normal, "componentType": 5126, "count": CHUNK, "type": "VEC3"},
        {"bufferView": uv, "componentType": 5126, "count": CHUNK, "type": "VEC2"},
        {"bufferView": index_view, "componentType": 5123, "count": 3, "type": "SCALAR"},
        {"bufferView": poison_position, "componentType": 5126, "count": 3, "type": "VEC3"},
        {"bufferView": poison_normal, "componentType": 5126, "count": 3, "type": "VEC3"},
        {"bufferView": poison_uv, "componentType": 5126, "count": 3, "type": "VEC2"},
    ]);
    let shared = json!({
        "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
        "indices": 3,
        "mode": 4
    });
    let primitives = json!([
        shared,
        shared,
        shared,
        {
            "attributes": {"POSITION": 4, "NORMAL": 5, "TEXCOORD_0": 6},
            "indices": 3,
            "mode": 4
        },
    ]);
    let json = json!({
        "asset": {"version": "2.0", "generator": "valle-3d-test"},
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
        "meshes": [{"name": "product", "primitives": primitives}],
        "nodes": [{"name": "product", "mesh": 0}],
        "scenes": [{"name": "default", "nodes": [0]}],
        "scene": 0
    });
    let bytes = encode_glb(serde_json::to_vec(&json).unwrap(), bin);
    assert!(
        (bytes.len() as u64) < valle_motion::scene3d::MAX_MODEL_BYTES,
        "fixture must pass the file-size gate so the aggregate path is the one under test"
    );

    let error = admit_glb(&bytes)
        .expect_err("three shared primitives exceed the aggregate vertex budget")
        .to_string();
    assert!(error.contains("aggregate decoded vertices"), "{error}");
    assert!(
        !error.contains("positions must be finite"),
        "decoding must stop at the budget limit before the last primitive: {error}"
    );
}

#[test]
fn prepare_resolves_every_control_charges_one_aggregate_budget_and_keys_content() {
    let bytes = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let resources = resources(&bytes);
    let cache_key = prepare_cache_key(&scene(), 320, 240, &resources).unwrap();
    let prepared = prepare_scene(&scene(), 320, 240, &resources).unwrap();
    assert_eq!(prepared.budget().vertices, 3);
    assert_eq!(prepared.budget().triangles, 1);
    assert_eq!(prepared.budget().texture_pixels, 4);
    assert_eq!(
        cache_key,
        prepare_cache_key(&scene(), 320, 240, &resources).unwrap(),
        "pre-prepare key must be deterministic for the exact inputs"
    );

    let mut missing = resources.clone();
    missing.models.clear();
    assert!(
        prepare_scene(&scene(), 320, 240, &missing)
            .unwrap_err()
            .to_string()
            .contains("model3d")
    );
    assert_ne!(
        cache_key,
        prepare_cache_key(&scene(), 321, 240, &resources).unwrap()
    );

    // Shared decoding pins the actual bytes; replacing a texture changes content identity.
    let mut changed_pixels = resources.clone();
    changed_pixels.textures.insert(
        ("productTexture".into(), TextureRole::Color),
        Arc::new(texture_image(
            2,
            2,
            &[
                0, 0, 0, 0, 40, 180, 255, 255, 24, 24, 24, 64, 150, 150, 150, 180,
            ],
            TextureRole::Color,
        )),
    );
    assert_ne!(
        cache_key,
        prepare_cache_key(&scene(), 320, 240, &changed_pixels).unwrap()
    );

    // A third binding can switch between two already-counted images. Storage deduplication
    // must not remove that binding from prepared-scene identity.
    let mut scene = scene();
    scene.meshes = ["a", "b", "c"]
        .map(|key| {
            let mut mesh = scene.meshes[0].clone();
            mesh.key = key.into();
            mesh.material
                .textures
                .get_mut(&MaterialTextureSlot::BaseColor)
                .unwrap()
                .as_mut()
                .unwrap()
                .control = key.into();
            mesh
        })
        .to_vec();
    scene.anchors.clear();
    let mut bindings = resources.clone();
    let a = Arc::clone(&resources.textures[&("productTexture".into(), TextureRole::Color)]);
    let b = Arc::clone(&changed_pixels.textures[&("productTexture".into(), TextureRole::Color)]);
    bindings.textures = BTreeMap::from([
        (("a".into(), TextureRole::Color), Arc::clone(&a)),
        (("b".into(), TextureRole::Color), Arc::clone(&b)),
        (("c".into(), TextureRole::Color), a),
    ]);
    let before = prepare_cache_key(&scene, 320, 240, &bindings).unwrap();
    bindings
        .textures
        .insert(("c".into(), TextureRole::Color), b);
    assert_ne!(
        before,
        prepare_cache_key(&scene, 320, 240, &bindings).unwrap()
    );
}

#[test]
fn raster_golden_locks_depth_top_left_linear_texture_lighting_and_anchor() {
    let near = triangle(0.5, [0.0, 0.0, 1.0]);
    let far = triangle(0.0, [1.0, 0.0, 0.0]);
    let bytes = glb(&[far.clone(), near.clone()]);
    let prepared = prepare_scene(&scene(), 320, 240, &resources(&bytes)).unwrap();
    let output = render_scene(&prepared, &frame()).unwrap();
    let visible_pixels = output.object_ids.iter().filter(|id| **id != 0).count();
    let translucent_pixels = output
        .premul_rgba8
        .chunks_exact(4)
        .filter(|pixel| pixel[3] > 0 && pixel[3] < 255)
        .count();
    assert!(visible_pixels > 2_000);
    assert!(
        output
            .premul_rgba8
            .chunks_exact(4)
            .all(|pixel| pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3])
    );
    assert_eq!(output.anchors.len(), 1);
    assert!(output.anchors[0].in_front);
    assert_eq!(
        project_anchors(&scene(), &frame(), 320, 240).unwrap(),
        output.anchors,
        "layout-only projection and full raster must share one anchor result",
    );
    let actual = json!({
        "width": output.width,
        "height": output.height,
        "premulRgba8Sha256": hash(&output.premul_rgba8),
        "depthU16LeSha256": hash(&u16_bytes(&output.depth)),
        "objectIdU16LeSha256": hash(&u16_bytes(&output.object_ids)),
        "visiblePixels": visible_pixels,
        "translucentPixels": translucent_pixels,
        "anchorBits": output.anchors[0].screen.map(f32::to_bits),
    });
    assert_eq!(
        actual,
        serde_json::from_str::<serde_json::Value>(RASTER_GOLDEN).unwrap()
    );

    let reversed = glb(&[near, far]);
    let reverse_prepared = prepare_scene(&scene(), 320, 240, &resources(&reversed)).unwrap();
    let reverse = render_scene(&reverse_prepared, &frame()).unwrap();
    assert_eq!(output.premul_rgba8, reverse.premul_rgba8);
    assert_eq!(output.depth, reverse.depth);
    assert_eq!(output.object_ids, reverse.object_ids);
    assert_eq!(output.anchors, reverse.anchors);
}

#[test]
fn raster_frame_reuse_preserves_allocations_and_exact_pixels() {
    let bytes = glb(&[
        triangle(0.0, [1.0, 0.0, 0.0]),
        triangle(0.5, [0.0, 0.0, 1.0]),
    ]);
    let prepared = prepare_scene(&scene(), 320, 240, &resources(&bytes)).unwrap();
    let first = render_scene(&prepared, &frame()).unwrap();
    let color_ptr = first.premul_rgba8.as_ptr();
    let depth_ptr = first.depth.as_ptr();
    let ids_ptr = first.object_ids.as_ptr();
    let nodes_ptr = first.node_ids.as_ptr();
    let expected = render_scene(&prepared, &frame()).unwrap();
    let reused = render_scene_reusing(&prepared, &frame(), Some(first)).unwrap();
    assert_eq!(reused, expected);
    assert_eq!(reused.premul_rgba8.as_ptr(), color_ptr);
    assert_eq!(reused.depth.as_ptr(), depth_ptr);
    assert_eq!(reused.object_ids.as_ptr(), ids_ptr);
    assert_eq!(reused.node_ids.as_ptr(), nodes_ptr);
}

#[test]
fn current_frame_metadata_and_u16le_planes_share_one_object_address_truth() {
    let bytes = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let spec = scene();
    let prepared = prepare_scene(&spec, 320, 240, &resources(&bytes)).unwrap();
    let output = render_scene(&prepared, &frame()).unwrap();
    let metadata = output.metadata("stage", &spec).unwrap();
    assert_eq!(metadata.scene_key, "stage");
    assert_eq!(metadata.background_object_id, 0);
    assert_eq!(metadata.clear_depth, u16::MAX);
    assert_eq!(metadata.objects.len(), 1);
    assert_eq!(metadata.objects[0].object_id, 1);
    assert_eq!(metadata.objects[0].object_key, "product");
    assert_eq!(metadata.objects[0].semantic_address, "stage::product");
    assert_eq!(metadata.anchors.len(), 1);
    assert_eq!(metadata.anchors[0].anchor_key, "label");
    assert_eq!(metadata.anchors[0].object_key, "product");
    assert_eq!(metadata.anchors[0].semantic_address, "stage::product");
    assert_eq!(metadata.anchors[0].screen, output.anchors[0].screen);
    assert_eq!(metadata.anchors[0].depth, output.anchors[0].depth);
    let wire = serde_json::to_value(&metadata).unwrap();
    assert_eq!(
        wire["objects"][0],
        json!({
            "objectId": 1,
            "objectKey": "product",
            "semanticAddress": "stage::product",
        })
    );
    assert_eq!(wire["backgroundObjectId"], 0);
    assert_eq!(wire["clearDepth"], 65_535);
    let mut unknown = wire;
    unknown["hostGuess"] = json!(true);
    assert!(
        serde_json::from_value::<valle_motion::scene3d::Scene3DFrameMetadata>(unknown).is_err(),
        "metadata must reject host-invented fields"
    );
    assert_eq!(output.depth_u16_le_bytes(), u16_bytes(&output.depth));
    assert_eq!(
        output.object_ids_u16_le_bytes(),
        u16_bytes(&output.object_ids)
    );
    let visible = output.object_ids.iter().position(|id| *id != 0).unwrap();
    let picked = output
        .pick(
            &metadata,
            visible as u32 % output.width,
            visible as u32 / output.width,
        )
        .expect("visible object-id pixel locates its semantic object");
    assert_eq!(picked.semantic_address, "stage::product");
    assert_eq!(picked.node_id, 0);
    assert_eq!(output.node_ids_u16_le_bytes(), u16_bytes(&output.node_ids));
    assert_eq!(metadata.background_node_id, u16::MAX);
    assert_eq!(picked.depth, output.depth[visible]);
    assert!(output.pick(&metadata, 0, 0).is_none());

    let mut malformed = output.clone();
    malformed.object_ids.pop();
    let error = malformed.metadata("stage", &spec).unwrap_err();
    assert!(error.to_string().contains("object-id plane length"));
}

#[test]
fn layout_projection_keeps_offscreen_points_but_rejects_points_without_depth() {
    let mut offscreen = scene();
    offscreen.anchors[0].position = Vec3::new(100.0, 0.0, 0.0);
    let projected = project_anchors(&offscreen, &frame(), 320, 240).unwrap();
    assert!(projected[0].in_front);
    assert!(projected[0].screen[0] > 320.0, "{projected:#?}");

    let mut behind = scene();
    behind.anchors[0].position = Vec3::new(0.0, 0.0, 10.0);
    let projected = project_anchors(&behind, &frame(), 320, 240).unwrap();
    assert!(!projected[0].in_front);
}

#[test]
fn near_and_side_frustum_clipping_backface_culling_and_extreme_offscreen_input_are_bounded() {
    let mut clipped = triangle(4.2, [0.0, 0.0, 1.0]);
    clipped.positions = [[-0.1, -0.1, 4.45], [0.15, -0.1, 4.2], [0.0, 0.15, 4.2]];
    let mut clipped_scene = scene();
    let mut clipped_frame = frame();
    clipped_frame.camera.position = Vec3::new(0.0, 0.0, 4.5);
    clipped_frame.camera.target = Vec3::ZERO;
    clipped_scene.meshes[0].material.textures.clear();
    let clipped_resources = SceneResources {
        environments: BTreeMap::new(),
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[clipped])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let clipped_output = render_scene(
        &prepare_scene(&clipped_scene, 256, 256, &clipped_resources).unwrap(),
        &clipped_frame,
    )
    .unwrap();
    assert!(clipped_output.object_ids.contains(&1));

    let mut backface = triangle(0.0, [0.0, 0.0, 1.0]);
    backface.indices = [0, 2, 1];
    let backface_resources = SceneResources {
        environments: BTreeMap::new(),
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[backface])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let backface_output = render_scene(
        &prepare_scene(&clipped_scene, 256, 256, &backface_resources).unwrap(),
        &clipped_frame,
    )
    .unwrap();
    assert!(backface_output.object_ids.iter().all(|id| *id == 0));

    let ordinary = triangle(0.0, [0.0, 0.0, 1.0]);
    let offscreen_resources = SceneResources {
        environments: BTreeMap::new(),
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[ordinary])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let mut offscreen_frame = clipped_frame.clone();
    offscreen_frame.meshes[0].transform.translation.0[0] = 10_000.0;
    offscreen_frame.meshes[0].transform.scale.0[0] = 1_000.0;
    offscreen_frame.meshes[0].transform.scale.0[1] = 1_000.0;
    offscreen_frame.meshes[0].transform.scale.0[2] = 1_000.0;
    let offscreen = render_scene(
        &prepare_scene(&clipped_scene, 256, 256, &offscreen_resources).unwrap(),
        &offscreen_frame,
    )
    .unwrap();
    assert!(offscreen.object_ids.iter().all(|id| *id == 0));
}

fn rewrite_json_chunk(glb: &[u8], needle: &str, replacement: &str) -> Vec<u8> {
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = std::str::from_utf8(&glb[20..20 + json_len])
        .unwrap()
        .trim_end();
    let changed = json.replacen(needle, replacement, 1);
    assert_ne!(changed, json, "test mutation needle must exist: {needle}");
    let bin_start = 20 + json_len;
    let bin_len = u32::from_le_bytes(glb[bin_start..bin_start + 4].try_into().unwrap()) as usize;
    encode_glb(
        changed.into_bytes(),
        glb[bin_start + 8..bin_start + 8 + bin_len].to_vec(),
    )
}

fn edit_glb(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value, &mut Vec<u8>)) -> Vec<u8> {
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut root: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let mut bin = bytes[28 + json_len..].to_vec();
    bin.truncate(root["buffers"][0]["byteLength"].as_u64().unwrap() as usize);
    edit(&mut root, &mut bin);
    root["buffers"][0]["byteLength"] = json!(bin.len());
    encode_glb(serde_json::to_vec(&root).unwrap(), bin)
}

#[test]
fn hierarchy_instances_preserve_geometry_and_match_explicit_mirrored_geometry() {
    let triangle = triangle(0.0, [0.4, 0.2, 1.0]);
    let source = glb(&[triangle.clone()]);
    let instanced = edit_glb(&source, |root, _| {
        root["nodes"] = json!([
            {"name":"part","mesh":0,"translation":[-0.75,0,0],"scale":[0.45,0.6,1]},
            {"name":"part","mesh":0,"matrix":[-0.54,0,0,0,0,0.48,0,0,0,0,1,0,0.9,0,0,1]},
            {"name":"group","children":[0],"scale":[1.2,0.8,1]},
            {"mesh":0,"translation":[8,0,0]}
        ]);
        root["scenes"][0]["nodes"] = json!([2, 1]);
    });
    let model = admit_glb(&instanced).unwrap();
    assert_eq!(model.vertex_count(), 3);
    assert_eq!(model.triangle_count(), 1);
    assert_eq!(model.rendered_vertex_count(), 6);
    assert_eq!(model.rendered_triangle_count(), 2);
    assert_eq!(
        model.instances().iter().map(|i| i.node).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(model.nodes()[0].parent, Some(2));
    assert!(!model.nodes()[3].active);
    let make_baked = |mirror: bool| {
        let sign = if mirror { -1.0 } else { 1.0 };
        let mut baked = triangle.clone();
        for position in &mut baked.positions {
            position[0] = sign * (0.54 * position[0] - 0.9);
            position[1] *= 0.48;
        }
        let normal = [sign * 0.4 / 0.54, 0.2 / 0.48, 1.0];
        let length = libm::sqrtf(normal.iter().map(|v| v * v).sum());
        baked.normals = [normal.map(|v| v / length); 3];
        if mirror {
            baked.indices = [0, 2, 1];
        }
        baked
    };
    let expected = glb(&[make_baked(false), make_baked(true)]);
    let prepared = prepare_scene(&scene(), 128, 128, &resources(&instanced)).unwrap();
    assert_eq!(prepared.budget().vertices, 6);
    assert_eq!(prepared.budget().triangles, 2);
    let baked = prepare_scene(&scene(), 128, 128, &resources(&expected)).unwrap();
    let mut repeat = None;
    for angle in [60.0, 0.0, 30.0, 60.0] {
        let mut state = frame();
        state.meshes[0].transform.rotation_degrees.0[1] = angle;
        let actual = render_scene(&prepared, &state).unwrap();
        let expected = render_scene(&baked, &state).unwrap();
        assert_eq!(actual.object_ids, expected.object_ids);
        let metadata = actual.metadata("stage", &scene()).unwrap();
        for node in [0, 1] {
            let at = actual
                .node_ids
                .iter()
                .position(|&id| id == node)
                .expect("both model instances remain visible");
            let pick = actual
                .pick(&metadata, at as u32 % 128, at as u32 / 128)
                .unwrap();
            assert_eq!(pick.node_id, u32::from(node));
            assert_eq!(pick.semantic_address, "stage::product");
        }
        assert!(actual.object_ids.iter().filter(|&&id| id == 1).count() > 100);
        assert!(
            actual
                .premul_rgba8
                .iter()
                .zip(&expected.premul_rgba8)
                .all(|(&a, &b)| a.abs_diff(b) <= 1)
        );
        if angle == 60.0 {
            if let Some(previous) = repeat.take() {
                assert_eq!(actual, previous)
            } else {
                repeat = Some(actual)
            }
        }
    }
}

#[test]
fn strided_vertices_unsigned_indices_and_missing_normals_use_standard_geometry_rules() {
    let original = triangle(0.0, [0.0, 0.0, 1.0]);
    let source = glb(&[original.clone()]);
    let interleaved = edit_glb(&source, |root, bin| {
        bin.clear();
        for i in 0..3 {
            for v in original.positions[i].into_iter().chain(original.normals[i]) {
                bin.extend(v.to_le_bytes());
            }
            // Normalized U16 UVs share the same vertex view and its 28-byte stride.
            for uv in original.uvs[i] {
                bin.extend(((uv * 65535.0).round() as u16).to_le_bytes());
            }
        }
        bin.extend([0, 1, 2]);
        root["bufferViews"] = json!([
            {"buffer":0,"byteLength":84,"byteStride":28},
            {"buffer":0,"byteOffset":84,"byteLength":3}
        ]);
        root["accessors"] = json!([
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":0,"byteOffset":12,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":0,"byteOffset":24,"componentType":5123,"normalized":true,"count":3,"type":"VEC2"},
            {"bufferView":1,"componentType":5121,"count":3,"type":"SCALAR"}
        ]);
    });
    let model = admit_glb(&interleaved).unwrap();
    assert_eq!(model.indices(), [0, 1, 2]);
    for (i, v) in model.vertices().iter().enumerate() {
        assert_eq!(v.position, original.positions[i]);
        assert_eq!(v.normal, original.normals[i]);
        assert!((v.uv[0] - original.uvs[i][0]).abs() < 1.0 / 65535.0);
    }
    let no_indices = edit_glb(&source, |root, _| {
        let primitive = root["meshes"][0]["primitives"][0].as_object_mut().unwrap();
        primitive.remove("indices");
        primitive["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("NORMAL");
    });
    assert_eq!(
        admit_glb(&no_indices).unwrap().vertices(),
        admit_glb(&source).unwrap().vertices()
    );
    let no_uv = edit_glb(&no_indices, |root, _| {
        root["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("TEXCOORD_0");
    });
    assert!(!admit_glb(&no_uv).unwrap().primitives()[0].has_uv);
    assert!(
        prepare_scene(&scene(), 64, 64, &resources(&no_uv))
            .unwrap_err()
            .to_string()
            .contains("TEXCOORD_0")
    );
    // A shared corner between two faces must split into two flat normals, not their average.
    let angled = edit_glb(&no_indices, |root, bin| {
        align4(bin, 0);
        let offset = bin.len();
        for p in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ] {
            for v in p {
                bin.extend(v.to_le_bytes());
            }
        }
        root["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":48}));
        root["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":6}));
        bin.extend([0, 1, 2, 0, 2, 3]);
        root["accessors"][0] = json!({"bufferView":4,"componentType":5126,"count":4,"type":"VEC3"});
        root["accessors"][3] =
            json!({"bufferView":5,"componentType":5121,"count":6,"type":"SCALAR"});
        root["meshes"][0]["primitives"][0]["indices"] = json!(3);
        root["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("TEXCOORD_0");
    });
    let flat = admit_glb(&angled).unwrap();
    assert_eq!(flat.vertex_count(), 6);
    assert!(
        flat.vertices()[..3]
            .iter()
            .all(|v| v.normal == [0.0, 0.0, 1.0])
    );
    assert!(
        flat.vertices()[3..]
            .iter()
            .all(|v| v.normal == [1.0, 0.0, 0.0])
    );
    for (field, value, expected) in [
        ("byteStride", json!(8), "misaligned"),
        ("byteStride", json!(27), "byteStride"),
        ("byteLength", json!(80), "buffer view"),
    ] {
        let bad = edit_glb(&interleaved, |root, _| {
            root["bufferViews"][0][field] = value
        });
        assert!(admit_glb(&bad).unwrap_err().to_string().contains(expected));
    }
}

#[test]
fn invalid_hierarchy_and_instance_cost_fail_before_drawing() {
    let source = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    for (nodes, expected) in [
        (json!([{"mesh":0,"children":[0]}]), "cycle"),
        (json!([{"children":[1,1]},{"mesh":0}]), "parent"),
        (json!([{"children":[2]}]), "range"),
        (json!([{"mesh":1}]), "mesh index"),
        (json!([{"mesh":0,"scale":[0,1,1]}]), "singular"),
        (
            json!([{"mesh":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],"scale":[1,1,1]}]),
            "mutually exclusive",
        ),
        (
            json!([{"mesh":0,"matrix":[1,0,0,0,1,1,0,0,0,0,1,0,0,0,0,1]}]),
            "shear",
        ),
    ] {
        let bytes = edit_glb(&source, |root, _| root["nodes"] = nodes);
        let error = admit_glb(&bytes).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    let deep = edit_glb(&source, |root, _| {
        root["nodes"] = json!(
            (0..33)
                .map(|i| if i == 32 {
                    json!({"mesh":0})
                } else {
                    json!({"children":[i+1]})
                })
                .collect::<Vec<_>>()
        );
    });
    assert!(admit_glb(&deep).unwrap_err().to_string().contains("depth"));
    let repeated = glb(&vec![triangle(0.0, [0.0, 0.0, 1.0]); 100]);
    let repeated = edit_glb(&repeated, |root, _| {
        root["nodes"] = json!(vec![json!({"mesh":0}); 220]);
        root["scenes"][0]["nodes"] = json!((0..220).collect::<Vec<_>>());
    });
    assert!(
        admit_glb(&repeated)
            .unwrap_err()
            .to_string()
            .contains("instanced geometry")
    );
}

fn material_glb(material: serde_json::Value, pixels: &[[u8; 4]]) -> Vec<u8> {
    use image::ImageEncoder;
    let geometry = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let json_len = u32::from_le_bytes(geometry[12..16].try_into().unwrap()) as usize;
    let mut root: serde_json::Value = serde_json::from_slice(&geometry[20..20 + json_len]).unwrap();
    let mut bin = geometry[28 + json_len..].to_vec();
    let mut images = Vec::new();
    for pixel in pixels {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(pixel, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        align4(&mut bin, 0);
        let views = root["bufferViews"].as_array_mut().unwrap();
        images.push(json!({"bufferView":views.len(),"mimeType":"image/png"}));
        views.push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":png.len()}));
        bin.extend(png);
    }
    root["buffers"][0]["byteLength"] = json!(bin.len());
    root["images"] = json!(images);
    root["textures"] = json!(
        (0..pixels.len())
            .map(|source| json!({"source":source}))
            .collect::<Vec<_>>()
    );
    root["materials"] = json!([material]);
    root["meshes"][0]["primitives"][0]["material"] = json!(0);
    encode_glb(serde_json::to_vec(&root).unwrap(), bin)
}

fn pbr_frame() -> Frame3DState {
    let mut frame = frame();
    frame.camera.position = Vec3::new(0.0, 0.0, 4.5);
    frame.camera.target = Vec3::ZERO;
    frame.meshes[0].material = MaterialFrameState::default();
    frame
}

fn pbr_scene() -> Scene3DSpec {
    let mut s = scene();
    s.meshes[0].material = MaterialSpec::default();
    s
}

#[test]
fn embedded_pbr_maps_keep_color_roles_alpha_semantics_and_random_access() {
    let material = json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1}},
        "emissiveTexture":{"index":2},"emissiveFactor":[1,1,1],"occlusionTexture":{"index":3},"normalTexture":{"index":4}});
    let bytes = material_glb(
        material,
        &[
            [128, 64, 32, 0],
            [0, 255, 0, 255],
            [64, 0, 0, 0],
            [128, 0, 0, 255],
            [128, 128, 255, 255],
        ],
    );
    let r = resources(&bytes);
    let model = &r.models["productModel"];
    assert_eq!(model.materials().len(), 1);
    assert_eq!(model.images().len(), 5);
    assert_eq!(model.primitives()[0].material_index, Some(0));
    let s = pbr_scene();
    let prepared = prepare_scene(&s, 64, 64, &r).unwrap();
    assert_eq!(prepared.budget().textures, 5);
    let mut f = pbr_frame();
    f.lights = test_lights(core::f32::consts::PI, 0.0);
    let expected = render_scene(&prepared, &f).unwrap();
    let pixel = &expected.premul_rgba8[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4];
    let decode = |v: f64| ((v / 255.0 + 0.055) / 1.055).powf(2.4);
    let encode = |v: f64| {
        ((if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    };
    assert_eq!(
        pixel,
        &[
            encode(decode(128.0) * 128.0 / 255.0 * 0.9852957129197725 + decode(64.0)),
            encode(decode(64.0) * 128.0 / 255.0 * 0.9852957129197725),
            encode(decode(32.0) * 128.0 / 255.0 * 0.9852957129197725),
            255
        ]
    );
    let mut reusable = Some(expected.clone());
    for angle in [35.0, 0.0, -40.0, 0.0, 35.0, 0.0] {
        f.meshes[0].transform.rotation_degrees.0[1] = angle;
        let cold = render_scene(&prepare_scene(&s, 64, 64, &r).unwrap(), &f).unwrap();
        let warm = render_scene_reusing(&prepared, &f, reusable).unwrap();
        assert_eq!(cold, warm);
        if angle == 0.0 {
            assert_eq!(cold, expected);
        }
        reusable = Some(warm);
    }
}

#[test]
fn alpha_mask_discards_visibility_while_opaque_and_double_sided_follow_material_rules() {
    let mut spec = pbr_scene();
    let mut behind = spec.meshes[0].clone();
    behind.key = "behind".into();
    behind.model_control = "behindModel".into();
    spec.meshes.push(behind);
    let back = material_glb(json!({"emissiveFactor":[0,1,0]}), &[]);
    let mut state = pbr_frame();
    state.lights = test_lights(0.0, 0.0);
    let mut behind = state.meshes[0].clone();
    behind.key = "behind".into();
    behind.transform.translation.0[2] = -0.5;
    state.meshes.push(behind);
    let center = 32 * 64 + 32;
    for (mode, alpha, tex_alpha, visible) in [
        ("MASK", 1.0, 0, false),
        ("MASK", 0.5, 255, true),
        ("MASK", 0.5, 128, false),
        ("OPAQUE", 0.0, 0, true),
    ] {
        let front = material_glb(
            json!({
                "alphaMode":mode,"alphaCutoff":0.5,"doubleSided":true,"emissiveFactor":[1,0,0],
                "pbrMetallicRoughness":{"baseColorFactor":[1,1,1,alpha],"baseColorTexture":{"index":0}}
            }),
            &[[255, 255, 255, tex_alpha]],
        );
        let mut r = resources(&front);
        r.models
            .insert("behindModel".into(), Arc::new(admit_glb(&back).unwrap()));
        let prepared = prepare_scene(&spec, 64, 64, &r).unwrap();
        let expected_color = if visible {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        };
        let expected_id = if visible { 1 } else { 2 };
        let mut first = None;
        for angle in [180.0, 0.0, 180.0] {
            state.meshes[0].transform.rotation_degrees.0[1] = angle;
            let output = render_scene(&prepared, &state).unwrap();
            assert_eq!(
                output.premul_rgba8[center * 4..center * 4 + 4],
                expected_color
            );
            assert_eq!(output.object_ids[center], expected_id);
            assert_eq!(output.node_ids[center], 0);
            let metadata = output.metadata("stage", &spec).unwrap();
            assert_eq!(
                output.pick(&metadata, 32, 32).unwrap().object_id,
                expected_id
            );
            if angle == 180.0 {
                if let Some(previous) = first.take() {
                    assert_eq!(previous, output)
                } else {
                    first = Some(output)
                }
            }
        }
        // The same reversed face is culled when the material is single-sided.
        let single = edit_glb(&front, |root, _| {
            root["materials"][0]["doubleSided"] = json!(false)
        });
        r.models
            .insert("productModel".into(), Arc::new(admit_glb(&single).unwrap()));
        let output = render_scene(&prepare_scene(&spec, 64, 64, &r).unwrap(), &state).unwrap();
        assert_eq!(output.object_ids[center], 2);
    }
    let bad = material_glb(json!({"alphaMode":"MASK","alphaCutoff":-0.1}), &[]);
    assert!(admit_glb(&bad).is_err());
}

#[test]
fn normal_map_changes_direct_light_while_occlusion_only_changes_indirect_light() {
    let material = json!({"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.5,1],"metallicFactor":0,"roughnessFactor":1},
        "normalTexture":{"index":0},"occlusionTexture":{"index":1}});
    for kind in [MaterialKind::Pbr, MaterialKind::Lambert] {
        let mut s = pbr_scene();
        s.meshes[0].material.kind = Some(kind);
        let mut f = pbr_frame();
        f.lights = test_lights(0.0, 1.0);
        let render = |normal, ao| {
            render_scene(
                &prepare_scene(
                    &s,
                    64,
                    64,
                    &resources(&material_glb(material.clone(), &[normal, ao])),
                )
                .unwrap(),
                &f,
            )
            .unwrap()
        };
        let flat = render([128, 128, 255, 255], [0, 0, 0, 255]);
        let unoccluded = render([128, 128, 255, 255], [255, 255, 255, 255]);
        assert_eq!(flat, unoccluded, "AO must not darken direct illumination");
        let tilted = render([255, 128, 128, 255], [0, 0, 0, 255]);
        let center = (32 * 64 + 32) * 4;
        assert!(flat.premul_rgba8[center] > 100);
        assert!(tilted.premul_rgba8[center] < 20);
        assert_eq!(flat.depth, tilted.depth);
    }
}

#[test]
fn current_frame_colors_directions_and_exposure_share_one_linear_output_rule() {
    let bytes = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let r = resources(&bytes);
    let mut spec = scene();
    spec.meshes[0].material.textures.clear();
    let mut state = pbr_frame();
    state.meshes[0].material.color = Some(Color4([128.0 / 255.0, 0.0, 0.0, 1.0]));
    state.lights = test_lights(0.5, 0.0);
    let center = (32 * 64 + 32) * 4;
    let output = render_scene(&prepare_scene(&spec, 64, 64, &r).unwrap(), &state).unwrap();
    let linear = ((128.0f64 / 255.0 + 0.055) / 1.055).powf(2.4) * 0.5;
    let expected = ((1.055 * linear.powf(1.0 / 2.4) - 0.055) * 255.0).round() as u8;
    assert_eq!(
        output.premul_rgba8[center], expected,
        "Lambert light scales linear radiance"
    );
    state.meshes[0].material.color = Some(Color4([1.0; 4]));
    spec.lights.push(LightKind::Hemisphere);
    state.lights = vec![
        LightFrameState::Ambient {
            color: Color4([0.0, 0.0, 1.0, 1.0]),
            intensity: 0.25,
        },
        LightFrameState::Directional {
            color: Color4([1.0, 0.0, 0.0, 1.0]),
            direction: Vec3::new(0.0, 0.0, 1.0),
            intensity: 0.25,
        },
        LightFrameState::Hemisphere {
            sky_color: Color4([1.0, 0.0, 0.0, 1.0]),
            ground_color: Color4([0.0, 0.0, 1.0, 1.0]),
            direction: Vec3::new(1.0, 0.0, 0.0),
            intensity: 0.5,
        },
    ];
    let prepared = prepare_scene(&spec, 64, 64, &r).unwrap();
    let mut first = None;
    for exposure in [2.0, 0.0, 1.0, 2.0] {
        state.exposure = exposure;
        let output = render_scene(&prepared, &state).unwrap();
        let expected = match exposure {
            0.0 => 0,
            1.0 => 188,
            _ => 255,
        };
        assert_eq!(
            output.premul_rgba8[center..center + 4],
            [expected, 0, expected, 255]
        );
        if exposure == 2.0 {
            if let Some(previous) = first.take() {
                assert_eq!(previous, output)
            } else {
                first = Some(output)
            }
        }
    }
    state.exposure = 1.0;
    if let LightFrameState::Hemisphere { direction, .. } = &mut state.lights[2] {
        *direction = Vec3::new(0.0, 0.0, 1.0);
    }
    let directed = render_scene(&prepared, &state).unwrap();
    assert!(directed.premul_rgba8[center] > directed.premul_rgba8[center + 2]);
    // The output configuration also applies to unlit materials; changing exposure only touches
    // frame values and keeps the prepared model and visibility planes intact.
    spec.meshes[0].material.kind = Some(MaterialKind::Unlit);
    spec.pbr.tone_mapping = valle_motion::scene3d::ToneMapping::Aces;
    let prepared = prepare_scene(&spec, 64, 64, &r).unwrap();
    let bright = render_scene(&prepared, &state).unwrap();
    state.exposure = 0.0;
    let dark = render_scene(&prepared, &state).unwrap();
    assert_eq!(dark.premul_rgba8[center..center + 4], [0, 0, 0, 255]);
    assert!(bright.premul_rgba8[center] > 0);
    assert_eq!(bright.depth, dark.depth);
    assert_eq!(bright.node_ids, dark.node_ids);
}

#[test]
fn one_image_can_fill_color_and_data_slots_with_separate_mips_and_identity() {
    let material = json!({
        "pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":1},
        "occlusionTexture":{"index":0}
    });
    let shared = material_glb(material.clone(), &[[128, 64, 32, 255]]);
    let mut separate_material = material;
    separate_material["occlusionTexture"]["index"] = json!(1);
    let separate = material_glb(separate_material, &[[128, 64, 32, 255], [128, 64, 32, 255]]);
    let spec = pbr_scene();
    let mut state = pbr_frame();
    state.lights = test_lights(core::f32::consts::PI, 0.0);
    let mut outputs = Vec::new();
    for bytes in [&shared, &separate] {
        let r = resources(bytes);
        let model = &r.models["productModel"];
        assert_eq!(model.images().len(), 2);
        assert_eq!(
            model.images()[0].content_digest(),
            model.images()[1].content_digest()
        );
        let prepared = prepare_scene(&spec, 64, 64, &r).unwrap();
        assert_eq!(prepared.budget().textures, 2);
        assert_eq!(prepared.budget().texture_pixels, 2);
        outputs.push(render_scene(&prepared, &state).unwrap());
    }
    assert_eq!(outputs[0], outputs[1]);
    // Independent expectation: base R decodes sRGB, while occlusion R remains 128/255.
    let linear =
        ((128.0f64 / 255.0 + 0.055) / 1.055).powf(2.4) * 128.0 / 255.0 * 0.9852957129197725;
    let expected = ((1.055 * linear.powf(1.0 / 2.4) - 0.055) * 255.0).round() as u8;
    assert_eq!(outputs[0].premul_rgba8[(32 * 64 + 32) * 4], expected);
}

#[test]
fn static_glb_node_trs_preserves_source_geometry_and_invalid_quaternion_fails() {
    let bytes = glb(&[triangle(0.0, [0.0, 0.0, 1.0])]);
    let rotated = rewrite_json_chunk(
        &bytes,
        "\"mesh\":0",
        "\"mesh\":0,\"rotation\":[0.70710678,0,0,0.70710678],\"translation\":[1,2,3],\"scale\":[2,3,4]",
    );
    let model = admit_glb(&rotated).unwrap();
    assert_eq!(model.vertices(), admit_glb(&bytes).unwrap().vertices());
    assert_eq!(model.nodes()[0].id, 0);
    let matrix = model.instances()[0].world_transform();
    let position = model.vertices()[0].position;
    let transformed: [f32; 3] = std::array::from_fn(|r| {
        matrix[r] * position[0]
            + matrix[4 + r] * position[1]
            + matrix[8 + r] * position[2]
            + matrix[12 + r]
    });
    for (got, want) in transformed.into_iter().zip([-0.8, 2.0, 0.84]) {
        assert!((got - want).abs() < 1e-5);
    }
    let invalid = rewrite_json_chunk(&bytes, "\"mesh\":0", "\"mesh\":0,\"rotation\":[1,1,1,1]");
    assert!(
        admit_glb(&invalid)
            .unwrap_err()
            .to_string()
            .contains("quaternion")
    );
}

#[test]
fn external_environment_and_aces_are_part_of_frame_identity_and_survive_reuse() {
    use valle_motion::scene3d::{
        EnvironmentAsset, EnvironmentSettings, EnvironmentSpec, ToneMapping,
    };
    let bytes = material_glb(
        json!({"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.7,0.9,1],"metallicFactor":1,"roughnessFactor":0.3}}),
        &[],
    );
    let mut r = resources(&bytes);
    let mut panorama = Vec::new();
    let pixels = (0..32)
        .map(|i| {
            image::Rgb(if i % 8 < 4 {
                [4.0, 0.4, 0.1]
            } else {
                [0.1, 0.4, 3.0]
            })
        })
        .collect::<Vec<_>>();
    image::codecs::hdr::HdrEncoder::new(&mut panorama)
        .encode(&pixels, 8, 4)
        .unwrap();
    r.environments.insert(
        "sky".into(),
        Arc::new(
            EnvironmentAsset::from_encoded(
                &panorama,
                EnvironmentSettings {
                    face_size: 16,
                    diffuse_size: 4,
                    samples: 32,
                },
            )
            .unwrap(),
        ),
    );
    let mut s = pbr_scene();
    let mut f = pbr_frame();
    f.lights = test_lights(0.0, 0.0);
    let dark = render_scene(&prepare_scene(&s, 64, 64, &r).unwrap(), &f).unwrap();
    assert!(
        dark.premul_rgba8
            .chunks_exact(4)
            .all(|p| p[..3] == [0, 0, 0])
    );
    let before = prepare_cache_key(&s, 64, 64, &r).unwrap();
    s.pbr.environment = Some(EnvironmentSpec {
        control: "sky".into(),
        background: false,
    });
    s.pbr.tone_mapping = ToneMapping::Aces;
    f.exposure = 1.1;
    assert_ne!(before, prepare_cache_key(&s, 64, 64, &r).unwrap());
    let prepared = prepare_scene(&s, 64, 64, &r).unwrap();
    let lit = render_scene(&prepared, &f).unwrap();
    assert!(
        lit.premul_rgba8
            .chunks_exact(4)
            .filter(|p| p[0] > 50 && p[3] == 255)
            .count()
            > 100
    );
    let mut reused = Some(lit.clone());
    for angle in [30.0, -40.0, 0.0, 30.0, 0.0] {
        f.environment_rotation_degrees = angle;
        let cold = render_scene(&prepare_scene(&s, 64, 64, &r).unwrap(), &f).unwrap();
        let warm = render_scene_reusing(&prepared, &f, reused).unwrap();
        assert_eq!(cold, warm);
        if angle == 0.0 {
            assert_eq!(cold, lit);
        } else {
            assert_ne!(cold, lit);
        }
        reused = Some(warm);
    }
    let lit_key = prepare_cache_key(&s, 64, 64, &r).unwrap();
    f.exposure = 0.0;
    assert_eq!(
        lit_key,
        prepare_cache_key(&s, 64, 64, &r).unwrap(),
        "frame exposure must not rebuild immutable resources"
    );
    let zero = render_scene(&prepare_scene(&s, 64, 64, &r).unwrap(), &f).unwrap();
    assert!(
        zero.premul_rgba8
            .chunks_exact(4)
            .all(|p| p[..3] == [0, 0, 0])
    );
}

#[test]
fn frame_node_replacements_match_static_hierarchy_and_never_mutate_shared_models() {
    use valle_motion::scene3d::NodeFrameState;
    let source = glb(&[triangle(0.0, [0.4, 0.2, 1.0])]);
    let model = edit_glb(&source, |root, _| {
        root["nodes"] = json!([
            {"mesh":0,"translation":[-0.6,0,0],"scale":[0.5,0.6,1]},
            {"mesh":0,"translation":[4,0,0]},
            {"children":[0],"translation":[5,0,0]},
            {"mesh":0}
        ]);
        root["scenes"][0]["nodes"] = json!([2, 1]);
    });
    let resources = resources(&model);
    let frozen_nodes = resources.models["productModel"].nodes().to_vec();
    let mut spec = scene();
    spec.meshes[0].node_ids = vec![2, 1];
    let prepared = prepare_scene(&spec, 128, 128, &resources).unwrap();
    let state_at = |angle: f32| {
        let mut state = frame();
        state.meshes[0].transform.scale = Vec3::new(-1.0, 1.0, 1.0);
        state.meshes[0].nodes = vec![
            NodeFrameState {
                id: 2,
                transform: Transform3D {
                    translation: Vec3::new(angle * 0.002, 0.0, 0.0),
                    rotation_degrees: Vec3::new(0.0, angle, 0.0),
                    scale: Vec3::new(1.2, 0.8, 1.0),
                },
            },
            NodeFrameState {
                id: 1,
                transform: Transform3D {
                    translation: Vec3::new(0.8, 0.0, 0.0),
                    rotation_degrees: Vec3::new(0.0, 0.0, angle),
                    scale: Vec3::new(-0.5, 0.6, 1.0),
                },
            },
        ];
        state
    };
    let mut repeat = None;
    let mut reusable = None;
    for angle in [60.0f32, 0.0, 30.0, 60.0] {
        let state = state_at(angle);
        let actual = render_scene_reusing(&prepared, &state, reusable).unwrap();
        let expected_model = edit_glb(&model, |root, _| {
            let half = angle * std::f32::consts::PI / 360.0;
            root["nodes"][2] = json!({"children":[0],"translation":[angle*0.002,0,0],"rotation":[0,libm::sinf(half),0,libm::cosf(half)],"scale":[1.2,0.8,1]});
            root["nodes"][1] = json!({"mesh":0,"translation":[0.8,0,0],"rotation":[0,0,libm::sinf(half),libm::cosf(half)],"scale":[-0.5,0.6,1]});
        });
        let mut expected_resources = resources.clone();
        expected_resources.models.insert(
            "productModel".into(),
            Arc::new(admit_glb(&expected_model).unwrap()),
        );
        let expected_prepared = prepare_scene(&scene(), 128, 128, &expected_resources).unwrap();
        let mut static_state = state.clone();
        static_state.meshes[0].nodes.clear();
        let expected = render_scene(&expected_prepared, &static_state).unwrap();
        assert_eq!(actual.object_ids, expected.object_ids);
        assert_eq!(actual.node_ids, expected.node_ids);
        assert!(
            actual
                .premul_rgba8
                .iter()
                .zip(&expected.premul_rgba8)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert!(
            actual
                .depth
                .iter()
                .zip(&expected.depth)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert!(actual.node_ids.contains(&0) && actual.node_ids.contains(&1));
        if angle == 60.0 {
            if let Some(previous) = &repeat {
                assert_eq!(&actual, previous);
            } else {
                repeat = Some(actual.clone());
            }
        }
        reusable = Some(actual);
    }
    std::thread::scope(|scope| {
        let a = scope.spawn(|| render_scene(&prepared, &state_at(60.0)).unwrap());
        let b = scope.spawn(|| render_scene(&prepared, &state_at(60.0)).unwrap());
        assert_eq!(a.join().unwrap(), repeat.clone().unwrap());
        assert_eq!(b.join().unwrap(), repeat.clone().unwrap());
    });
    assert_eq!(resources.models["productModel"].nodes(), frozen_nodes);
    for id in [3, 255] {
        let mut invalid = spec.clone();
        invalid.meshes[0].node_ids = vec![id];
        assert!(prepare_scene(&invalid, 128, 128, &resources).is_err());
    }
    let mut invalid = state_at(0.0);
    invalid.meshes[0].nodes[0].transform.translation = Vec3::new(10_000.0, 0.0, 0.0);
    invalid.meshes[0].nodes[0].transform.scale = Vec3::new(-1000.0, 1.0, 1.0);
    assert!(render_scene(&prepared, &invalid).is_err());
}

#[test]
fn external_material_slots_match_embedded_maps_and_content_replacement_invalidates_prepare() {
    use valle_motion::scene3d::MaterialTextureSlot::*;
    let pixels = [
        [180, 80, 40, 255],
        [0, 150, 200, 0],
        [128, 128, 255, 0],
        [128, 0, 0, 0],
        [100, 50, 200, 0],
    ];
    let material = json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1}},"normalTexture":{"index":2},"occlusionTexture":{"index":3},"emissiveTexture":{"index":4},"emissiveFactor":[0.2,0.3,0.4]});
    let embedded = material_glb(material, &pixels);
    let external_model = edit_glb(&embedded, |root, _| {
        root["images"] = json!([]);
        root["textures"] = json!([]);
        let material = &mut root["materials"][0];
        let pbr = material["pbrMetallicRoughness"].as_object_mut().unwrap();
        pbr.remove("baseColorTexture");
        pbr.remove("metallicRoughnessTexture");
        let material = material.as_object_mut().unwrap();
        for key in ["normalTexture", "occlusionTexture", "emissiveTexture"] {
            material.remove(key);
        }
    });
    let baseline = prepare_scene(&pbr_scene(), 64, 64, &resources(&embedded)).unwrap();
    let mut resources = resources(&external_model);
    resources.textures.clear();
    let mut spec = pbr_scene();
    for (i, slot) in [BaseColor, MetallicRoughness, Normal, Occlusion, Emissive]
        .into_iter()
        .enumerate()
    {
        let control = format!("map{i}");
        resources.textures.insert(
            (control.clone(), slot.role()),
            Arc::new(texture_image(1, 1, &pixels[i], slot.role())),
        );
        spec.meshes[0].material.textures.insert(
            slot,
            Some(MaterialTexture {
                control,
                wrap_u: TextureWrap::Repeat,
                wrap_v: TextureWrap::Repeat,
                min_filter: TextureFilter::Linear,
                mag_filter: TextureFilter::Linear,
                mipmap: MipmapFilter::Linear,
            }),
        );
    }
    let key = prepare_cache_key(&spec, 64, 64, &resources).unwrap();
    let prepared = prepare_scene(&spec, 64, 64, &resources).unwrap();
    assert_eq!(prepared.budget().textures, 5);
    let mut repeat = None;
    for angle in [60.0, 0.0, 30.0, 60.0] {
        let mut state = pbr_frame();
        state.meshes[0].transform.rotation_degrees.0[1] = angle;
        let actual = render_scene(&prepared, &state).unwrap();
        assert_eq!(actual, render_scene(&baseline, &state).unwrap());
        if angle == 60.0 {
            if let Some(previous) = repeat.take() {
                assert_eq!(actual, previous);
            } else {
                repeat = Some(actual);
            }
        }
    }
    resources.textures.insert(
        ("map4".into(), TextureRole::Color),
        Arc::new(texture_image(1, 1, &[0, 255, 0, 0], TextureRole::Color)),
    );
    assert_ne!(key, prepare_cache_key(&spec, 64, 64, &resources).unwrap());
    assert_ne!(
        render_scene(&prepared, &pbr_frame()).unwrap().premul_rgba8,
        render_scene(
            &prepare_scene(&spec, 64, 64, &resources).unwrap(),
            &pbr_frame()
        )
        .unwrap()
        .premul_rgba8
    );
    let mut changed_sampling = spec.clone();
    changed_sampling.meshes[0]
        .material
        .textures
        .get_mut(&BaseColor)
        .unwrap()
        .as_mut()
        .unwrap()
        .wrap_u = TextureWrap::Mirror;
    assert_ne!(
        prepare_cache_key(&spec, 64, 64, &resources).unwrap(),
        prepare_cache_key(&changed_sampling, 64, 64, &resources).unwrap()
    );
    resources
        .textures
        .remove(&("map2".into(), TextureRole::Data));
    assert!(prepare_scene(&spec, 64, 64, &resources).is_err());
}

#[test]
fn material_index_overrides_merge_with_complete_frame_values_and_common_alpha_rules() {
    use valle_motion::scene3d::{AlphaMode, MaterialOverrideSpec, MaterialOverrideState};
    let model = edit_glb(
        &material_glb(json!({}), &[[192, 128, 224, 255], [128, 0, 0, 255]]),
        |root, _| {
            root["materials"] = json!([{"pbrMetallicRoughness":{"baseColorFactor":[0,0,1,1]},"normalTexture":{"index":0},"occlusionTexture":{"index":1}},{"pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1]}}]);
            root["meshes"][0]["primitives"][0]["material"] = json!(0);
            let mut back = root["meshes"][0].clone();
            back["primitives"][0]["material"] = json!(1);
            root["meshes"].as_array_mut().unwrap().push(back);
            root["nodes"] =
                json!([{"mesh":0},{"mesh":1,"translation":[0,0,-0.5],"scale":[1.2,1.2,1]}]);
            root["scenes"][0]["nodes"] = json!([0, 1]);
        },
    );
    let resources = resources(&model);
    for kind in [
        MaterialKind::Pbr,
        MaterialKind::Lambert,
        MaterialKind::Unlit,
    ] {
        let mut spec = pbr_scene();
        spec.meshes[0].material.kind = Some(kind);
        spec.meshes[0].material_overrides = vec![MaterialOverrideSpec {
            id: 0,
            material: MaterialSpec {
                alpha_mode: Some(AlphaMode::Mask),
                double_sided: Some(true),
                ..Default::default()
            },
        }];
        let prepared = prepare_scene(&spec, 64, 64, &resources).unwrap();
        let state_at = |value: f32| {
            let mut state = pbr_frame();
            state.meshes[0].material = MaterialFrameState {
                color: Some(Color4([0.0, 1.0, 0.0, 1.0])),
                metallic: Some(0.0),
                roughness: Some(1.0),
                emissive_intensity: Some(0.5),
                ..Default::default()
            };
            state.meshes[0].material_overrides = vec![MaterialOverrideState {
                id: 0,
                material: MaterialFrameState {
                    color: Some(Color4([1.0, 0.0, 0.0, value])),
                    metallic: Some(value),
                    roughness: Some(0.2 + value * 0.6),
                    emissive: Some(Color4([0.0, 0.0, 1.0, 1.0])),
                    emissive_intensity: Some(value),
                    normal_scale: Some(value),
                    occlusion_strength: Some(value),
                    alpha_cutoff: Some(0.5),
                },
            }];
            state
        };
        let mut reuse = None;
        let mut first = None;
        for value in [1.0, 0.0, 0.5, 1.0] {
            let state = state_at(value);
            let actual = render_scene_reusing(&prepared, &state, reuse).unwrap();
            let center = 32 * 64 + 32;
            assert_eq!(actual.node_ids[center], if value < 0.5 { 1 } else { 0 });
            assert_eq!(actual.premul_rgba8[center * 4 + 3], 255);
            let baked_model = edit_glb(&model, |root, _| {
                root["materials"][1] = json!({"pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1],"metallicFactor":0,"roughnessFactor":1}});
                root["materials"][0] = json!({"alphaMode":"MASK","alphaCutoff":0.5,"doubleSided":true,"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,value],"metallicFactor":value,"roughnessFactor":0.2+value*0.6},"emissiveFactor":[0,0,value],"normalTexture":{"index":0,"scale":value},"occlusionTexture":{"index":1,"strength":value}});
            });
            let mut baked_resources = resources.clone();
            baked_resources.models.insert(
                "productModel".into(),
                Arc::new(admit_glb(&baked_model).unwrap()),
            );
            let mut baked_spec = pbr_scene();
            baked_spec.meshes[0].material.kind = Some(kind);
            let expected = render_scene(
                &prepare_scene(&baked_spec, 64, 64, &baked_resources).unwrap(),
                &pbr_frame(),
            )
            .unwrap();
            assert_eq!(actual, expected);
            if value == 1.0 {
                if let Some(previous) = &first {
                    assert_eq!(&actual, previous);
                } else {
                    first = Some(actual.clone());
                }
            }
            reuse = Some(actual);
        }
        std::thread::scope(|scope| {
            let a = scope.spawn(|| render_scene(&prepared, &state_at(1.0)).unwrap());
            let b = scope.spawn(|| render_scene(&prepared, &state_at(1.0)).unwrap());
            assert_eq!(a.join().unwrap(), first.clone().unwrap());
            assert_eq!(b.join().unwrap(), first.unwrap());
        });
        spec.meshes[0].material_overrides[0].id = 2;
        assert!(prepare_scene(&spec, 64, 64, &resources).is_err());
    }
}
