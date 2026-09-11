//! Valle-owned inference implementations. Weights are installed separately from Hugging Face.

#[cfg(any(feature = "model-birefnet-onnx", feature = "model-coreml"))]
pub mod birefnet;
#[cfg(feature = "model-demucs-onnx")]
pub mod demucs;
#[cfg(feature = "model-dpdfnet-onnx")]
pub mod dpdfnet;
#[cfg(feature = "model-edgetam-onnx")]
pub mod edgetam;
#[cfg(feature = "model-lama-onnx")]
pub mod lama;
#[cfg(any(feature = "model-modnet-onnx", feature = "model-coreml"))]
pub mod modnet;
#[cfg(feature = "model-omnishotcut-onnx")]
pub mod omnishotcut;
#[cfg(all(feature = "model-qwen-native", not(target_os = "windows")))]
pub mod qwen_asr;
#[cfg(feature = "model-realesrgan-onnx")]
pub mod realesrgan;
#[cfg(feature = "model-rife-onnx")]
pub mod rife;
#[cfg(feature = "model-transnetv2-onnx")]
pub mod transnetv2;
