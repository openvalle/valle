//! Reusable LaMa image-inpainting adapter and load-once ONNX session.
//!
//! Callers provide tightly packed RGB8/RGBA8 images and strict binary Gray8
//! masks. This crate exclusively owns mask-aware crop planning, sparse
//! partitioning, fixed bucket selection, symmetric canvas padding, the neural
//! `x`/`rgb` tensor boundary, finite checks, restoration and masked composite.
//! It does not decode media, shell out, or retain a video frame history.

mod adapter;
mod frame;
mod plan;

#[cfg(feature = "model-lama-onnx")]
use std::path::Path;
#[cfg(feature = "model-lama-onnx")]
use std::time::Instant;

#[cfg(feature = "model-lama-onnx")]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-lama-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
#[cfg(feature = "model-lama-onnx")]
use anyhow::anyhow;
#[cfg(any(feature = "model-lama-onnx", test))]
use anyhow::{Context, Result, ensure};

pub use adapter::{AdapterMetrics, LamaAdapter, LamaBackend};
pub use frame::{BinaryMaskView, ImageFrame, ImageView, PixelFormat};
pub use plan::{
    CONTEXT_PIXELS, CanvasBucket, InpaintPlan, InpaintTask, PARTITION_CORE_LIMIT, PlanStrategy,
    Rect, SPARSE_FILL_THRESHOLD, plan_inpaint,
};

pub const ADAPTER: &str = "lama-image-inpainting";
pub const CONTRACT_VERSION: u32 = 1;
/// Logical release inputs are `image` and `mask`; the exported neural graph
/// concatenates them behind the adapter as one four-channel tensor named `x`.
pub const ONNX_INPUT_NAME: &str = "x";
pub const ONNX_OUTPUT_NAME: &str = "rgb";

#[cfg(feature = "model-lama-onnx")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOptions {
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

#[cfg(feature = "model-lama-onnx")]
impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

#[cfg(feature = "model-lama-onnx")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InpaintMetrics {
    pub load_ms: f64,
    pub inference_ms: f64,
    pub total_ms: f64,
    pub tasks: usize,
    pub scaled_tasks: usize,
    pub planned_bucket: Option<CanvasBucket>,
    pub actual_bucket: CanvasBucket,
    pub scratch_capacity_bytes: usize,
}

#[cfg(feature = "model-lama-onnx")]
#[derive(Debug, Clone, PartialEq)]
pub struct InpaintResult {
    pub frame: ImageFrame,
    pub plan: InpaintPlan,
    pub metrics: InpaintMetrics,
}

/// One fixed-bucket ONNX Runtime session reused across consecutive images.
///
/// Load the largest published artifact up front for a stream that may contain
/// mixed plan sizes, or use [`plan_inpaint`] when the stream is constrained to
/// one bucket. Every non-empty mask is planned again. A larger loaded bucket
/// may execute a plan whose required width and height both fit its fixed
/// canvas; an undersized bucket fails instead of silently loading another
/// graph or retaining both.
#[cfg(feature = "model-lama-onnx")]
pub struct LamaSession {
    session: OnnxSession,
    adapter: LamaAdapter,
    load_ms: f64,
}

#[cfg(feature = "model-lama-onnx")]
impl LamaSession {
    pub fn load(model: impl AsRef<Path>, bucket: CanvasBucket) -> Result<Self> {
        Self::load_with_options(model, bucket, SessionOptions::default())
    }

