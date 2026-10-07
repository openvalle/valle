#![cfg(all(feature = "model-coreml", target_os = "macos"))]

use valle_media::models::runtime::{
    TensorInput,
    coreml::{CoreMlComputeUnits, CoreMlSession},
};

#[test]
fn coreml_native_arithmetic_and_cache_roundtrip_preserve_shape_and_values() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("affine.mlpackage");
    std::fs::create_dir_all(source.join("Data/com.apple.CoreML")).unwrap();
    std::fs::write(
        source.join("Manifest.json"),
        include_bytes!("fixtures/models/affine.mlpackage/Manifest.json"),
    )
    .unwrap();
    std::fs::write(
        source.join("Data/com.apple.CoreML/model.mlmodel"),
        include_bytes!("fixtures/models/affine.mlpackage/Data/com.apple.CoreML/model.mlmodel"),
    )
    .unwrap();
    let cache = root.path().join("cache");
    let session = CoreMlSession::load(
        &source,
        &cache,
        "affine-contract-v1",
        CoreMlComputeUnits::CpuOnly,
    )
    .unwrap();
    let path = session.compiled_path().to_path_buf();
    let input = TensorInput::owned("x", [1, 2, 2], vec![-2.0, 0.0, 0.5, 3.0]);
    let output = session.run_f32(&[input.clone()], &["y"]).unwrap();
    assert_eq!(output[0].shape, [1, 2, 2]);
    assert_eq!(output[0].data, [-3.0, 1.0, 2.0, 7.0]);
    assert!(
        session
            .run_f32(&[input], &["missing"])
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    assert!(
        session
            .run_f32(&[TensorInput::owned("x", [1, 2, 2], vec![0.0])], &["y"])
            .unwrap_err()
            .to_string()
            .contains("requires 4")
    );
    drop(session);
    let reused = CoreMlSession::load(
        &source,
        &cache,
        "affine-contract-v1",
        CoreMlComputeUnits::CpuOnly,
    )
    .unwrap();
    assert_eq!(reused.compiled_path(), path);
    let compiled =
        CoreMlSession::load(&path, &cache, "unused", CoreMlComputeUnits::CpuOnly).unwrap();
    assert_eq!(compiled.compiled_path(), path);
}
