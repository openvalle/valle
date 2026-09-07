//! Task-specific model adapters. Complete file workflows belong in `crate::tools`.

#[cfg(feature = "tool-matte")]
pub(crate) mod birefnet;

#[cfg(feature = "tool-transcribe")]
pub(crate) mod qwen_asr;

#[cfg(feature = "model-dpdfnet-onnx")]
pub(crate) mod dpdfnet;

#[cfg(feature = "model-demucs-onnx")]
pub(crate) mod demucs;

#[cfg(feature = "model-edgetam-onnx")]
pub(crate) mod edgetam;

#[cfg(feature = "model-lama-onnx")]
pub(crate) mod lama;

#[cfg(all(feature = "model-omnishotcut-onnx", feature = "model-transnetv2-onnx"))]
pub(crate) mod omnishotcut;

#[cfg(feature = "model-realesrgan-onnx")]
pub(crate) mod realesrgan;

#[cfg(feature = "model-rife-onnx")]
pub(crate) mod rife;
