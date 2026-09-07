//! Product bridge for the shared BiRefNet and MODNet matting adapters.
//!
//! The model factory owns tensor names, resize, normalization, output validation and CoreML
//! compilation. This module only resolves one pinned release and keeps one selected session alive
//! for the complete Valle media job.

use std::path::Path;

use crate::models::inference::birefnet::{
    MatteSession as BiRefNetSession, SessionRequest as BiRefNetRequest,
};
use crate::models::inference::modnet::{
    MatteSession as ModNetSession, SessionRequest as ModNetRequest,
};
use crate::models::spec::Backend;
use anyhow::{Context, Result, bail, ensure};

use crate::models::manager::ResolvedModel;

pub(crate) const BIREFNET_MODEL_ID: &str = "birefnet";
pub(crate) const MODNET_MODEL_ID: &str = "modnet";

enum InnerSession {
    BiRefNet(BiRefNetSession),
    ModNet(ModNetSession),
}

/// One explicitly selected, load-once matting session.
pub(crate) struct MatteSession {
    inner: InnerSession,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MatteFrameOutput {
    pub(crate) alpha: Vec<u8>,
    pub(crate) preprocess_seconds: f64,
    pub(crate) inference_seconds: f64,
    pub(crate) postprocess_seconds: f64,
    pub(crate) total_seconds: f64,
}

impl MatteSession {
    pub(crate) fn open(
        resolved: &ResolvedModel,
        compile_cache: &Path,
        cpu_threads: usize,
    ) -> Result<Self> {
        ensure!(cpu_threads > 0, "matte cpu_threads must be positive");
        ensure!(
            matches!(
                resolved.resolved.id.as_str(),
                BIREFNET_MODEL_ID | MODNET_MODEL_ID
            ),
            "matte supports model {BIREFNET_MODEL_ID:?} or {MODNET_MODEL_ID:?}, got {:?}",
            resolved.resolved.id
        );
        let inner = load_session(resolved, compile_cache, cpu_threads)?;
        Ok(Self { inner })
    }

    pub(crate) fn load_seconds(&self) -> f64 {
        let milliseconds = match &self.inner {
            InnerSession::BiRefNet(session) => session.load_ms(),
            InnerSession::ModNet(session) => session.load_ms(),
        };
        milliseconds / 1_000.0
    }

    pub(crate) fn alpha(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<MatteFrameOutput> {
        match &mut self.inner {
            InnerSession::BiRefNet(session) => {
                let frame = session.inference_rgba8(width, height, rgba)?;
                ensure!(
                    frame.width == width && frame.height == height,
                    "BiRefNet output dimensions drifted"
                );
                Ok(MatteFrameOutput {
                    alpha: frame.alpha,
                    preprocess_seconds: frame.timings.preprocess_ms / 1_000.0,
                    inference_seconds: frame.timings.inference_ms / 1_000.0,
                    postprocess_seconds: frame.timings.postprocess_ms / 1_000.0,
                    total_seconds: frame.timings.total_ms / 1_000.0,
                })
            }
            InnerSession::ModNet(session) => {
                let frame = session.inference_rgba8(width, height, rgba)?;
                ensure!(
                    frame.width == width && frame.height == height,
                    "MODNet output dimensions drifted"
                );
                Ok(MatteFrameOutput {
                    alpha: frame.alpha,
                    preprocess_seconds: frame.timings.preprocess_ms / 1_000.0,
                    inference_seconds: frame.timings.inference_ms / 1_000.0,
                    postprocess_seconds: frame.timings.postprocess_ms / 1_000.0,
                    total_seconds: frame.timings.total_ms / 1_000.0,
                })
            }
        }
    }
}

fn load_session(
    model: &ResolvedModel,
    compile_cache: &Path,
    cpu_threads: usize,
) -> Result<InnerSession> {
    let mut route = model.route.clone();
    if route.backend == Backend::OnnxCpu {
        apply_cpu_threads(&mut route.options, cpu_threads)?;
    }
    match model.resolved.id.as_str() {
        BIREFNET_MODEL_ID => BiRefNetSession::load(BiRefNetRequest {
            manifest: &model.manifest,
            route: &route,
            artifact: &model.artifact,
            artifact_root: &model.resolved.root,
            compile_cache: Some(compile_cache),
            size: None,
        })
        .map(InnerSession::BiRefNet)
        .with_context(|| {
            format!(
                "load BiRefNet artifact {} from {}",
                model.resolved.artifact,
                model.resolved.root.display()
            )
        }),
        MODNET_MODEL_ID => ModNetSession::load(ModNetRequest {
            manifest: &model.manifest,
            route: &route,
            artifact: &model.artifact,
            artifact_root: &model.resolved.root,
            compile_cache: Some(compile_cache),
        })
        .map(InnerSession::ModNet)
        .with_context(|| {
            format!(
                "load MODNet artifact {} from {}",
                model.resolved.artifact,
                model.resolved.root.display()
            )
        }),
        id => bail!("resolved unsupported matte model {id:?}"),
    }
}

fn apply_cpu_threads(options: &mut serde_json::Value, cpu_threads: usize) -> Result<()> {
    if options.is_null() {
        *options = serde_json::json!({});
    }
    options
        .as_object_mut()
        .context("matte ONNX route options must be an object")?
        .insert("intra_threads".to_owned(), cpu_threads.into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_model_ids_match_the_published_catalog() {
        assert_eq!(BIREFNET_MODEL_ID, "birefnet");
        assert_eq!(MODNET_MODEL_ID, "modnet");
    }

    #[test]
    fn onnx_thread_budget_can_override_a_null_release_options_value() {
        let mut options = serde_json::Value::Null;
        apply_cpu_threads(&mut options, 4).unwrap();
        assert_eq!(options["intra_threads"], 4);

        let mut invalid = serde_json::json!([]);
        assert!(apply_cpu_threads(&mut invalid, 1).is_err());
    }
}
