use std::path::Path;

use crate::models::runtime::onnx::{
    OnnxI64TensorOutput, OnnxSession, OnnxSessionOptions, OnnxTensorInput,
};
use crate::models::spec::{Artifact, ModelManifest, Route};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::models::inference::omnishotcut::contract::{ContractNames, validate_contract};
use crate::models::inference::omnishotcut::{
    MODEL_HEIGHT, MODEL_WIDTH, MODEL_WINDOW_BYTES, OmniShotCutSession, QUERY_COUNT, WINDOW_FRAMES,
    WindowPrediction, WindowPredictor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrtSessionOptions {
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

impl Default for OrtSessionOptions {
    fn default() -> Self {
        Self {
            intra_threads: None,
            memory_pattern: false,
            prepacking: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RouteOptions {
    intra_threads: Option<usize>,
    memory_pattern: Option<bool>,
    prepacking: Option<bool>,
}

/// One ONNX Runtime session reused by every rolling window and video stream.
pub struct OrtWindowPredictor {
    session: OnnxSession,
    names: ContractNames,
}

impl OrtWindowPredictor {
    pub fn open(model: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(model, OrtSessionOptions::default())
    }

    pub fn open_with_options(model: impl AsRef<Path>, options: OrtSessionOptions) -> Result<Self> {
        let names = ContractNames {
            input: "frames".into(),
            intra: "intra_idx".into(),
            inter: "inter_idx".into(),
            range: "range_idx".into(),
        };
        Self::open_named(model.as_ref(), options, names)
    }

    fn open_named(model: &Path, options: OrtSessionOptions, names: ContractNames) -> Result<Self> {
        if let Some(threads) = options.intra_threads {
            ensure!(threads > 0, "ONNX intra_threads must be positive");
        }
        let session = OnnxSession::load_with_options(
            model,
            &OnnxSessionOptions {
                intra_threads: options.intra_threads,
                memory_pattern: options.memory_pattern,
                prepacking: options.prepacking,
            },
        )
        .with_context(|| format!("load OmniShotCut model {}", model.display()))?;
        Ok(Self { session, names })
    }
}

impl WindowPredictor for OrtWindowPredictor {
    fn predict_window(&mut self, frames: &[u8]) -> Result<WindowPrediction> {
        ensure!(
            frames.len() == MODEL_WINDOW_BYTES,
            "OmniShotCut window has {} bytes; expected {MODEL_WINDOW_BYTES}",
            frames.len()
        );
        let output_names = [
            self.names.intra.as_str(),
            self.names.inter.as_str(),
            self.names.range.as_str(),
        ];
        let outputs = self
            .session
            .run_typed_i64(
                &[OnnxTensorInput::borrowed_u8(
                    &self.names.input,
                    [WINDOW_FRAMES, MODEL_HEIGHT, MODEL_WIDTH, 3],
                    frames,
                )],
                &output_names,
            )
            .context("run OmniShotCut ONNX inference")?;
        let [intra, inter, ranges]: [OnnxI64TensorOutput; 3] =
            outputs.try_into().map_err(|outputs: Vec<_>| {
                anyhow::anyhow!("OmniShotCut returned {} outputs", outputs.len())
            })?;
        for output in [&intra, &inter, &ranges] {
            ensure!(
                output.shape == [QUERY_COUNT],
                "OmniShotCut output {:?} shape is {:?}; expected [{QUERY_COUNT}]",
                output.name,
                output.shape
            );
        }
        Ok(WindowPrediction::new(intra.data, inter.data, ranges.data))
    }
}

impl OmniShotCutSession<OrtWindowPredictor> {
    pub fn open(model: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::with_predictor(OrtWindowPredictor::open(model)?))
    }

    pub fn open_with_options(model: impl AsRef<Path>, options: OrtSessionOptions) -> Result<Self> {
        Ok(Self::with_predictor(OrtWindowPredictor::open_with_options(
            model, options,
        )?))
    }

    /// Load a selected artifact using its validated route options.
    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
    ) -> Result<Self> {
        let names = validate_contract(manifest, route, artifact)?;
        let route_options: RouteOptions = if route.options.is_null() {
            RouteOptions::default()
        } else {
            serde_json::from_value(route.options.clone())
                .context("OmniShotCut ONNX route options are invalid")?
        };
        let options = OrtSessionOptions {
            intra_threads: route_options.intra_threads,
            memory_pattern: route_options
                .memory_pattern
                .unwrap_or(artifact.precision == "fp32"),
            prepacking: route_options.prepacking.unwrap_or(true),
        };
        let entrypoint = artifact_root.join(&artifact.entrypoint);
        ensure!(
            entrypoint.is_file(),
            "model artifact is missing: {}",
            entrypoint.display()
        );
        Ok(Self::with_predictor(OrtWindowPredictor::open_named(
            &entrypoint,
            options,
            names,
        )?))
    }
}