    pub fn load_with_options(
        model: impl AsRef<Path>,
        bucket: CanvasBucket,
        options: SessionOptions,
    ) -> Result<Self> {
        if let Some(threads) = options.intra_threads {
            ensure!(threads > 0, "ONNX intra_threads must be positive");
        }
        let model = model.as_ref();
        ensure!(
            model.is_file(),
            "LaMa model is missing: {}",
            model.display()
        );

        let load_started = Instant::now();
        let session = OnnxSession::load_with_options(
            model,
            &OnnxSessionOptions {
                intra_threads: options.intra_threads,
                memory_pattern: options.memory_pattern,
                prepacking: options.prepacking,
            },
        )
        .with_context(|| format!("load LaMa model {}", model.display()))?;
        validate_graph_contract(&session)?;
        Ok(Self {
            session,
            adapter: LamaAdapter::new(bucket)?,
            load_ms: load_started.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    pub const fn bucket(&self) -> CanvasBucket {
        self.adapter.bucket()
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    pub fn scratch_capacity_bytes(&self) -> usize {
        self.adapter.scratch_capacity_bytes()
    }

    pub fn inpaint(
        &mut self,
        image: ImageView<'_>,
        mask: BinaryMaskView<'_>,
    ) -> Result<InpaintResult> {
        let started = Instant::now();
        let mut backend = OnnxBackend {
            session: &mut self.session,
            inference_ms: 0.0,
        };
        let (frame, plan, adapter_metrics) = self.adapter.inpaint(&mut backend, image, mask)?;
        Ok(InpaintResult {
            frame,
            plan,
            metrics: InpaintMetrics {
                load_ms: self.load_ms,
                inference_ms: backend.inference_ms,
                total_ms: started.elapsed().as_secs_f64() * 1_000.0,
                tasks: adapter_metrics.tasks,
                scaled_tasks: adapter_metrics.scaled_tasks,
                planned_bucket: adapter_metrics.planned_bucket,
                actual_bucket: adapter_metrics.actual_bucket,
                scratch_capacity_bytes: self.adapter.scratch_capacity_bytes(),
            },
        })
    }

    pub fn inpaint_planned(
        &mut self,
        image: ImageView<'_>,
        mask: BinaryMaskView<'_>,
        plan: &InpaintPlan,
    ) -> Result<InpaintResult> {
        let started = Instant::now();
        let mut backend = OnnxBackend {
            session: &mut self.session,
            inference_ms: 0.0,
        };
        let (frame, actual_plan, adapter_metrics) =
            self.adapter
                .inpaint_planned(&mut backend, image, mask, plan)?;
        Ok(InpaintResult {
            frame,
            plan: actual_plan,
            metrics: InpaintMetrics {
                load_ms: self.load_ms,
                inference_ms: backend.inference_ms,
                total_ms: started.elapsed().as_secs_f64() * 1_000.0,
                tasks: adapter_metrics.tasks,
                scaled_tasks: adapter_metrics.scaled_tasks,
                planned_bucket: adapter_metrics.planned_bucket,
                actual_bucket: adapter_metrics.actual_bucket,
                scratch_capacity_bytes: self.adapter.scratch_capacity_bytes(),
            },
        })
    }
}

#[cfg(feature = "model-lama-onnx")]
struct OnnxBackend<'a> {
    session: &'a mut OnnxSession,
    inference_ms: f64,
}

#[cfg(feature = "model-lama-onnx")]
impl LamaBackend for OnnxBackend<'_> {
    fn infer(
        &mut self,
        input: &[f32],
        input_shape: [usize; 4],
        output: &mut [f32],
    ) -> Result<[usize; 4]> {
        let started = Instant::now();
        let value = self
            .session
            .run_f32(
                TensorInput::borrowed(ONNX_INPUT_NAME, input_shape, input),
                ONNX_OUTPUT_NAME,
            )
            .context("run LaMa ONNX inference")?;
        self.inference_ms += started.elapsed().as_secs_f64() * 1_000.0;
        ensure!(
            value.data.len() == output.len(),
            "LaMa rgb returned {} values, expected {}",
            value.data.len(),
            output.len()
        );
        output.copy_from_slice(&value.data);
        value.shape.try_into().map_err(|dimensions: Vec<usize>| {
            anyhow!("LaMa rgb shape is {dimensions:?}, expected rank 4")
        })
    }
}

#[cfg(feature = "model-lama-onnx")]
fn validate_graph_contract(session: &OnnxSession) -> Result<()> {
    let inputs = session.input_names();
    ensure!(
        inputs == [ONNX_INPUT_NAME],
        "LaMa ONNX graph must expose exactly one input named {ONNX_INPUT_NAME:?}; got {inputs:?}"
    );
    let outputs = session.output_names();
    ensure!(
        outputs == [ONNX_OUTPUT_NAME],
        "LaMa ONNX graph must expose exactly one output named {ONNX_OUTPUT_NAME:?}; got {outputs:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn release() -> Value {
        serde_json::from_str(include_str!("../../catalog/release-lama-1.0.0.json")).unwrap()
    }

    fn validate_release_contract(release: &Value) -> Result<()> {
        let contract = &release["contract"];
        ensure!(contract["adapter"] == ADAPTER, "adapter id drifted");
        ensure!(
            contract["version"] == CONTRACT_VERSION,
            "adapter version drifted"
        );
        ensure!(
            contract["inputs"][0]["name"] == "image"
                && contract["inputs"][0]["dtype"] == "f32"
                && contract["inputs"][0]["layout"] == "nchw"
                && contract["inputs"][0]["shape"] == json!([1, 3, "height", "width"])
                && contract["inputs"][0]["range"] == json!([0, 1]),
            "logical image tensor drifted"
        );
        ensure!(
            contract["inputs"][1]["name"] == "mask"
                && contract["inputs"][1]["dtype"] == "f32"
                && contract["inputs"][1]["layout"] == "nchw"
                && contract["inputs"][1]["shape"] == json!([1, 1, "height", "width"])
                && contract["inputs"][1]["range"] == json!([0, 1]),
            "logical mask tensor drifted"
        );
        ensure!(
            contract["outputs"][0]["name"] == ONNX_OUTPUT_NAME
                && contract["outputs"][0]["dtype"] == "f32"
                && contract["outputs"][0]["layout"] == "nchw"
                && contract["outputs"][0]["shape"] == json!([1, 3, "height", "width"])
                && contract["outputs"][0]["range"] == json!([0, 1]),
            "logical output tensor drifted"
        );
        ensure!(
            contract["preprocess"]["canvas"] == json!([640, 384]),
            "fallback canvas drifted"
        );
        ensure!(
            contract["preprocess"]["canvas_buckets"] == json!([[256, 256], [640, 384]]),
            "canvas buckets drifted"
        );
        ensure!(
            contract["preprocess"]["bucket_policy"]
                == "first context-expanded crop that fits without scaling; final bucket is fallback",
            "bucket policy drifted"
        );
        ensure!(
            contract["preprocess"]["host_contract"] == "crop-mask-with-context-and-pad",
            "host contract drifted"
        );
        ensure!(
            contract["postprocess"]["reject_non_finite"] == true
                && contract["postprocess"]["composite_to_source"] == true,
            "postprocess contract drifted"
        );
        let artifact = release["artifacts"]
            .as_array()
            .and_then(|artifacts| {
                artifacts
                    .iter()
                    .find(|item| item["id"] == "onnx-fp32-buckets")
            })
            .context("ONNX bucket artifact is missing")?;
        ensure!(artifact["metadata"]["opset"] == 17, "ONNX opset drifted");
        ensure!(
            artifact["metadata"]["fixed_shape_buckets"]["256x256"]
                == CanvasBucket::Square256.onnx_filename()
                && artifact["metadata"]["fixed_shape_buckets"]["fallback"]
                    == CanvasBucket::Wide640x384.onnx_filename(),
            "ONNX bucket entrypoints drifted"
        );
        Ok(())
    }

    #[test]
    fn checked_in_release_matches_the_adapter_contract() {
        validate_release_contract(&release()).unwrap();
    }

    #[test]
    fn contract_validation_rejects_tensor_bucket_and_postprocess_drift() {
        let mut value = release();
        value["contract"]["inputs"][1]["name"] = "soft_mask".into();
        assert!(validate_release_contract(&value).is_err());

        let mut value = release();
        value["contract"]["preprocess"]["canvas_buckets"] = json!([[640, 384]]);
        assert!(validate_release_contract(&value).is_err());

        let mut value = release();
        value["contract"]["postprocess"]["reject_non_finite"] = false.into();
        assert!(validate_release_contract(&value).is_err());
    }
}
