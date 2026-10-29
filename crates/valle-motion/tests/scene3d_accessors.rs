use serde_json::{Value, json};
use valle_motion::scene3d::admit_glb;

fn pack(root: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(root).unwrap();
    json.resize(json.len().div_ceil(4) * 4, b' ');
    let mut bin = bin.to_vec();
    bin.resize(bin.len().div_ceil(4) * 4, 0);
    let mut out = Vec::new();
    for word in [
        0x46546c67,
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942_u32.to_le_bytes());
    out.extend(bin);
    out
}

// A rectangle with integer positions, packed signed normals, and a sparse morph delta.
fn fixture() -> (Value, Vec<u8>) {
    let mut bin = Vec::new();
    for p in [[-1_i16, -1, 0], [1, -1, 0], [-1, 1, 0], [1, 1, 0]] {
        for v in p {
            bin.extend(v.to_le_bytes());
        }
        bin.extend([0, 0]);
    }
    for _ in 0..4 {
        bin.extend([0, 0, 127, 0]);
    }
    bin.extend([1, 3, 0, 0]); // sparse indices, padded before values
    bin.extend([0, 0, 127, 0, 0, 0, 64, 0]); // signed normalized morph values with vertex padding
    let root = json!({
        "asset":{"version":"2.0"},
        "extensionsUsed":["KHR_mesh_quantization"],"extensionsRequired":["KHR_mesh_quantization"],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":32,"byteStride":8},
            {"buffer":0,"byteOffset":32,"byteLength":16,"byteStride":4},
            {"buffer":0,"byteOffset":48,"byteLength":2},
            {"buffer":0,"byteOffset":52,"byteLength":8}
        ],
        "accessors":[
            {"bufferView":0,"componentType":5122,"type":"VEC3","count":4},
            {"bufferView":1,"componentType":5120,"normalized":true,"type":"VEC3","count":4},
            {"componentType":5120,"normalized":true,"type":"VEC3","count":4,"sparse":{"count":2,"indices":{"bufferView":2,"componentType":5121},"values":{"bufferView":3}}}
        ],
        "meshes":[{"weights":[0.5],"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"mode":5,"targets":[{"POSITION":2}]}]}],
        "nodes":[{"mesh":0,"scale":[0.5,0.5,0.5]}],"scenes":[{"nodes":[0]}]
    });
    (root, bin)
}

#[test]
fn quantized_strips_and_sparse_morphs_decode_to_shared_meshes() {
    let (root, bin) = fixture();
    let model = admit_glb(&pack(&root, &bin)).unwrap();
    assert_eq!(model.indices(), &[0, 1, 2, 2, 1, 3]);
    assert_eq!(model.vertex_count(), 4);
    assert_eq!(model.vertices()[0].position, [-1.0, -1.0, 0.0]);
    assert_eq!(model.vertices()[3].normal, [0.0, 0.0, 1.0]);
    assert_eq!(model.meshes()[0].morph_target_count, 1);
    let mut fan = root.clone();
    fan["meshes"][0]["primitives"][0]["mode"] = json!(6);
    assert_eq!(
        admit_glb(&pack(&fan, &bin)).unwrap().indices(),
        &[0, 1, 2, 0, 2, 3]
    );
    let mut normalized = root.clone();
    normalized["accessors"][0]["normalized"] = json!(true);
    let mut signed = bin.clone();
    signed[0..2].copy_from_slice(&i16::MIN.to_le_bytes());
    assert_eq!(
        admit_glb(&pack(&normalized, &signed)).unwrap().vertices()[0].position[0],
        -1.0
    );
}

#[test]
fn sparse_positions_replace_interleaved_base_and_initialize_missing_base_to_zero() {
    let (mut root, bin) = fixture();
    root["accessors"][0] = json!({"componentType":5120,"normalized":true,"type":"VEC3","count":4,"sparse":{"count":2,"indices":{"bufferView":2,"componentType":5121},"values":{"bufferView":3}}});
    let model = admit_glb(&pack(&root, &bin)).unwrap();
    assert_eq!(model.vertices()[0].position, [0.0; 3]);
    assert_eq!(model.vertices()[1].position, [0.0, 0.0, 1.0]);
    assert_eq!(model.vertices()[2].position, [0.0; 3]);
    assert!((model.vertices()[3].position[2] - 64.0 / 127.0).abs() < 1e-7);
    root["accessors"][0]["bufferView"] = json!(1);
    let model = admit_glb(&pack(&root, &bin)).unwrap();
    assert_eq!(model.vertices()[0].position, [0.0, 0.0, 1.0]);
    assert_eq!(model.vertices()[2].position, [0.0, 0.0, 1.0]);
}

#[test]
fn sparse_and_quantized_malformed_inputs_fail_before_rasterization() {
    let (root, bin) = fixture();
    for (pointer, value) in [
        ("/extensionsRequired", json!([])),
        ("/accessors/1/normalized", json!(false)),
        ("/accessors/1/componentType", json!(5121)),
        ("/accessors/2/sparse/count", json!(0)),
        ("/accessors/2/sparse/count", json!(5)),
        ("/accessors/2/sparse/indices/componentType", json!(5122)),
        ("/accessors/2/sparse/values/byteOffset", json!(1)),
        ("/accessors/2/byteOffset", json!(4)),
        ("/bufferViews/3/byteLength", json!(5)),
        ("/bufferViews/3/target", json!(34962)),
        ("/bufferViews/2/byteStride", json!(4)),
        ("/meshes/0/primitives/0/mode", json!(1)),
    ] {
        let mut bad = root.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        bad.pointer_mut(parent).unwrap()[key] = value;
        assert!(admit_glb(&pack(&bad, &bin)).is_err(), "{pointer}: {bad}");
    }
    for indices in [[1, 1], [3, 1], [1, 4]] {
        let mut bad = bin.clone();
        bad[48..50].copy_from_slice(&indices);
        assert!(admit_glb(&pack(&root, &bad)).is_err(), "{indices:?}");
    }
}
