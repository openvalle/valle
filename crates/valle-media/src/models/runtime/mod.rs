//! Low-level graph execution shared by task-specific model adapters.
//!
//! Task semantics intentionally do not live here. Each model adapter owns preprocessing,
//! postprocessing and state, while this module owns the runtime boundary.

use std::borrow::Cow;
use std::path::PathBuf;

use serde::Serialize;

#[cfg(feature = "model-onnx")]
pub mod onnx;

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
pub mod coreml;

#[derive(Debug, Clone)]
pub struct TensorInput<'a> {
    pub name: Cow<'a, str>,
    pub shape: Vec<usize>,
    pub data: Cow<'a, [f32]>,
}

impl<'a> TensorInput<'a> {
    pub fn borrowed(name: &'a str, shape: impl Into<Vec<usize>>, data: &'a [f32]) -> Self {
        Self {
            name: Cow::Borrowed(name),
            shape: shape.into(),
            data: Cow::Borrowed(data),
        }
    }
}

impl TensorInput<'static> {
    pub fn owned(name: impl Into<String>, shape: impl Into<Vec<usize>>, data: Vec<f32>) -> Self {
        Self {
            name: Cow::Owned(name.into()),
            shape: shape.into(),
            data: Cow::Owned(data),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TensorOutput {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub model: String,
    pub version: String,
    pub backend: String,
    pub artifact: String,
    pub input: PathBuf,
    pub output: PathBuf,
    pub source_width: u32,
    pub source_height: u32,
    pub model_width: u32,
    pub model_height: u32,
    pub load_ms: f64,
    pub preprocess_ms: f64,
    pub inference_ms: f64,
    pub postprocess_ms: f64,
    pub total_ms: f64,
}
