//! Canonical resource-manifest wire DTOs.
//!
//! The manifest contains only stable semantic identity and verified
//! descriptors. Every resource kind and descriptor is closed; runtime binding
//! information belongs to a separate API.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::internal::{
    ContentDigest,
    time::{ExactRational, RationalTime},
};

pub type ResourceId = String;

/// Complete frozen resource-manifest envelope.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceManifestEnvelopeWire {
    pub entries: BTreeMap<ResourceId, ResourceEntryWire>,
}

/// Closed resource-kind union. A descriptor from one kind cannot be decoded as
/// another kind's descriptor.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResourceEntryWire {
    Video {
        digest: ContentDigest,
        descriptor: VideoResourceDescriptorWire,
    },
    Audio {
        digest: ContentDigest,
        descriptor: AudioResourceDescriptorWire,
    },
    Image {
        digest: ContentDigest,
        descriptor: ImageResourceDescriptorWire,
    },
    Lottie {
        digest: ContentDigest,
        abi: LottieArtifactAbiWire,
        descriptor: LottieResourceDescriptorWire,
    },
    Font {
        digest: ContentDigest,
        descriptor: FontResourceDescriptorWire,
    },
    MotionArtifact {
        digest: ContentDigest,
        abi: MotionArtifactAbiWire,
        descriptor: MotionArtifactDescriptorWire,
    },
    Shader {
        digest: ContentDigest,
        abi: ShaderArtifactAbiWire,
        descriptor: ShaderResourceDescriptorWire,
    },
}

/// Frozen video stream and source-time semantics.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoResourceDescriptorWire {
    pub duration: RationalTime,
    pub time_base: ExactRational,
    pub presentation_index_digest: ContentDigest,
    pub width: u32,
    pub height: u32,
    pub orientation: MediaOrientationWire,
    pub color: MediaColorDescriptorWire,
    pub video_stream: u32,
    #[cfg_attr(
        feature = "schema",
        schemars(with = "crate::internal::schema::RequiredNullable<u32>")
    )]
    #[serde(deserialize_with = "required_option")]
    pub audio_stream: Option<u32>,
}

/// Frozen audio stream and source-time semantics.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioResourceDescriptorWire {
    pub duration: RationalTime,
    pub time_base: ExactRational,
    pub presentation_index_digest: ContentDigest,
    pub sample_rate: u32,
    pub channel_layout: AudioChannelLayoutWire,
    pub audio_stream: u32,
}

/// Frozen raster extent and interpretation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageResourceDescriptorWire {
    pub width: u32,
    pub height: u32,
    pub orientation: MediaOrientationWire,
    pub color: MediaColorDescriptorWire,
}

/// Frozen Lottie source clock, extent, and boundary-sample semantics.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LottieResourceDescriptorWire {
    pub duration: RationalTime,
    pub time_base: ExactRational,
    pub width: u32,
    pub height: u32,
    pub boundary_sampling: ContinuousBoundarySamplingWire,
}

/// One concrete font face and its supported variable-font axes.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontResourceDescriptorWire {
    pub face_index: u32,
    pub variation_axes: BTreeMap<String, FontVariationAxisWire>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontVariationAxisWire {
    pub minimum: f64,
    pub default: f64,
    pub maximum: f64,
}

/// Motion artifact compositor-read semantics. The artifact's embedded `controls` object is the
/// only control schema; the manifest does not carry a second digest projection of it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionArtifactDescriptorWire {
    pub reads_destination: bool,
    pub boundary_sampling: ContinuousBoundarySamplingWire,
}

/// Frozen shader parameter schema and destination-read semantics.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShaderResourceDescriptorWire {
    pub controls_schema_digest: ContentDigest,
    pub reads_destination: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaOrientationWire {
    Identity,
    #[serde(rename = "rotate-90")]
    Rotate90,
    #[serde(rename = "rotate-180")]
    Rotate180,
    #[serde(rename = "rotate-270")]
    Rotate270,
    FlipHorizontal,
    FlipVertical,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaColorDescriptorWire {
    pub primaries: ColorPrimariesWire,
    pub transfer: ColorTransferWire,
    pub matrix: ColorMatrixWire,
    pub full_range: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorPrimariesWire {
    Srgb,
    Bt709,
    DisplayP3,
    Bt2020,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorTransferWire {
    Srgb,
    Bt709,
    Pq,
    Hlg,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMatrixWire {
    Identity,
    Bt709,
    Bt2020Ncl,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioChannelLayoutWire {
    Mono,
    Stereo,
    Surround51,
    Surround71,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContinuousBoundarySamplingWire {
    LeftLimit,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotionArtifactAbiWire {
    #[serde(rename = "valle.motion/artifact@1")]
    Canonical,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LottieArtifactAbiWire {
    #[serde(rename = "valle.lottie/artifact@1")]
    Canonical,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderArtifactAbiWire {
    #[serde(rename = "valle.shader/artifact@1")]
    Canonical,
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
