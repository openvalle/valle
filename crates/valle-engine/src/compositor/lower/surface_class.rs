use serde::{Deserialize, Serialize};

use crate::resource::{
    Extent2d, LogicalTextureDesc, ResourceContractError, TextureFormat, TextureUsage,
    WorkingAlphaMode, WorkingColorSpace,
};

/// Immutable allocation requirements, shared by outer and program-local surface slots.
/// Dimensions and byte estimates belong to the current frame's bound range schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceClass {
    pub format: TextureFormat,
    pub working_space: WorkingColorSpace,
    pub alpha: WorkingAlphaMode,
    pub usages: Vec<TextureUsage>,
    pub sample_count: u8,
}

impl SurfaceClass {
    pub(super) fn production() -> Self {
        Self {
            format: TextureFormat::Rgba16Float,
            working_space: WorkingColorSpace::LinearRec2020D65,
            alpha: WorkingAlphaMode::PremultipliedCoverage,
            usages: vec![
                TextureUsage::Sampled,
                TextureUsage::StorageRead,
                TextureUsage::StorageWrite,
                TextureUsage::ColorAttachment,
            ],
            sample_count: 1,
        }
    }

    pub(super) fn validate(&self) -> Result<(), ResourceContractError> {
        // Reuse the texture contract's canonical usage and sample-count validation without
        // manufacturing a full-frame allocation or making an extent part of the template.
        let canonical = LogicalTextureDesc::new(
            Extent2d::new(1, 1).expect("nonzero constant extent"),
            self.format,
            self.usages.iter().copied(),
            self.sample_count,
        )?;
        if Self::from(&canonical) != *self {
            return Err(ResourceContractError::InvalidWorkingContract);
        }
        Ok(())
    }
}

impl From<&LogicalTextureDesc> for SurfaceClass {
    fn from(texture: &LogicalTextureDesc) -> Self {
        Self {
            format: texture.format,
            working_space: texture.working_space,
            alpha: texture.alpha,
            usages: texture.usages().to_vec(),
            sample_count: texture.sample_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_validates_usage_order_and_samples_without_an_extent() {
        let class = SurfaceClass::production();
        class.validate().unwrap();
        for samples in [0, 3, 16] {
            let mut invalid = class.clone();
            invalid.sample_count = samples;
            assert!(invalid.validate().is_err());
        }
        for usages in [
            vec![],
            vec![TextureUsage::Sampled, TextureUsage::Sampled],
            vec![TextureUsage::ColorAttachment, TextureUsage::Sampled],
        ] {
            let mut invalid = class.clone();
            invalid.usages = usages;
            assert!(invalid.validate().is_err());
        }
        let small = LogicalTextureDesc::production(
            Extent2d::new(8, 8).unwrap(),
            class.usages.iter().copied(),
        )
        .unwrap();
        let large = LogicalTextureDesc::production(
            Extent2d::new(3840, 2160).unwrap(),
            class.usages.iter().copied(),
        )
        .unwrap();
        assert_eq!(SurfaceClass::from(&small), SurfaceClass::from(&large));
    }
}
