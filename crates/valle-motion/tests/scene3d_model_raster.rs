use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use sha2::{Digest, Sha256};
use valle_motion::{
    ContentDigest,
    scene3d::{
        AnchorSpec, CameraFrameState, CameraSpec, Color4, Frame3DState, LightSpec, MaterialKind,
        MaterialSpec, MeshFrameState, MeshSpec, Scene3DSpec, SceneResources, TextureAsset,
        Transform3D, Vec3, admit_glb, prepare_cache_key, prepare_scene, project_anchors,
        render_scene, render_scene_reusing,
    },
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
        camera: CameraSpec {
            position: Vec3::new(0.0, 0.35, 4.5),
            target: Vec3::new(0.0, 0.1, 0.0),
            fov_y_degrees: 38.0,
            near: 0.1,
            far: 20.0,
        },
        meshes: vec![MeshSpec {
            key: "product".into(),
            model_control: "productModel".into(),
            material: MaterialSpec {
                kind: MaterialKind::Lambert,
                color: Color4([0.8, 0.95, 1.0, 0.82]),
                texture_control: Some("productTexture".into()),
            },
            transform: Transform3D::default(),
        }],
        lights: vec![
            LightSpec::Ambient { intensity: 0.12 },
            LightSpec::Directional {
                direction: Vec3::new(0.0, 0.0, 1.0),
                intensity: 0.88,
            },
        ],
        anchors: vec![AnchorSpec {
            key: "label".into(),
            parent: "product".into(),
            position: Vec3::new(0.62, 0.54, 0.5),
        }],
    }
}

fn frame() -> Frame3DState {
    Frame3DState {
        camera: CameraFrameState {
            orbit_yaw_degrees: 0.0,
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
            rotation_y_degrees: 0.0,
            rotation_z_degrees: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            scale_z: 1.0,
        }],
        light_intensities: vec![0.12, 0.88],
    }
}

fn resources(model_bytes: &[u8]) -> SceneResources {
    let texture_bytes = vec![
        255, 80, 30, 255, 40, 180, 255, 255, 24, 24, 24, 64, 150, 150, 150, 180,
    ];
    SceneResources {
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(model_bytes).unwrap()),
        )]),
        textures: BTreeMap::from([(
            "productTexture".into(),
            Arc::new(
                TextureAsset::new(
                    ContentDigest::of_bytes(b"project-image-control-content"),
                    2,
                    2,
                    texture_bytes,
                )
                .unwrap(),
            ),
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
fn external_uri_extensions_materials_bad_indices_and_missing_attributes_fail_closed() {
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
        ("\"asset\":{", "\"materials\":[],\"asset\":{", "materials"),
        ("\"NORMAL\":1,", "", "NORMAL"),
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

    // Content identity alone cannot hide a host decoder mismatch: decoded premul bytes are also
    // part of the prepare key, so a stale Native/Web texture cache cannot remain silently valid.
    let mut changed_pixels = resources.clone();
    changed_pixels.textures.insert(
        "productTexture".into(),
        Arc::new(
            TextureAsset::new(
                ContentDigest::of_bytes(b"project-image-control-content"),
                2,
                2,
                vec![
                    0, 0, 0, 0, 40, 180, 255, 255, 24, 24, 24, 64, 150, 150, 150, 180,
                ],
            )
            .unwrap(),
        ),
    );
    assert_ne!(
        cache_key,
        prepare_cache_key(&scene(), 320, 240, &changed_pixels).unwrap()
    );
}

#[test]
fn raster_golden_locks_depth_top_left_lighting_texture_premul_and_anchor() {
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
    let expected = render_scene(&prepared, &frame()).unwrap();
    let reused = render_scene_reusing(&prepared, &frame(), Some(first)).unwrap();
    assert_eq!(reused, expected);
    assert_eq!(reused.premul_rgba8.as_ptr(), color_ptr);
    assert_eq!(reused.depth.as_ptr(), depth_ptr);
    assert_eq!(reused.object_ids.as_ptr(), ids_ptr);
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
    clipped_scene.camera.position = Vec3::new(0.0, 0.0, 4.5);
    clipped_scene.camera.target = Vec3::ZERO;
    clipped_scene.meshes[0].material.texture_control = None;
    let clipped_resources = SceneResources {
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[clipped])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let clipped_output = render_scene(
        &prepare_scene(&clipped_scene, 256, 256, &clipped_resources).unwrap(),
        &frame(),
    )
    .unwrap();
    assert!(clipped_output.object_ids.contains(&1));

    let mut backface = triangle(0.0, [0.0, 0.0, 1.0]);
    backface.indices = [0, 2, 1];
    let backface_resources = SceneResources {
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[backface])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let backface_output = render_scene(
        &prepare_scene(&clipped_scene, 256, 256, &backface_resources).unwrap(),
        &frame(),
    )
    .unwrap();
    assert!(backface_output.object_ids.iter().all(|id| *id == 0));

    let ordinary = triangle(0.0, [0.0, 0.0, 1.0]);
    let offscreen_resources = SceneResources {
        models: BTreeMap::from([(
            "productModel".into(),
            Arc::new(admit_glb(&glb(&[ordinary])).unwrap()),
        )]),
        textures: BTreeMap::new(),
    };
    let mut offscreen_frame = frame();
    offscreen_frame.meshes[0].translation_x = 10_000.0;
    offscreen_frame.meshes[0].scale_x = 1_000.0;
    offscreen_frame.meshes[0].scale_y = 1_000.0;
    offscreen_frame.meshes[0].scale_z = 1_000.0;
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
