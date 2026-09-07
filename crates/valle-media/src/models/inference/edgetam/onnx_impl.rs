use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use crate::models::runtime::TensorOutput;
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions, OnnxTensorInput};
use crate::models::spec::{Artifact, ModelManifest, Route};
use anyhow::{Context, Result, ensure};

use crate::models::inference::edgetam::adapter::full_memory_positions;
use crate::models::inference::edgetam::{
    AdapterOptions, CANVAS_SIDE, DecodedFrame, EMBEDDING_DIM, EMBEDDING_GRID, EdgeTamAdapter,
    EdgeTamBackend, EdgeTamConstants, EdgeTamSession, EncodedFrame, KEY_VALUE_TOKENS, LOW_SIDE,
    MEMORY_TOKENS_PER_FRAME,
};

#[derive(Debug, Clone)]
pub struct OnnxOptions {
    pub adapter: AdapterOptions,
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

impl Default for OnnxOptions {
    fn default() -> Self {
        Self {
            adapter: AdapterOptions::default(),
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

/// Five load-once ONNX sessions plus immutable hot-path attention constants.
pub struct OnnxBackend {
    image_encoder: OnnxSession,
    memory_attention: OnnxSession,
    seed_decoder: OnnxSession,
    tracking_decoder: OnnxSession,
    memory_encoder: OnnxSession,
    current_positions: Vec<f32>,
    full_memory_positions: Vec<f32>,
    zero_attention_bias: Vec<f32>,
}

impl OnnxBackend {
    fn load(
        artifact_root: &Path,
        constants: &EdgeTamConstants,
        options: &OnnxOptions,
    ) -> Result<Self> {
        if let Some(threads) = options.intra_threads {
            ensure!(threads > 0, "ONNX intra_threads must be positive");
        }
        let runtime_options = OnnxSessionOptions {
            intra_threads: options.intra_threads,
            memory_pattern: options.memory_pattern,
            prepacking: options.prepacking,
        };
        let image_encoder = load_component(
            artifact_root,
            "image_encoder.onnx",
            &runtime_options,
            &["image"],
            &["pix_feat", "hr0", "hr1"],
        )?;
        let memory_attention = load_component(
            artifact_root,
            "memattn.onnx",
            &runtime_options,
            &["curr", "curr_pos", "memory", "memory_pos", "bias"],
            &["conditioned"],
        )?;
        let seed_decoder = load_component(
            artifact_root,
            "decoder_p3.onnx",
            &runtime_options,
            &["pix", "hr0", "hr1", "coords", "labels"],
            &["masks4", "ious4", "objptr4", "obj_score"],
        )?;
        let tracking_decoder = load_component(
            artifact_root,
            "decoder_p1.onnx",
            &runtime_options,
            &["pix", "hr0", "hr1", "coords", "labels"],
            &["masks4", "ious4", "objptr4", "obj_score"],
        )?;
        let memory_encoder = load_component(
            artifact_root,
            "memencode.onnx",
            &runtime_options,
            &["pix_feat", "mask_mem"],
            &["maskmem", "maskmem_pos"],
        )?;
        Ok(Self {
            image_encoder,
            memory_attention,
            seed_decoder,
            tracking_decoder,
            memory_encoder,
            current_positions: constants.current_positions.clone(),
            full_memory_positions: full_memory_positions(
                &constants.memory_positions,
                &constants.temporal_positions,
            )?,
            zero_attention_bias: vec![0.0; KEY_VALUE_TOKENS],
        })
    }
}

impl EdgeTamBackend for OnnxBackend {
    fn encode(&mut self, image_nchw_0_255: &[f32]) -> Result<EncodedFrame> {
        let outputs = self.image_encoder.run_typed(
            &[OnnxTensorInput::borrowed_f32(
                "image",
                [1, 3, CANVAS_SIDE, CANVAS_SIDE],
                image_nchw_0_255,
            )],
            &["pix_feat", "hr0", "hr1"],
        )?;
        let mut outputs = outputs.into_iter();
        Ok(EncodedFrame {
            pixel_features: take_output(
                outputs.next(),
                "pix_feat",
                &[1, EMBEDDING_DIM, EMBEDDING_GRID, EMBEDDING_GRID],
            )?,
            high_resolution_0: take_output(outputs.next(), "hr0", &[1, 32, LOW_SIDE, LOW_SIDE])?,
            high_resolution_1: take_output(outputs.next(), "hr1", &[1, 64, 128, 128])?,
        })
    }

    fn memory_attention_warm(
        &mut self,
        current_tokens: &[f32],
        memory: &[f32],
        memory_positions: &[f32],
        attention_bias: &[f32],
    ) -> Result<Vec<f32>> {
        self.run_memory_attention(current_tokens, memory, memory_positions, attention_bias)
    }

    fn memory_attention_hot(&mut self, current_tokens: &[f32], memory: &[f32]) -> Result<Vec<f32>> {
        // Borrowing self fields while mutably borrowing the session requires narrow locals.
        let positions = self.full_memory_positions.as_slice();
        let bias = self.zero_attention_bias.as_slice();
        let inputs = [
            OnnxTensorInput::borrowed_f32(
                "curr",
                [1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
                current_tokens,
            ),
            OnnxTensorInput::borrowed_f32(
                "curr_pos",
                [1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
                &self.current_positions,
            ),
            OnnxTensorInput::borrowed_f32("memory", [1, KEY_VALUE_TOKENS, 64], memory),
            OnnxTensorInput::borrowed_f32("memory_pos", [1, KEY_VALUE_TOKENS, 64], positions),
            OnnxTensorInput::borrowed_f32("bias", [1, 1, 1, KEY_VALUE_TOKENS], bias),
        ];
        let output = self
            .memory_attention
            .run_typed(&inputs, &["conditioned"])?
            .into_iter()
            .next();
        take_output(
            output,
            "conditioned",
            &[1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
        )
    }

    fn decode(
        &mut self,
        pixel_features: &[f32],
        high_resolution_0: &[f32],
        high_resolution_1: &[f32],
        coordinates: &[f32],
        labels: &[i32],
    ) -> Result<DecodedFrame> {
        ensure!(
            labels.len() == 1 || labels.len() == 3,
            "EdgeTAM decoder requires one or three labels"
        );
        ensure!(
            coordinates.len() == labels.len() * 2,
            "EdgeTAM decoder coordinates and labels disagree"
        );
        let session = if labels.len() == 3 {
            &mut self.seed_decoder
        } else {
            &mut self.tracking_decoder
        };
        let inputs = [
            OnnxTensorInput::borrowed_f32(
                "pix",
                [1, EMBEDDING_DIM, EMBEDDING_GRID, EMBEDDING_GRID],
                pixel_features,
            ),
            OnnxTensorInput::borrowed_f32("hr0", [1, 32, LOW_SIDE, LOW_SIDE], high_resolution_0),
            OnnxTensorInput::borrowed_f32("hr1", [1, 64, 128, 128], high_resolution_1),
            OnnxTensorInput::borrowed_f32("coords", [1, labels.len(), 2], coordinates),
            OnnxTensorInput::borrowed_i32("labels", [1, labels.len()], labels),
        ];
        let outputs = session.run_typed(&inputs, &["masks4", "ious4", "objptr4", "obj_score"])?;
        let mut outputs = outputs.into_iter();
        let mask_logits = take_output(outputs.next(), "masks4", &[1, 4, LOW_SIDE, LOW_SIDE])?;
        let predicted_ious = take_output(outputs.next(), "ious4", &[1, 4])?;
        let object_pointers = take_output(outputs.next(), "objptr4", &[1, 4, EMBEDDING_DIM])?;
        let object_score = take_output(outputs.next(), "obj_score", &[1, 1])?
            .into_iter()
            .next()
            .context("obj_score output is empty")?;
        Ok(DecodedFrame {
            mask_logits,
            predicted_ious,
            object_pointers,
            object_score,
        })
    }

    fn encode_memory(&mut self, pixel_features: &[f32], memory_mask: &[f32]) -> Result<Vec<f32>> {
        let inputs = [
            OnnxTensorInput::borrowed_f32(
                "pix_feat",
                [1, EMBEDDING_DIM, EMBEDDING_GRID, EMBEDDING_GRID],
                pixel_features,
            ),
            OnnxTensorInput::borrowed_f32(
                "mask_mem",
                [1, 1, CANVAS_SIDE, CANVAS_SIDE],
                memory_mask,
            ),
        ];
        let output = self
            .memory_encoder
            .run_typed(&inputs, &["maskmem"])?
            .into_iter()
            .next();
        take_output(output, "maskmem", &[1, MEMORY_TOKENS_PER_FRAME, 64])
    }
}

impl OnnxBackend {
    fn run_memory_attention(
        &mut self,
        current_tokens: &[f32],
        memory: &[f32],
        memory_positions: &[f32],
        attention_bias: &[f32],
    ) -> Result<Vec<f32>> {
        let inputs = [
            OnnxTensorInput::borrowed_f32(
                "curr",
                [1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
                current_tokens,
            ),
            OnnxTensorInput::borrowed_f32(
                "curr_pos",
                [1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
                &self.current_positions,
            ),
            OnnxTensorInput::borrowed_f32("memory", [1, KEY_VALUE_TOKENS, 64], memory),
            OnnxTensorInput::borrowed_f32(
                "memory_pos",
                [1, KEY_VALUE_TOKENS, 64],
                memory_positions,
            ),
            OnnxTensorInput::borrowed_f32("bias", [1, 1, 1, KEY_VALUE_TOKENS], attention_bias),
        ];
        let output = self
            .memory_attention
            .run_typed(&inputs, &["conditioned"])?
            .into_iter()
            .next();
        take_output(
            output,
            "conditioned",
            &[1, EMBEDDING_GRID * EMBEDDING_GRID, EMBEDDING_DIM],
        )
    }
}

/// Concrete load-once ONNX model. Each `start_session` creates isolated job state.
pub struct OnnxEdgeTam {
    adapter: EdgeTamAdapter<OnnxBackend>,
    load_ms: f64,
}

impl OnnxEdgeTam {
    pub fn load(
        manifest: &ModelManifest,
        route: &Route,
        artifact: &Artifact,
        artifact_root: &Path,
        options: &OnnxOptions,
    ) -> Result<Self> {
        let started = Instant::now();
        crate::models::inference::edgetam::contract::validate_contract(
            manifest,
            route,
            artifact,
            artifact_root,
        )?;
        let constants = EdgeTamConstants::load(artifact_root)?;
        let backend = OnnxBackend::load(artifact_root, &constants, options)?;
        let adapter = EdgeTamAdapter::new(backend, constants, options.adapter)?;
        Ok(Self {
            adapter,
            load_ms: started.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    pub const fn options(&self) -> AdapterOptions {
        self.adapter.options()
    }

    pub fn start_session(&mut self) -> EdgeTamSession<'_, OnnxBackend> {
        self.adapter.start_session()
    }
}

fn load_component(
    root: &Path,
    filename: &str,
    options: &OnnxSessionOptions,
    expected_inputs: &[&str],
    expected_outputs: &[&str],
) -> Result<OnnxSession> {
    let path = root.join(filename);
    let session = OnnxSession::load_with_options(&path, options)
        .with_context(|| format!("failed to load EdgeTAM component {}", path.display()))?;
    ensure!(
        session.input_names().into_iter().collect::<BTreeSet<_>>()
            == expected_inputs
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        "EdgeTAM component {filename} input names drifted"
    );
    ensure!(
        session.output_names().into_iter().collect::<BTreeSet<_>>()
            == expected_outputs
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        "EdgeTAM component {filename} output names drifted"
    );
    Ok(session)
}

fn take_output(output: Option<TensorOutput>, name: &str, shape: &[usize]) -> Result<Vec<f32>> {
    let output = output.with_context(|| format!("EdgeTAM component omitted output {name}"))?;
    ensure!(
        output.name == name && output.shape == shape,
        "EdgeTAM output {name} has name {:?} and shape {:?}, expected {shape:?}",
        output.name,
        output.shape
    );
    ensure!(
        output.data.iter().all(|value| value.is_finite()),
        "EdgeTAM output {name} contains NaN or Inf"
    );
    Ok(output.data)
}
