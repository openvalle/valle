use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route, TensorSpec};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::rife::{ADAPTER, CONTRACT_VERSION, PAD_MULTIPLE};

#[derive(Debug, Deserialize)]
struct Preprocess {
    color_space: String,
    pad_dimension_multiple: u32,
    pad_edges: Vec<String>,
    resize: bool,
    concatenate: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Postprocess {
    crop_to_source: bool,
    clamp: [f64; 2],
    forbid_cross_shot: bool,
}

#[derive(Debug, Deserialize)]
struct OnnxMetadata {
    opset: u32,
    dynamic_shape: bool,
    dimension_multiple: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ContractNames {
    pub frames: String,
    pub timestep: String,
    pub output: String,
}

pub(crate) fn validate_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
) -> Result<ContractNames> {
    manifest.validate()?;
    ensure!(manifest.model.id == "rife", "manifest is not RIFE");
    ensure!(
        manifest.model.task == "frame-interpolation",
        "RIFE manifest has unsupported task {:?}",
        manifest.model.task
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported RIFE adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 2 && manifest.contract.outputs.len() == 1,
        "RIFE contract must expose exactly two inputs and one output"
    );
    validate_frames_input(&manifest.contract.inputs[0])?;
    validate_timestep_input(&manifest.contract.inputs[1])?;
    validate_output(&manifest.contract.outputs[0])?;

    let preprocess: Preprocess = serde_json::from_value(manifest.contract.preprocess.clone())
        .context("RIFE preprocess contract is invalid")?;
    ensure!(preprocess.color_space == "rgb", "RIFE requires RGB input");
    ensure!(
        preprocess.pad_dimension_multiple == PAD_MULTIPLE,
        "RIFE dimensions must be padded to a multiple of {PAD_MULTIPLE}"
    );
    ensure!(
        preprocess.pad_edges == ["right", "bottom"],
        "RIFE only supports zero-padding the right and bottom edges"
    );
    ensure!(
        !preprocess.resize,
        "RIFE host preprocessing must not resize frames"
    );
    ensure!(
        preprocess.concatenate == ["frame0", "frame1"],
        "RIFE frame concatenation order must be frame0 then frame1"
    );

    let postprocess: Postprocess = serde_json::from_value(manifest.contract.postprocess.clone())
        .context("RIFE postprocess contract is invalid")?;
    ensure!(
        postprocess.crop_to_source,
        "RIFE output must crop to source geometry"
    );
    ensure!(
        postprocess.clamp == [0.0, 1.0],
        "RIFE output must clamp to [0,1]"
    );
    ensure!(
        postprocess.forbid_cross_shot,
        "RIFE callers must forbid interpolation across shot boundaries"
    );

    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "RIFE ONNX session requires an onnx-cpu route"
    );
    ensure!(
        artifact.format == "onnx",
        "ONNX route needs an ONNX artifact"
    );
    ensure!(
        artifact.precision == "fp32",
        "RIFE ONNX artifact must be fp32"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("RIFE ONNX artifact metadata is invalid")?;
    ensure!(metadata.opset == 17, "RIFE ONNX artifact must use opset 17");
    ensure!(
        metadata.dynamic_shape,
        "RIFE ONNX artifact must support dynamic shapes"
    );
    ensure!(
        metadata.dimension_multiple == PAD_MULTIPLE,
        "RIFE ONNX artifact dimension_multiple must be {PAD_MULTIPLE}"
    );

    Ok(ContractNames {
        frames: manifest.contract.inputs[0].name.clone(),
        timestep: manifest.contract.inputs[1].name.clone(),
        output: manifest.contract.outputs[0].name.clone(),
    })
}

fn validate_frames_input(input: &TensorSpec) -> Result<()> {
    ensure!(
        input.name == "x"
            && input.dtype == "f32"
            && input.layout == "nchw"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(6),
                    Dimension::Symbol("height".into()),
                    Dimension::Symbol("width".into()),
                ]
            && input.range == Some([0.0, 1.0]),
        "RIFE x input must be f32 NCHW [1,6,height,width] in [0,1]"
    );
    Ok(())
}

fn validate_timestep_input(input: &TensorSpec) -> Result<()> {
    ensure!(
        input.name == "t"
            && input.dtype == "f32"
            && input.layout == "nchw"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(1),
                    Dimension::Fixed(1),
                    Dimension::Fixed(1),
                ]
            && input.range == Some([0.0, 1.0]),
        "RIFE t input must be f32 NCHW [1,1,1,1] in [0,1]"
    );
    Ok(())
}

fn validate_output(output: &TensorSpec) -> Result<()> {
    ensure!(
        output.name == "mid"
            && output.dtype == "f32"
            && output.layout == "nchw"
            && output.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(3),
                    Dimension::Symbol("height".into()),
                    Dimension::Symbol("width".into()),
                ]
            && output.range == Some([0.0, 1.0]),
        "RIFE mid output must be f32 NCHW [1,3,height,width] in [0,1]"
    );
    Ok(())
}
