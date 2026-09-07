use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::omnishotcut::{
    MODEL_HEIGHT, MODEL_WIDTH, OVERLAP_FRAMES, QUERY_COUNT, WINDOW_FRAMES,
};

pub const ADAPTER: &str = "omnishotcut-shot-detection";
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractNames {
    pub input: String,
    pub intra: String,
    pub inter: String,
    pub range: String,
}

#[derive(Debug, Deserialize)]
struct PreprocessContract {
    window_frames: usize,
    overlap_frames: usize,
    resize: [usize; 2],
    color_space: String,
}

#[derive(Debug, Deserialize)]
struct PostprocessContract {
    range_end: String,
    overlap_merge: String,
}

#[derive(Debug, Deserialize)]
struct OnnxMetadata {
    opset: u32,
    window_frames: usize,
    overlap_frames: usize,
}

/// Validate the canonical release, selected route and selected artifact as one boundary.
pub fn validate_release_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
) -> Result<()> {
    validate_contract(manifest, route, artifact).map(|_| ())
}

pub(crate) fn validate_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
) -> Result<ContractNames> {
    manifest.validate()?;
    ensure!(
        manifest.model.id == "omnishotcut" && manifest.model.task == "shot-boundary-detection",
        "manifest is not the OmniShotCut shot-boundary release"
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported OmniShotCut adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 3,
        "OmniShotCut contract must expose one input and three outputs"
    );

    let input = &manifest.contract.inputs[0];
    ensure!(
        input.name == "frames"
            && input.dtype == "u8"
            && input.layout == "thwc"
            && input.shape
                == vec![
                    Dimension::Fixed(WINDOW_FRAMES as u64),
                    Dimension::Fixed(MODEL_HEIGHT as u64),
                    Dimension::Fixed(MODEL_WIDTH as u64),
                    Dimension::Fixed(3),
                ]
            && input.range == Some([0.0, 255.0]),
        "OmniShotCut input must be frames u8 THWC [100,96,128,3] in [0,255]"
    );

    for (output, name) in
        manifest
            .contract
            .outputs
            .iter()
            .zip(["intra_idx", "inter_idx", "range_idx"])
    {
        ensure!(
            output.name == name
                && output.dtype == "i64"
                && output.layout == "q"
                && output.shape == vec![Dimension::Fixed(QUERY_COUNT as u64)],
            "OmniShotCut output {name:?} must be i64 Q [24]"
        );
    }

    let preprocess: PreprocessContract =
        serde_json::from_value(manifest.contract.preprocess.clone())
            .context("OmniShotCut preprocess contract is invalid")?;
    ensure!(
        preprocess.window_frames == WINDOW_FRAMES
            && preprocess.overlap_frames == OVERLAP_FRAMES
            && preprocess.resize == [MODEL_WIDTH, MODEL_HEIGHT]
            && preprocess.color_space == "rgb",
        "OmniShotCut preprocess contract does not match the rolling adapter"
    );
    let postprocess: PostprocessContract =
        serde_json::from_value(manifest.contract.postprocess.clone())
            .context("OmniShotCut postprocess contract is invalid")?;
    ensure!(
        postprocess.range_end == "exclusive" && postprocess.overlap_merge == "model-contract",
        "OmniShotCut postprocess contract does not match the rolling adapter"
    );

    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "OmniShotCut session requires an onnx-cpu route"
    );
    ensure!(
        artifact.format == "onnx"
            && matches!(artifact.precision.as_str(), "fp32" | "int8-conv-fp32-rest"),
        "OmniShotCut requires a supported ONNX artifact"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("OmniShotCut ONNX metadata is invalid")?;
    ensure!(
        metadata.opset == 17
            && metadata.window_frames == WINDOW_FRAMES
            && metadata.overlap_frames == OVERLAP_FRAMES,
        "OmniShotCut ONNX metadata does not match the rolling adapter"
    );

    Ok(ContractNames {
        input: input.name.clone(),
        intra: manifest.contract.outputs[0].name.clone(),
        inter: manifest.contract.outputs[1].name.clone(),
        range: manifest.contract.outputs[2].name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-omnishotcut-1.0.0.json")).unwrap()
    }

    #[test]
    fn checked_in_release_matches_both_onnx_artifacts() {
        let manifest = release();
        for route_id in ["onnx-cpu-macos-arm64", "onnx-fp32-macos-arm64"] {
            let route = manifest
                .routes
                .iter()
                .find(|route| route.id == route_id)
                .unwrap();
            let artifact = manifest
                .artifacts
                .iter()
                .find(|artifact| artifact.id == route.artifact)
                .unwrap();
            let names = validate_contract(&manifest, route, artifact).unwrap();
            assert_eq!(names.input, "frames");
            assert_eq!(names.range, "range_idx");
        }
    }

    #[test]
    fn validation_rejects_adapter_tensor_and_route_drift() {
        let manifest = release();
        let route = manifest
            .routes
            .iter()
            .find(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        assert!(validate_contract(&manifest, route, artifact).is_ok());

        let mut changed = manifest.clone();
        changed.contract.adapter = "wrong-adapter".into();
        let route = changed
            .routes
            .iter()
            .find(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        let artifact = changed
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        assert!(validate_contract(&changed, route, artifact).is_err());

        let mut changed = manifest.clone();
        changed.contract.outputs[0].dtype = "i32".into();
        let route = changed
            .routes
            .iter()
            .find(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        let artifact = changed
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        assert!(validate_contract(&changed, route, artifact).is_err());

        let mut changed = manifest;
        let route_index = changed
            .routes
            .iter()
            .position(|route| route.id == "onnx-cpu-macos-arm64")
            .unwrap();
        changed.routes[route_index].backend = Backend::Coreml;
        let route = &changed.routes[route_index];
        let artifact = changed
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        assert!(validate_contract(&changed, route, artifact).is_err());
    }
}
