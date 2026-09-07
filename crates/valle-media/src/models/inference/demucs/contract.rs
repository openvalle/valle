use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route, TensorSpec};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::demucs::{
    ADAPTER, CHANNELS, CONTRACT_VERSION, SAMPLE_RATE, SEGMENT_SAMPLES, STRIDE_SAMPLES,
};

#[derive(Debug, Deserialize)]
struct Preprocess {
    sample_rate: u32,
    channels: u16,
    segment_samples: usize,
    stride_samples: usize,
    overlap: f64,
    last_segment: String,
    normalization_reference: String,
    normalization_mean: String,
    normalization_std: String,
    overlap_window: String,
    long_audio: String,
}

#[derive(Debug, Deserialize)]
struct Postprocess {
    stem_order: Vec<String>,
    instrumental_definition: String,
    denormalization: Denormalization,
    long_audio: String,
    output_length: String,
}

#[derive(Debug, Deserialize)]
struct Denormalization {
    vocals: String,
    instrumental: String,
}

#[derive(Debug, Deserialize)]
struct OnnxMetadata {
    opset: u32,
    native_fft: bool,
    minimum_onnxruntime: String,
    output_stems: Vec<String>,
    internal_sources: Vec<String>,
    segment_samples: usize,
    sample_rate: u32,
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
    ensure!(manifest.model.id == "demucs", "manifest is not Demucs");
    ensure!(
        manifest.model.task == "audio-source-separation",
        "Demucs manifest has unsupported task {:?}",
        manifest.model.task
    );
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported Demucs adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 1,
        "Demucs contract must expose exactly one input and one output"
    );
    validate_input(&manifest.contract.inputs[0])?;
    validate_output(&manifest.contract.outputs[0])?;

    let preprocess: Preprocess = serde_json::from_value(manifest.contract.preprocess.clone())
        .context("Demucs preprocess contract is invalid")?;
    ensure!(
        preprocess.sample_rate == SAMPLE_RATE && preprocess.channels == CHANNELS,
        "Demucs requires 44.1 kHz stereo input"
    );
    ensure!(
        preprocess.segment_samples == SEGMENT_SAMPLES
            && preprocess.stride_samples == STRIDE_SAMPLES
            && preprocess.overlap == 0.25,
        "Demucs segment/stride/overlap contract drifted"
    );
    ensure!(
        preprocess.last_segment == "right-zero-pad",
        "Demucs last segment must be right-zero-padded"
    );
    ensure!(
        preprocess.normalization_reference
            == "mono reference ref[t] = (left[t] + right[t]) / 2 over the decoded whole track"
            && preprocess.normalization_mean == "mean = sum(ref) / T"
            && preprocess.normalization_std == "std = sqrt(sum((ref - mean)^2) / (T - 1)) + 1e-8",
        "Demucs whole-track normalization contract drifted"
    );
    ensure!(
        preprocess.overlap_window
            == "w[i] = (i + 1) / 171990 for 0 <= i < 171990; w[i] = (343980 - i) / 171990 for 171990 <= i < 343980"
            && preprocess.long_audio == "triangular weighted overlap-add with transition_power=1",
        "Demucs overlap-add contract drifted"
    );

    let postprocess: Postprocess = serde_json::from_value(manifest.contract.postprocess.clone())
        .context("Demucs postprocess contract is invalid")?;
    ensure!(
        postprocess.stem_order == ["vocals", "instrumental"]
            && postprocess.instrumental_definition == "sum(drums,bass,other)",
        "Demucs stem order or instrumental definition drifted"
    );
    ensure!(
        postprocess.denormalization.vocals == "output * std + mean"
            && postprocess.denormalization.instrumental == "output * std + 3 * mean",
        "Demucs denormalization contract drifted"
    );
    ensure!(
        postprocess.long_audio == "divide the weighted sum by accumulated triangular weights"
            && postprocess.output_length
                == "crop each reconstructed stem to the original decoded sample count",
        "Demucs long-audio postprocess contract drifted"
    );

    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    ensure!(
        route.backend == Backend::OnnxCpu,
        "Demucs ONNX session requires an onnx-cpu route"
    );
    ensure!(
        artifact.format == "onnx",
        "ONNX route needs an ONNX artifact"
    );
    ensure!(
        artifact.precision == "fp32",
        "Demucs ONNX artifact must be fp32"
    );
    let metadata: OnnxMetadata = serde_json::from_value(artifact.metadata.clone())
        .context("Demucs ONNX artifact metadata is invalid")?;
    ensure!(
        metadata.opset == 18 && metadata.native_fft && metadata.minimum_onnxruntime == "1.28.0",
        "Demucs artifact requires native FFT ops and ONNX Runtime 1.28"
    );
    ensure!(
        metadata.output_stems == ["vocals", "instrumental"]
            && metadata.internal_sources == ["drums", "bass", "other", "vocals"],
        "Demucs artifact stem metadata drifted"
    );
    ensure!(
        metadata.segment_samples == SEGMENT_SAMPLES && metadata.sample_rate == SAMPLE_RATE,
        "Demucs artifact geometry metadata drifted"
    );

    Ok(ContractNames {
        input: manifest.contract.inputs[0].name.clone(),
        output: manifest.contract.outputs[0].name.clone(),
    })
}

fn validate_input(input: &TensorSpec) -> Result<()> {
    ensure!(
        input.name == "mix"
            && input.dtype == "f32"
            && input.layout == "nct"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(2),
                    Dimension::Fixed(SEGMENT_SAMPLES as u64),
                ]
            && input.range.is_none(),
        "Demucs input must be mix f32 NCT [1,2,{SEGMENT_SAMPLES}]"
    );
    Ok(())
}

fn validate_output(output: &TensorSpec) -> Result<()> {
    ensure!(
        output.name == "stems"
            && output.dtype == "f32"
            && output.layout == "nsct"
            && output.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(2),
                    Dimension::Fixed(2),
                    Dimension::Fixed(SEGMENT_SAMPLES as u64),
                ]
            && output.range.is_none(),
        "Demucs output must be stems f32 NSCT [1,2,2,{SEGMENT_SAMPLES}]"
    );
    Ok(())
}
