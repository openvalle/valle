use std::collections::BTreeSet;
use std::path::Path;

use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route, TensorSpec};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::edgetam::{ADAPTER, CANVAS_SIDE, CONTRACT_VERSION};

const REQUIRED_FILES: [&str; 11] = [
    "image_encoder.onnx",
    "memattn.onnx",
    "decoder_p3.onnx",
    "decoder_p1.onnx",
    "memencode.onnx",
    "const_curr_pos.npy",
    "const_maskmem_pos.npy",
    "const_maskmem_tpos.npy",
    "const_no_mem_embed.npy",
    "const_no_obj_ptr.npy",
    "prompt_protocol.v1.json",
];

#[derive(Debug, Deserialize)]
struct Preprocess {
    color_space: String,
    canvas: [usize; 2],
    prompt_protocol: String,
}

#[derive(Debug, Deserialize)]
struct Postprocess {
    stateful: bool,
    host_contract: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OnnxMetadata {
    opset: u32,
    components: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PromptProtocol {
    schema_version: u32,
    decoder: String,
    seed_frame: u32,
    coordinate_space: String,
    point_count: usize,
    labels: serde_json::Value,
    selection_rule: Vec<String>,
}

pub(crate) fn validate_contract(
    manifest: &ModelManifest,
    route: &Route,
    artifact: &Artifact,
    artifact_root: &Path,
) -> Result<()> {
    manifest.validate()?;
    ensure!(manifest.model.id == "edgetam", "manifest is not EdgeTAM");
    ensure!(
        manifest.model.version == "1.0.0",
        "unsupported EdgeTAM release {}",
        manifest.model.version
    );
    ensure!(
        manifest.model.task == "video-object-segmentation",
        "EdgeTAM manifest has unsupported task {:?}",
        manifest.model.task
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported EdgeTAM adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 3 && manifest.contract.outputs.len() == 1,
        "EdgeTAM contract must expose image/coords/labels and one mask"
    );
    validate_tensor(
        &manifest.contract.inputs[0],
        "image",
        "f32",
        "nchw",
        &[1, 3, CANVAS_SIDE as u64, CANVAS_SIDE as u64],
    )?;
    validate_tensor(
        &manifest.contract.inputs[1],
        "coords",
        "f32",
        "npc",
        &[1, 3, 2],
    )?;
    validate_tensor(&manifest.contract.inputs[2], "labels", "i32", "np", &[1, 3])?;
    validate_tensor(
        &manifest.contract.outputs[0],
        "mask",
        "f32",
        "nchw",
        &[1, 1, CANVAS_SIDE as u64, CANVAS_SIDE as u64],
    )?;

    let preprocess: Preprocess = serde_json::from_value(manifest.contract.preprocess.clone())
        .context("EdgeTAM preprocess contract is invalid")?;
    ensure!(
        preprocess.color_space == "rgb"
            && preprocess.canvas == [CANVAS_SIDE, CANVAS_SIDE]
            && preprocess.prompt_protocol == "prompt_protocol.v1.json",
        "EdgeTAM preprocessing contract drifted"
    );
    let postprocess: Postprocess = serde_json::from_value(manifest.contract.postprocess.clone())
        .context("EdgeTAM postprocess contract is invalid")?;
    ensure!(
        postprocess.stateful,
        "EdgeTAM contract must remain stateful"
    );
    ensure!(
        postprocess.host_contract
            == [
                "memory-bank",
                "pointer-queue",
                "static-kv-slots",
                "mask-selection",
                "resize-to-source",
            ],
        "EdgeTAM host contract drifted"
    );

    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "EdgeTAM ONNX adapter requires an onnx-cpu route"
    );
    ensure!(
        route.requirements.minimum_runtime.as_deref() == Some("ONNX Runtime 1.28.0"),
        "EdgeTAM ONNX route must require ONNX Runtime 1.28.0"
    );
    ensure!(
        route
            .options
            .get("bounded_prefetch")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
            && route
                .options
                .get("borrow_inputs")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && route
                .options
                .get("cache_full_memory_constants")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
        "EdgeTAM ONNX route options drifted"
    );
    ensure!(
        artifact.id == "onnx-fp32-pipeline"
            && artifact.format == "onnx"
            && artifact.precision == "fp32"
            && artifact.entrypoint == "image_encoder.onnx",
        "EdgeTAM ONNX artifact identity drifted"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("EdgeTAM ONNX metadata is invalid")?;
    ensure!(
        metadata.opset == 17
            && metadata.components
                == [
                    "image_encoder",
                    "memattn",
                    "decoder_p3",
                    "decoder_p1",
                    "memencode",
                ],
        "EdgeTAM ONNX component contract drifted"
    );
    let declared = artifact
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for required in REQUIRED_FILES {
        ensure!(
            declared.contains(required),
            "EdgeTAM artifact does not declare {required}"
        );
        ensure!(
            artifact_root.join(required).is_file(),
            "EdgeTAM artifact file is missing: {}",
            artifact_root.join(required).display()
        );
    }
    validate_prompt_protocol(&artifact_root.join("prompt_protocol.v1.json"))?;
    Ok(())
}

fn validate_tensor(
    tensor: &TensorSpec,
    name: &str,
    dtype: &str,
    layout: &str,
    shape: &[u64],
) -> Result<()> {
    ensure!(
        tensor.name == name
            && tensor.dtype == dtype
            && tensor.layout == layout
            && tensor.range.is_none()
            && tensor.shape
                == shape
                    .iter()
                    .copied()
                    .map(Dimension::Fixed)
                    .collect::<Vec<_>>(),
        "EdgeTAM tensor {name:?} contract drifted"
    );
    Ok(())
}

fn validate_prompt_protocol(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read EdgeTAM prompt protocol {}", path.display()))?;
    let protocol: PromptProtocol = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid EdgeTAM prompt protocol {}", path.display()))?;
    ensure!(
        protocol.schema_version == 1
            && protocol.decoder == "decoder_p3"
            && protocol.seed_frame == 0
            && protocol.coordinate_space == "original_frame_pixels_xy"
            && protocol.point_count == 3,
        "EdgeTAM prompt protocol identity drifted"
    );
    ensure!(
        protocol.labels == serde_json::json!({"0": "negative", "1": "positive"}),
        "EdgeTAM prompt labels drifted"
    );
    ensure!(
        protocol.selection_rule.len() == 3
            && protocol.selection_rule[2]
                == "Do not derive prompts from any frame after the seed frame.",
        "EdgeTAM prompt selection rules drifted"
    );
    Ok(())
}
