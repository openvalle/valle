//! Author material overrides. Resource bindings are immutable; numeric values belong to a frame.
use super::{Color4, ContractErrors, MaterialKind, validate_control};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MATERIAL_FRAME_SCALARS: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextureRole {
    Color,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MaterialTextureSlot {
    BaseColor,
    MetallicRoughness,
    Normal,
    Occlusion,
    Emissive,
}
impl MaterialTextureSlot {
    pub fn role(self) -> TextureRole {
        match self {
            Self::BaseColor | Self::Emissive => TextureRole::Color,
            _ => TextureRole::Data,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextureWrap {
    Clamp,
    #[default]
    Repeat,
    Mirror,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextureFilter {
    Nearest,
    #[default]
    Linear,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MipmapFilter {
    None,
    Nearest,
    #[default]
    Linear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialTexture {
    pub control: String,
    #[serde(default)]
    pub wrap_u: TextureWrap,
    #[serde(default)]
    pub wrap_v: TextureWrap,
    #[serde(default)]
    pub min_filter: TextureFilter,
    #[serde(default)]
    pub mag_filter: TextureFilter,
    #[serde(default)]
    pub mipmap: MipmapFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    Opaque,
    Mask,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MaterialKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<AlphaMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_sided: Option<bool>,
    /// Missing key inherits the prior source/global binding; null explicitly removes it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub textures: BTreeMap<MaterialTextureSlot, Option<MaterialTexture>>,
}
impl MaterialSpec {
    pub(super) fn validate_at(&self, path: &str, errors: &mut ContractErrors) {
        for (slot, texture) in &self.textures {
            if let Some(texture) = texture {
                validate_control(
                    &texture.control,
                    &format!("{path}/textures/{slot:?}/control"),
                    errors,
                );
            }
        }
    }
    pub fn texture_controls(&self) -> impl Iterator<Item = (&str, TextureRole)> {
        self.textures.iter().filter_map(|(slot, texture)| {
            texture.as_ref().map(|t| (t.control.as_str(), slot.role()))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialOverrideSpec {
    /// Original glTF material index. Materials without an index use the global override only.
    pub id: u32,
    pub material: MaterialSpec,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialFrameState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<Color4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive_intensity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_cutoff: Option<f32>,
}
impl MaterialFrameState {
    pub fn validate(&self) -> Result<(), ContractErrors> {
        let mut errors = ContractErrors::default();
        self.validate_at("/material", &mut errors);
        errors.finish()
    }
    pub(super) fn validate_at(&self, path: &str, errors: &mut ContractErrors) {
        for (name, color) in [("color", self.color), ("emissive", self.emissive)] {
            if let Some(color) = color {
                if !color.valid() || (name == "emissive" && color.0[3] != 1.0) {
                    errors.push(
                        format!("{path}/{name}"),
                        "material colors must be finite sRGB colors; emissive must be opaque",
                    );
                }
            }
        }
        for (name, value, min, max) in [
            ("metallic", self.metallic, 0.0, 1.0),
            ("roughness", self.roughness, 0.0, 1.0),
            ("emissiveIntensity", self.emissive_intensity, 0.0, 16.0),
            ("normalScale", self.normal_scale, -16.0, 16.0),
            ("occlusionStrength", self.occlusion_strength, 0.0, 1.0),
            ("alphaCutoff", self.alpha_cutoff, 0.0, 1.0),
        ] {
            if value.is_some_and(|v| !v.is_finite() || !(min..=max).contains(&v)) {
                errors.push(
                    format!("{path}/{name}"),
                    format!("material value must be finite in {min}..={max}"),
                );
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialOverrideState {
    pub id: u32,
    pub material: MaterialFrameState,
}
