use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::transition::{TransitionKind, TransitionValues};

use crate::render::{EXTENSION_CROSS_FADE_ABI, engine_owned_kernel_implementation_sha256};

/// Closed compositor-owned transition kernels with canonical, validated scalar parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PreparedTransitionKernel {
    Builtin {
        kind: TransitionKind,
        params: TransitionValues,
    },
    ExtensionCrossFade {
        implementation_sha256: [u8; 32],
        past_frames: u32,
        future_frames: u32,
    },
}

impl From<TransitionKind> for PreparedTransitionKernel {
    fn from(kind: TransitionKind) -> Self {
        Self::default_builtin(kind)
    }
}

impl PreparedTransitionKernel {
    pub const fn default_builtin(kind: TransitionKind) -> Self {
        Self::Builtin {
            kind,
            params: kind.default_values(),
        }
    }

    pub const fn builtin(self) -> TransitionKind {
        match self {
            Self::Builtin { kind, .. } => kind,
            Self::ExtensionCrossFade { .. } => TransitionKind::Fade,
        }
    }

    pub const fn params(self) -> TransitionValues {
        match self {
            Self::Builtin { params, .. } => params,
            Self::ExtensionCrossFade { .. } => TransitionKind::Fade.default_values(),
        }
    }

    pub(crate) fn validate_wire(self) -> Result<(), PreparedTransitionValidationError> {
        match self {
            Self::Builtin { kind, params } => kind
                .validate_values(params)
                .map_err(PreparedTransitionValidationError::Params),
            Self::ExtensionCrossFade {
                implementation_sha256,
                ..
            } => {
                if implementation_sha256
                    != engine_owned_kernel_implementation_sha256(EXTENSION_CROSS_FADE_ABI)
                        .expect("cross-fade ABI has an engine-owned implementation")
                {
                    return Err(PreparedTransitionValidationError::InvalidImplementation);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub(crate) enum PreparedTransitionValidationError {
    #[error("transition implementation digest is not executable by this engine")]
    InvalidImplementation,
    #[error(transparent)]
    Params(valle_draw::transition::TransitionParamError),
}
