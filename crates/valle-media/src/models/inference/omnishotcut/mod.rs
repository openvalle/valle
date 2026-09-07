//! Reusable OmniShotCut shot-range and transition-class adapter.
//!
//! The preferred library boundary is a stream of tightly packed, display-oriented
//! straight-alpha RGBA8 frames. This adapter owns resize to 128x96, RGB packing,
//! the upstream 100-frame window, 20-frame overlap, black tail padding, query walk
//! and overlap merge. It never decodes media, shells out, or retains a complete
//! video. A session owns one backend predictor and can be reused for multiple
//! videos after [`OmniShotCutSession::finish`].

mod contract;
mod frame;
#[cfg(feature = "model-omnishotcut-onnx")]
mod onnx_impl;
mod preprocess;
mod stream;

pub use contract::{ADAPTER, CONTRACT_VERSION, validate_release_contract};
pub use frame::{FRAME_BYTES, FrameView, MODEL_HEIGHT, MODEL_WIDTH, PixelFormat};
#[cfg(feature = "model-omnishotcut-onnx")]
pub use onnx_impl::{OrtSessionOptions, OrtWindowPredictor};
pub use stream::{
    INTER_LABELS, INTRA_LABELS, InterTransition, IntraTransition, MAX_BUFFERED_FRAMES,
    MODEL_WINDOW_BYTES, OVERLAP_FRAMES, OmniShotCutSession, PushTiming, QUERY_COUNT, STRIDE_FRAMES,
    Shot, ShotList, WINDOW_FRAMES, WindowPrediction, WindowPredictor,
};
