use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

fn temp_dir(prefix: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}_{seq}", std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_runtime() -> PathBuf {
    let root = temp_dir("valle_wrt");
    let html_path = "apps/preview/index.html";
    let js_path = "apps/preview/app.js";
    let specs: [(&str, &str, &str, &[u8], Option<&str>); 8] = [
        (
            "html",
            "html",
            html_path,
            b"<script src=\"./app.js\"></script>",
            None,
        ),
        ("js", "app", js_path, b"console.log('runtime')", None),
        (
            "engine-glue",
            "glue",
            valle_cli::webruntime::ENGINE_GLUE_PATH,
            b"engine glue",
            Some("engine-core"),
        ),
        (
            "engine-wasm",
            "wasm",
            valle_cli::webruntime::ENGINE_WASM_PATH,
            b"engine wasm",
            Some("engine-core"),
        ),
        (
            "canvas-full-glue",
            "glue",
            valle_cli::webruntime::CANVASKIT_FULL_GLUE_PATH,
            b"canvas full glue",
            Some("canvaskit-full"),
        ),
        (
            "canvas-full-wasm",
            "wasm",
            valle_cli::webruntime::CANVASKIT_FULL_WASM_PATH,
            b"canvas full wasm",
            Some("canvaskit-full"),
        ),
        (
            "font",
            "font",
            valle_cli::webruntime::DEFAULT_SANS_FONT_PATH,
            b"font",
            None,
        ),
        (
            "worker",
            "worker",
            valle_cli::webruntime::PRODUCT_FRAME_WORKER_PATH,
            b"worker",
            None,
        ),
    ];
    let mut assets = Vec::new();
    for (id, role, path, bytes, group) in specs {
        write(&root, path, bytes);
        assets.push(asset(id, role, path, bytes, group));
    }
    write(
        &root,
        "runtime/manifests/fixture.json",
        &serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "package": "fixture",
            "assets": assets.clone(),
        }))
        .unwrap(),
    );
    write(
        &root,
        valle_cli::webruntime::BUILD_MANIFEST_FILE,
        &serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "runtimeVersion": env!("CARGO_PKG_VERSION"),
            "protocolVersion": valle_cli::webruntime::PROTOCOL_VERSION,
            "packages": [{"id": "fixture", "manifest": "runtime/manifests/fixture.json"}],
            "assets": assets,
            "assetGroups": [
                {
                    "id": "engine-core",
                    "glue": valle_cli::webruntime::ENGINE_GLUE_PATH,
                    "wasm": [valle_cli::webruntime::ENGINE_WASM_PATH]
                },
                {
                    "id": "canvaskit-full",
                    "glue": valle_cli::webruntime::CANVASKIT_FULL_GLUE_PATH,
                    "wasm": [valle_cli::webruntime::CANVASKIT_FULL_WASM_PATH]
                },
            ],
            "workers": [{
                "id": "product-frame",
                "path": valle_cli::webruntime::PRODUCT_FRAME_WORKER_PATH,
            }],
            "apps": [{"id": "preview", "html": html_path}],
            "routes": {"preview.html": html_path, "app.js": js_path},
            "runtimeAssets": valle_cli::webruntime::runtime_assets_json(),
        }))
        .unwrap(),
    );
    root
}

fn asset(id: &str, role: &str, path: &str, bytes: &[u8], group: Option<&str>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": id,
        "owner": "fixture",
        "role": role,
        "path": path,
        "bytes": bytes.len(),
        "sha256": hex::encode(Sha256::digest(bytes)),
        "license": "Apache-2.0",
    });
    if let Some(group) = group {
        value["group"] = serde_json::json!(group);
    }
    value
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn explicit_development_resources_resolve_without_installation() {
    let source = fake_runtime();
    let runtime = valle_cli::webruntime::resolve(Some(&source)).unwrap();
    assert_eq!(runtime.source.as_str(), "dev-dir");
}

#[test]
fn explicit_directory_requires_a_manifest() {
    let source = temp_dir("valle-empty-runtime");
    assert!(valle_cli::webruntime::resolve(Some(&source)).is_err());
}

#[test]
fn resolving_tampered_resources_fails_closed() {
    let source = fake_runtime();
    std::fs::write(source.join("apps/preview/app.js"), b"tampered").unwrap();
    assert!(valle_cli::webruntime::resolve(Some(&source)).is_err());
}
