use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::render::{EXTENSION_COLOR_GAIN_ABI, engine_owned_kernel_implementation_sha256};

use super::{
    DynamicBindingId, DynamicBindingKind, DynamicValue,
    request::{DynamicAllocator, RequestError},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreparedEffectSpace {
    Layer {
        transform: DynamicBindingId,
        bounds: DynamicBindingId,
    },
    Root,
}

/// Effect-space unit rectangle: clip-box local for Layer effects, root-canvas local for Root.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedUnitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PreparedUnitRect {
    pub(crate) fn validate_wire(self) -> Result<(), PreparedEffectValidationError> {
        let right = self.x + self.width;
        let bottom = self.y + self.height;
        if [self.x, self.y, self.width, self.height, right, bottom]
            .iter()
            .all(|value| value.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && right <= 1.0
            && bottom <= 1.0
        {
            Ok(())
        } else {
            Err(PreparedEffectValidationError::InvalidRegion)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedBlurAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreparedEffectKernel {
    ChromaKey {
        key_working_linear_rec2020: [f32; 3],
        intensity: f32,
        shadow: f32,
        feather_sigma_device_px: DynamicBindingId,
        edge_clean: f32,
    },
    ColorGrade {
        brightness: f32,
        contrast: f32,
        saturation: f32,
        temperature: f32,
        vignette: f32,
    },
    GaussianBlur {
        sigma_device_px: DynamicBindingId,
        region: Option<PreparedUnitRect>,
    },
    Mosaic {
        block_size_device_px: DynamicBindingId,
        region: Option<PreparedUnitRect>,
    },
    DirectionalBlur {
        axis: PreparedBlurAxis,
        span_device_px: DynamicBindingId,
    },
    Spotlight {
        center: [f32; 2],
        radius: f32,
        feather: f32,
        intensity: f32,
    },
    /// Deterministic engine-owned implementation of
    /// `valle.compositor/color-gain@1`.
    ExtensionColorGain {
        implementation_sha256: [u8; 32],
        gain: f32,
        past_frames: u32,
        future_frames: u32,
    },
}

impl PreparedEffectKernel {
    pub(crate) fn validate_wire(self) -> Result<(), PreparedEffectValidationError> {
        match self {
            Self::ChromaKey {
                key_working_linear_rec2020,
                intensity,
                shadow,
                edge_clean,
                ..
            } => {
                finite(&key_working_linear_rec2020)?;
                unit(intensity)?;
                unit(shadow)?;
                unit(edge_clean)
            }
            Self::ColorGrade {
                brightness,
                contrast,
                saturation,
                temperature,
                vignette,
            } => {
                signed_unit(brightness)?;
                signed_unit(contrast)?;
                signed_unit(saturation)?;
                signed_unit(temperature)?;
                unit(vignette)
            }
            Self::GaussianBlur { region, .. } | Self::Mosaic { region, .. } => {
                if let Some(region) = region {
                    region.validate_wire()?;
                }
                Ok(())
            }
            Self::DirectionalBlur { .. } => Ok(()),
            Self::Spotlight {
                center,
                radius,
                feather,
                intensity,
            } => {
                unit(center[0])?;
                unit(center[1])?;
                unit(radius)?;
                unit(feather)?;
                unit(intensity)
            }
            Self::ExtensionColorGain {
                implementation_sha256,
                gain,
                ..
            } => {
                if implementation_sha256
                    != engine_owned_kernel_implementation_sha256(EXTENSION_COLOR_GAIN_ABI)
                        .expect("color-gain ABI has an engine-owned implementation")
                {
                    Err(PreparedEffectValidationError::InvalidImplementation)
                } else if gain.is_finite() && (0.0..=16.0).contains(&gain) {
                    Ok(())
                } else {
                    Err(PreparedEffectValidationError::InvalidParameter)
                }
            }
        }
    }

    pub(crate) fn device_lengths(self) -> Vec<(DynamicBindingId, &'static str)> {
        match self {
            Self::ChromaKey {
                feather_sigma_device_px,
                ..
            } => vec![(feather_sigma_device_px, "featherSigmaDevicePx")],
            Self::GaussianBlur {
                sigma_device_px, ..
            } => vec![(sigma_device_px, "sigmaDevicePx")],
            Self::Mosaic {
                block_size_device_px,
                ..
            } => vec![(block_size_device_px, "blockSizeDevicePx")],
            Self::DirectionalBlur { span_device_px, .. } => {
                vec![(span_device_px, "spanDevicePx")]
            }
            Self::ColorGrade { .. } | Self::Spotlight { .. } | Self::ExtensionColorGain { .. } => {
                Vec::new()
            }
        }
    }

    /// Operators that must run on the local primary source before a generated placement backdrop
    /// is composed. They are never valid as root effects.
    pub(crate) const fn is_source_operator(self) -> bool {
        matches!(self, Self::ChromaKey { .. })
    }

    pub(crate) const fn canonical_rank(self) -> u8 {
        match self {
            Self::ChromaKey { .. } => 5,
            Self::ColorGrade { .. } => 10,
            Self::GaussianBlur { .. } => 20,
            Self::Mosaic { .. } => 21,
            Self::DirectionalBlur { .. } => 22,
            Self::Spotlight { .. } => 23,
            Self::ExtensionColorGain { .. } => 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedEffect {
    pub semantic_path: String,
    pub space: PreparedEffectSpace,
    pub kernel: PreparedEffectKernel,
}

impl PreparedEffect {
    pub(crate) fn validate_wire(&self) -> Result<(), PreparedEffectValidationError> {
        if self.semantic_path.is_empty() {
            return Err(PreparedEffectValidationError::EmptySemanticPath);
        }
        self.kernel.validate_wire()?;
        let space_is_canonical = !self.kernel.is_source_operator()
            || matches!(self.space, PreparedEffectSpace::Layer { .. });
        if space_is_canonical {
            Ok(())
        } else {
            Err(PreparedEffectValidationError::InvalidResources)
        }
    }
}

/// Lower an already evaluated `animate.presets.blur` value into the same typed kernel used by
/// authored layer effects. `ResolvedClipAnimation::blur_sigma_px` is a Gaussian sigma in design-canvas
/// pixels (unlike `Effect::Blur::radius`, which is a blur radius), so this path deliberately does
/// not apply the radius-to-sigma conversion a second time.
pub(crate) fn prepare_animation_blur(
    path: &str,
    space: PreparedEffectSpace,
    sigma_device_px: f64,
    dynamic: &mut DynamicAllocator,
) -> Result<Option<PreparedEffect>, EffectPrepareError> {
    if !sigma_device_px.is_finite() || sigma_device_px < 0.0 {
        return Err(EffectPrepareError::InvalidParameter);
    }
    if sigma_device_px == 0.0 {
        return Ok(None);
    }
    let prepared = PreparedEffect {
        semantic_path: path.to_owned(),
        space,
        kernel: PreparedEffectKernel::GaussianBlur {
            sigma_device_px: device_length(dynamic, path, "sigmaDevicePx", sigma_device_px)?,
            region: None,
        },
    };
    prepared.validate_wire()?;
    Ok(Some(prepared))
}

fn device_length(
    dynamic: &mut DynamicAllocator,
    path: &str,
    name: &'static str,
    value: f64,
) -> Result<DynamicBindingId, EffectPrepareError> {
    dynamic
        .push(
            format!("{path}.{name}"),
            DynamicBindingKind::DeviceLength,
            DynamicValue::Scalar(value),
        )
        .map_err(Into::into)
}

fn finite(values: &[f32]) -> Result<(), PreparedEffectValidationError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(PreparedEffectValidationError::InvalidParameter)
    }
}

fn unit(value: f32) -> Result<(), PreparedEffectValidationError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(PreparedEffectValidationError::InvalidParameter)
    }
}

fn signed_unit(value: f32) -> Result<(), PreparedEffectValidationError> {
    if value.is_finite() && (-1.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(PreparedEffectValidationError::InvalidParameter)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PreparedEffectValidationError {
    #[error("effect semantic path must not be empty")]
    EmptySemanticPath,
    #[error("effect region must be a non-empty finite rectangle inside the unit domain")]
    InvalidRegion,
    #[error("effect parameter is outside its typed range")]
    InvalidParameter,
    #[error("effect implementation digest is not executable by this engine")]
    InvalidImplementation,
    #[error("effect resources or coordinate space are not canonical for its kernel")]
    InvalidResources,
}

#[derive(Debug, Error)]
pub enum EffectPrepareError {
    #[error("effect parameter is non-finite or outside its typed range")]
    InvalidParameter,
    #[error(transparent)]
    Validation(#[from] PreparedEffectValidationError),
    #[error(transparent)]
    Request(#[from] RequestError),
}
