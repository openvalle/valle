use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route, TensorSpec};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::realesrgan::{
    ADAPTER, CONTRACT_VERSION, DEFAULT_TILE_HEIGHT, DEFAULT_TILE_WIDTH, SCALE,
};

#[derive(Debug, Deserialize)]
struct Preprocess {
    color_space: String,
    denoise_strength: f64,
    /// Stored as `[height, width]`, matching the NCHW spatial-axis order.
    coreml_tile: [u32; 2],
    overlap_tiles: bool,
}

#[derive(Debug, Deserialize)]
struct Postprocess {
    clamp: [f64; 2],
    scale: u32,
    preserve_aspect_ratio: bool,
}

#[derive(Debug, Deserialize)]
struct OnnxMetadata {
    opset: u32,
    dynamic_shape: bool,
    scale: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ContractNames {
    pub input: String,
    pub output: String,
}

pub(crate) fn validate_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
) -> Result<ContractNames> {
    manifest.validate()?;
    ensure!(
        manifest.model.id == "realesrgan",
        "manifest is not Real-ESRGAN"
    );
    ensure!(
        manifest.model.task == "image-super-resolution",
        "Real-ESRGAN manifest has unsupported task {:?}",
        manifest.model.task
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported Real-ESRGAN adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 1,
        "Real-ESRGAN contract must expose exactly one input and one output"
    );
    validate_input(&manifest.contract.inputs[0])?;
    validate_output(&manifest.contract.outputs[0])?;

    let preprocess: Preprocess = serde_json::from_value(manifest.contract.preprocess.clone())
        .context("Real-ESRGAN preprocess contract is invalid")?;
    ensure!(
        preprocess.color_space == "rgb",
        "Real-ESRGAN requires RGB input"
    );
    ensure!(
        preprocess.denoise_strength == 0.0,
        "this adapter only supports denoise_strength=0"
    );
    ensure!(
        preprocess.coreml_tile == [DEFAULT_TILE_HEIGHT, DEFAULT_TILE_WIDTH],
        "Real-ESRGAN coreml_tile must be [height,width] = [{DEFAULT_TILE_HEIGHT},{DEFAULT_TILE_WIDTH}]"
    );
    ensure!(
        preprocess.overlap_tiles,
        "Real-ESRGAN contract must enable overlapping tiles"
    );

    let postprocess: Postprocess = serde_json::from_value(manifest.contract.postprocess.clone())
        .context("Real-ESRGAN postprocess contract is invalid")?;
    ensure!(
        postprocess.clamp == [0.0, 1.0],
        "Real-ESRGAN output must clamp to [0,1]"
    );
    ensure!(
        postprocess.scale == SCALE,
        "Real-ESRGAN output scale must be {SCALE}"
    );
    ensure!(
        postprocess.preserve_aspect_ratio,
        "Real-ESRGAN adapter requires aspect-ratio preservation"
    );

    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "Real-ESRGAN ONNX session requires an onnx-cpu route"
    );
    ensure!(
        artifact.format == "onnx",
        "ONNX route needs an ONNX artifact"
    );
    ensure!(
        artifact.precision == "fp32",
        "Real-ESRGAN ONNX artifact must be fp32"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("Real-ESRGAN ONNX artifact metadata is invalid")?;
    ensure!(
        metadata.opset == 17,
        "Real-ESRGAN ONNX artifact must use opset 17"
    );
    ensure!(
        metadata.dynamic_shape,
        "Real-ESRGAN ONNX artifact must support dynamic shapes"
    );
    ensure!(
        metadata.scale == SCALE,
        "Real-ESRGAN ONNX metadata scale must be {SCALE}"
    );

    Ok(ContractNames {
        input: manifest.contract.inputs[0].name.clone(),
        output: manifest.contract.outputs[0].name.clone(),
    })
}

fn validate_input(input: &TensorSpec) -> Result<()> {
    ensure!(
        input.name == "lr"
            && input.dtype == "f32"
            && input.layout == "nchw"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(3),
                    Dimension::Symbol("height".into()),
                    Dimension::Symbol("width".into()),
                ]
            && input.range == Some([0.0, 1.0]),
        "Real-ESRGAN input must be lr f32 NCHW [1,3,height,width] in [0,1]"
    );
    Ok(())
}

fn validate_output(output: &TensorSpec) -> Result<()> {
    ensure!(
        output.name == "sr"
            && output.dtype == "f32"
            && output.layout == "nchw"
            && output.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(3),
                    Dimension::Symbol("height4".into()),
                    Dimension::Symbol("width4".into()),
                ]
            && output.range == Some([0.0, 1.0]),
        "Real-ESRGAN output must be sr f32 NCHW [1,3,height4,width4] in [0,1]"
    );
    Ok(())
}
