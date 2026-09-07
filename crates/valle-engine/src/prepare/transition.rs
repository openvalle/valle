use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::render::{EXTENSION_CROSS_FADE_ABI, engine_owned_kernel_implementation_sha256};

/// Closed compositor-owned transition kernel set. Author/registry strings end at Prepare; a
/// future registry kernel must carry a content digest and typed ABI rather than enter this enum by
/// name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedTransitionKernel {
    Fade,
    WipeLeft,
    WipeRight,
    CircleOpen,
    SimpleZoom,
    CrossWarp,
    LinearBlur,
    DirectionalWarp,
    DreamyZoom,
    Ripple,
    FlyEye,
    MultiplyBlend,
    Perlin,
    /// Deterministic engine-owned implementation of
    /// `valle.compositor/cross-fade@1`.
    ExtensionCrossFade {
        implementation_sha256: [u8; 32],
        past_frames: u32,
        future_frames: u32,
    },
}

impl PreparedTransitionKernel {
    pub(crate) fn validate_wire(self) -> Result<(), PreparedTransitionValidationError> {
        if let Self::ExtensionCrossFade {
            implementation_sha256,
            ..
        } = self
            && implementation_sha256
                != engine_owned_kernel_implementation_sha256(EXTENSION_CROSS_FADE_ABI)
                    .expect("cross-fade ABI has an engine-owned implementation")
        {
            return Err(PreparedTransitionValidationError::InvalidImplementation);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedTransitionValidationError {
    #[error("transition implementation digest is not executable by this engine")]
    InvalidImplementation,
}
