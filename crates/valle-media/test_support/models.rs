use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const VERSION: &str = "1.0.1";
pub const REVISION: &str = "1111111111111111111111111111111111111111";

fn manifest(id: &str) -> Value {
    let text = match id {
        "lama" => include_str!("../src/models/catalog/release-lama-1.0.0.json"),
        "realesrgan" => include_str!("../src/models/catalog/release-realesrgan-1.0.0.json"),
        "rife" => include_str!("../src/models/catalog/release-rife-1.0.0.json"),
        "demucs" => include_str!("../src/models/catalog/release-demucs-1.0.0.json"),
        "dpdfnet" => include_str!("../src/models/catalog/release-dpdfnet-1.0.0.json"),
        "birefnet" => include_str!("../src/models/catalog/release-birefnet-1.0.0.json"),
        "modnet" => include_str!("../src/models/catalog/release-modnet-1.0.0.json"),
        "transnetv2" => include_str!("../src/models/catalog/release-transnetv2-1.0.0.json"),
        "omnishotcut" => include_str!("../src/models/catalog/release-omnishotcut-1.0.0.json"),
        _ => panic!("no contract graph for {id}"),
    };
    serde_json::from_str(text).unwrap()
}

fn graph(id: &str, path: &str) -> &'static [u8] {
    match (id, path) {
        ("lama", "lama_256x256.onnx") => {
            include_bytes!("../tests/fixtures/models/lama_256x256.onnx")
        }
        ("lama", "lama_640x384.onnx") => {
            include_bytes!("../tests/fixtures/models/lama_640x384.onnx")
        }
        ("realesrgan", _) => include_bytes!("../tests/fixtures/models/realesrgan.onnx"),
        ("rife", _) => include_bytes!("../tests/fixtures/models/rife.onnx"),
        ("demucs", _) => include_bytes!("../tests/fixtures/models/demucs.onnx"),
        ("dpdfnet", _) => include_bytes!("../tests/fixtures/models/dpdfnet.onnx"),
        ("birefnet", _) => include_bytes!("../tests/fixtures/models/birefnet.onnx"),
        ("modnet", _) => include_bytes!("../tests/fixtures/models/modnet.onnx"),
        ("transnetv2", _) => include_bytes!("../tests/fixtures/models/transnetv2.onnx"),
        ("omnishotcut", _) => include_bytes!("../tests/fixtures/models/omnishotcut.onnx"),
        _ => panic!("no graph for {id}/{path}"),
    }
}

/// An isolated, explicitly selected test release with genuine file hashes and install locks.
/// It goes through the normal offline catalog/store/manager verification. Pinned releases
/// are never replaced, and these graphs cannot be mistaken for published learned weights.
pub fn install(models_root: &Path, id: &str) -> PathBuf {
    let mut manifest = manifest(id);
    manifest["model"]["version"] = json!(VERSION);
    let mut route = manifest["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["backend"] == "onnx-cpu")
        .unwrap()
        .clone();
    // This isolated graph release tests the adapter on the current host. It does not change
    // the published weights' verification status for other platforms or architectures.
    route["id"] = json!("contract-onnx-cpu");
    route["platforms"] = json!([std::env::consts::OS]);
    route["architectures"] = json!([std::env::consts::ARCH]);
    route["status"] = json!("verified");
    route["requirements"]["minimum_os"] = Value::Null;
    manifest["routes"] = json!([route]);
    let artifact_id = manifest["routes"][0]["artifact"]
        .as_str()
        .unwrap()
        .to_owned();
    let root = models_root
        .join(id)
        .join(VERSION)
        .join(REVISION)
        .join(&artifact_id);
    fs::create_dir_all(&root).unwrap();
    let artifact = manifest["artifacts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|artifact| artifact["id"] == artifact_id)
        .unwrap();
    for file in artifact["files"].as_array_mut().unwrap() {
        let path = file["path"].as_str().unwrap();
        let bytes = graph(id, path);
        let target = root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
        file["bytes"] = json!(bytes.len());
        file["sha256"] = json!(hex::encode(Sha256::digest(bytes)));
    }
    let artifact_files = artifact["files"].clone();
    for file in manifest["license"]["files"].as_array_mut().unwrap() {
        let path = root.join(file["path"].as_str().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = b"Valle deterministic contract graph test fixture\n";
        fs::write(path, bytes).unwrap();
        file["bytes"] = json!(bytes.len());
        file["sha256"] = json!(hex::encode(Sha256::digest(bytes)));
    }
    let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(root.join("release.v1.json"), &bytes).unwrap();
    let repository = manifest["release"]["repository"].clone();
    let manifest_path = format!("{id}/release.v1.json");
    let lock = json!({
        "schema_version": 1, "model": id, "version": VERSION, "repository": repository,
        "revision": REVISION, "manifest_path": manifest_path, "manifest_sha256": hex::encode(Sha256::digest(&bytes)),
        "artifact": artifact_id, "license_files": manifest["license"]["files"], "files": artifact_files,
    });
    fs::write(
        root.join("install.lock.json"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let release = json!({"version": VERSION, "repository": repository, "revision": REVISION, "manifest_path": manifest_path});
    let catalog = json!({"schema_version": 1, "generated_by": "valle-contract-tests", "models": [{
        "id": id, "display_name": manifest["model"]["display_name"], "task": manifest["model"]["task"],
        "latest": VERSION, "releases": [release],
    }]});
    let catalogs = models_root.join(".metadata/catalogs");
    fs::create_dir_all(&catalogs).unwrap();
    fs::write(
        catalogs.join(format!("{id}-contract.json")),
        serde_json::to_vec_pretty(&catalog).unwrap(),
    )
    .unwrap();
    let release_cache = models_root
        .join(".metadata/releases")
        .join(id)
        .join(VERSION)
        .join(REVISION);
    fs::create_dir_all(&release_cache).unwrap();
    fs::write(release_cache.join("release.v1.json"), bytes).unwrap();
    root
}
