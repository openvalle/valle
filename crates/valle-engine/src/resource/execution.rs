use std::{
    collections::BTreeMap,
    num::{NonZeroU16, NonZeroU32},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_timeline::RationalTime;

use super::{
    ContentDigest, Extent2d, OperatorAlphaBehavior, OperatorColorDomain, VisualInterpretation,
    WorkingAlphaMode,
};

/// Interpretation is deliberately separate from content identity. The same bytes decoded with a
/// different color description, alpha mode, orientation, crop or SAR are different resources.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceInterpretation {
    Visual {
        interpretation: VisualInterpretation,
    },
    FontFace {
        face_index: u32,
    },
    RuntimeShader {
        abi_digest: ContentDigest,
        color_domain: OperatorColorDomain,
        alpha_behavior: OperatorAlphaBehavior,
    },
    Scene3d {
        topology_digest: ContentDigest,
    },
    AudioPcm {
        sample_rate: NonZeroU32,
        channels: NonZeroU16,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ResourceInterpretationWire {
    Visual {
        interpretation: VisualInterpretation,
    },
    FontFace {
        face_index: u32,
    },
    RuntimeShader {
        abi_digest: ContentDigest,
        color_domain: OperatorColorDomain,
        alpha_behavior: OperatorAlphaBehavior,
    },
    Scene3d {
        topology_digest: ContentDigest,
    },
    AudioPcm {
        sample_rate: NonZeroU32,
        channels: NonZeroU16,
    },
}

impl<'de> Deserialize<'de> for ResourceInterpretation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(
            match ResourceInterpretationWire::deserialize(deserializer)? {
                ResourceInterpretationWire::Visual { interpretation } => {
                    Self::Visual { interpretation }
                }
                ResourceInterpretationWire::FontFace { face_index } => {
                    Self::FontFace { face_index }
                }
                ResourceInterpretationWire::RuntimeShader {
                    abi_digest,
                    color_domain,
                    alpha_behavior,
                } => Self::RuntimeShader {
                    abi_digest,
                    color_domain,
                    alpha_behavior,
                },
                ResourceInterpretationWire::Scene3d { topology_digest } => {
                    Self::Scene3d { topology_digest }
                }
                ResourceInterpretationWire::AudioPcm {
                    sample_rate,
                    channels,
                } => Self::AudioPcm {
                    sample_rate,
                    channels,
                },
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceKey {
    pub content: ContentDigest,
    pub interpretation: ResourceInterpretation,
}

impl ResourceKey {
    pub const fn new(content: ContentDigest, interpretation: ResourceInterpretation) -> Self {
        Self {
            content,
            interpretation,
        }
    }
}

/// Stable identifier inside one prepared/render plan. Zero is reserved as an invalid/unbound ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ExternalHandleId(u32);

impl ExternalHandleId {
    pub fn new(value: u32) -> Result<Self, ResourceContractError> {
        if value == 0 {
            Err(ResourceContractError::InvalidHandle)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for ExternalHandleId {
    type Error = ResourceContractError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExternalHandleId> for u32 {
    fn from(value: ExternalHandleId) -> Self {
        value.0
    }
}

/// Host-local incarnation of an [`ExternalObjectTable`](crate::compositor::ExternalObjectTable).
///
/// Handles are intentionally small and may be reused by a later fulfillment batch. Pairing every
/// [`RenderBindings`](crate::compositor::lower::RenderBindings) packet with a non-zero generation
/// prevents an otherwise type-correct stale packet from resolving an ABA-reused handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ExternalGeneration(u64);

impl ExternalGeneration {
    pub fn new(value: u64) -> Result<Self, ResourceContractError> {
        if value == 0 {
            Err(ResourceContractError::InvalidGeneration)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for ExternalGeneration {
    type Error = ResourceContractError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExternalGeneration> for u64 {
    fn from(value: ExternalGeneration) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkingColorSpace {
    LinearRec2020D65,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureFormat {
    Rgba16Float,
    Rgba32Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureUsage {
    Sampled,
    StorageRead,
    StorageWrite,
    ColorAttachment,
    CopySource,
    CopyDestination,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LogicalTextureDescWire", into = "LogicalTextureDescWire")]
pub struct LogicalTextureDesc {
    pub extent: Extent2d,
    pub format: TextureFormat,
    pub working_space: WorkingColorSpace,
    pub alpha: WorkingAlphaMode,
    usages: Vec<TextureUsage>,
    sample_count: u8,
}

impl LogicalTextureDesc {
    pub fn new(
        extent: Extent2d,
        format: TextureFormat,
        usages: impl IntoIterator<Item = TextureUsage>,
        sample_count: u8,
    ) -> Result<Self, ResourceContractError> {
        let mut usages: Vec<_> = usages.into_iter().collect();
        usages.sort_unstable();
        usages.dedup();
        if usages.is_empty() {
            return Err(ResourceContractError::EmptyTextureUsage);
        }
        if !matches!(sample_count, 1 | 2 | 4 | 8) {
            return Err(ResourceContractError::InvalidSampleCount { sample_count });
        }
        Ok(Self {
            extent,
            format,
            working_space: WorkingColorSpace::LinearRec2020D65,
            alpha: WorkingAlphaMode::PremultipliedCoverage,
            usages,
            sample_count,
        })
    }

    pub fn production(
        extent: Extent2d,
        usages: impl IntoIterator<Item = TextureUsage>,
    ) -> Result<Self, ResourceContractError> {
        Self::new(extent, TextureFormat::Rgba16Float, usages, 1)
    }

    pub fn reference(
        extent: Extent2d,
        usages: impl IntoIterator<Item = TextureUsage>,
    ) -> Result<Self, ResourceContractError> {
        Self::new(extent, TextureFormat::Rgba32Float, usages, 1)
    }

    pub fn usages(&self) -> &[TextureUsage] {
        &self.usages
    }

    pub const fn sample_count(&self) -> u8 {
        self.sample_count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogicalTextureDescWire {
    extent: Extent2d,
    format: TextureFormat,
    working_space: WorkingColorSpace,
    alpha: WorkingAlphaMode,
    usages: Vec<TextureUsage>,
    sample_count: u8,
}

impl TryFrom<LogicalTextureDescWire> for LogicalTextureDesc {
    type Error = ResourceContractError;

    fn try_from(value: LogicalTextureDescWire) -> Result<Self, Self::Error> {
        let result = Self::new(value.extent, value.format, value.usages, value.sample_count)?;
        if value.working_space != result.working_space || value.alpha != result.alpha {
            return Err(ResourceContractError::InvalidWorkingContract);
        }
        Ok(result)
    }
}

impl From<LogicalTextureDesc> for LogicalTextureDescWire {
    fn from(value: LogicalTextureDesc) -> Self {
        Self {
            extent: value.extent,
            format: value.format,
            working_space: value.working_space,
            alpha: value.alpha,
            usages: value.usages,
            sample_count: value.sample_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalPixelLayout {
    Rgba8,
    Rgba16Float,
    Nv12,
    P010,
    I420,
}

/// Host-side layout the backend will actually import.
///
/// Prepare may advertise a preferred decoder layout (opaque video → NV12). If that layout is not
/// in the backend's import set, rewrite to a layout the host can produce before
/// `resource_requests()` is observed. Working-surface format is independent of this choice.
pub fn admitted_visual_pixel_layout(
    requested: ExternalPixelLayout,
    supported: &[ExternalPixelLayout],
) -> ExternalPixelLayout {
    if supported.contains(&requested) {
        return requested;
    }
    for candidate in [ExternalPixelLayout::Rgba8, ExternalPixelLayout::Rgba16Float] {
        if supported.contains(&candidate) {
            return candidate;
        }
    }
    requested
}

/// Type contract for an object supplied by a host-owned `ExternalObjectTable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExternalResourceDesc {
    VisualFrame {
        extent: Extent2d,
        pixel_layout: ExternalPixelLayout,
    },
    FontBytes,
    RuntimeShader,
    Scene3d,
    AudioBlock {
        sample_rate: NonZeroU32,
        channels: NonZeroU16,
        frames: NonZeroU32,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ExternalResourceDescWire {
    VisualFrame {
        extent: Extent2d,
        pixel_layout: ExternalPixelLayout,
    },
    FontBytes {},
    RuntimeShader {},
    Scene3d {},
    AudioBlock {
        sample_rate: NonZeroU32,
        channels: NonZeroU16,
        frames: NonZeroU32,
    },
}

impl<'de> Deserialize<'de> for ExternalResourceDesc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match ExternalResourceDescWire::deserialize(deserializer)? {
            ExternalResourceDescWire::VisualFrame {
                extent,
                pixel_layout,
            } => Self::VisualFrame {
                extent,
                pixel_layout,
            },
            ExternalResourceDescWire::FontBytes {} => Self::FontBytes,
            ExternalResourceDescWire::RuntimeShader {} => Self::RuntimeShader,
            ExternalResourceDescWire::Scene3d {} => Self::Scene3d,
            ExternalResourceDescWire::AudioBlock {
                sample_rate,
                channels,
                frames,
            } => Self::AudioBlock {
                sample_rate,
                channels,
                frames,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "time", rename_all = "camelCase")]
pub enum ResourceSample {
    Static,
    SourceTime(RationalTime),
}

/// Pure, bounded fulfillment data that is part of a request identity but never enters a render
/// plan. Scene3D uses it to carry the complete random-access scene/frame request to Native and Web
/// hosts without exposing Motion semantics to the mechanical executor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ResourcePayload {
    Scene3dFrame { canonical_request: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ResourceRequestWire", into = "ResourceRequestWire")]
pub struct ResourceRequest {
    handle: ExternalHandleId,
    key: ResourceKey,
    sample: ResourceSample,
    expected: ExternalResourceDesc,
    payload: Option<ResourcePayload>,
}

/// Exact canonical identity for a host-cached object. It excludes only the frame-local external
/// handle and remains opaque so hosts cannot accidentally build a weaker key from selected fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceCacheIdentity(Vec<u8>);

impl ResourceRequest {
    pub fn new(
        handle: ExternalHandleId,
        key: ResourceKey,
        sample: ResourceSample,
        expected: ExternalResourceDesc,
    ) -> Result<Self, ResourceContractError> {
        Self::new_with_payload(handle, key, sample, expected, None)
    }

    pub fn new_with_payload(
        handle: ExternalHandleId,
        key: ResourceKey,
        sample: ResourceSample,
        expected: ExternalResourceDesc,
        payload: Option<ResourcePayload>,
    ) -> Result<Self, ResourceContractError> {
        if !request_shape_matches(&key.interpretation, sample, &expected, payload.as_ref()) {
            return Err(ResourceContractError::RequestTypeMismatch);
        }
        Ok(Self {
            handle,
            key,
            sample,
            expected,
            payload,
        })
    }

    pub const fn handle(&self) -> ExternalHandleId {
        self.handle
    }

    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    pub const fn sample(&self) -> ResourceSample {
        self.sample
    }

    pub const fn expected(&self) -> &ExternalResourceDesc {
        &self.expected
    }

    pub const fn payload(&self) -> Option<&ResourcePayload> {
        self.payload.as_ref()
    }

    pub fn with_expected(
        &self,
        expected: ExternalResourceDesc,
    ) -> Result<Self, ResourceContractError> {
        Self::new_with_payload(
            self.handle,
            self.key.clone(),
            self.sample,
            expected,
            self.payload.clone(),
        )
    }

    /// Stable host-cache identity. The frame-local handle is deliberately excluded: two prepared
    /// frames may assign different handles to the same immutable content/sample/interpretation,
    /// while any semantic or delivery-contract difference must produce a different identity.
    pub fn cache_identity(&self) -> Result<ResourceCacheIdentity, ResourceContractError> {
        let bytes =
            crate::canonical::bytes(&(&self.key, self.sample, &self.expected, &self.payload))
                .map_err(|_| ResourceContractError::CanonicalCacheIdentity)?;
        Ok(ResourceCacheIdentity(bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceRequestWire {
    handle: ExternalHandleId,
    key: ResourceKey,
    sample: ResourceSample,
    expected: ExternalResourceDesc,
    payload: Option<ResourcePayload>,
}

impl TryFrom<ResourceRequestWire> for ResourceRequest {
    type Error = ResourceContractError;

    fn try_from(value: ResourceRequestWire) -> Result<Self, Self::Error> {
        Self::new_with_payload(
            value.handle,
            value.key,
            value.sample,
            value.expected,
            value.payload,
        )
    }
}

impl From<ResourceRequest> for ResourceRequestWire {
    fn from(value: ResourceRequest) -> Self {
        Self {
            handle: value.handle,
            key: value.key,
            sample: value.sample,
            expected: value.expected,
            payload: value.payload,
        }
    }
}

/// Canonically ordered, immutable request set. Repeating an identical request is idempotent;
/// reusing a handle for a different request is a hard plan error.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "Vec<ResourceRequest>", into = "Vec<ResourceRequest>")]
pub struct ResourceRequestSet {
    requests: Vec<ResourceRequest>,
}

pub const RESOURCE_REQUESTS_FORMAT_VERSION: u32 = 1;

impl ResourceRequestSet {
    pub fn try_from_requests(
        requests: impl IntoIterator<Item = ResourceRequest>,
    ) -> Result<Self, ResourceContractError> {
        let mut by_handle = BTreeMap::new();
        for request in requests {
            match by_handle.get(&request.handle) {
                Some(existing) if existing == &request => {}
                Some(_) => {
                    return Err(ResourceContractError::ConflictingHandle {
                        handle: request.handle,
                    });
                }
                None => {
                    by_handle.insert(request.handle, request);
                }
            }
        }
        Ok(Self {
            requests: by_handle.into_values().collect(),
        })
    }

    pub fn requests(&self) -> &[ResourceRequest] {
        &self.requests
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Versioned binary batch consumed by Native and Web fulfillment hosts.
    pub fn packed_bytes(&self) -> Result<Vec<u8>, crate::compositor::lower::PackedPlanError> {
        crate::compositor::lower::packed::encode(
            self,
            crate::compositor::lower::packed::Contract::requests(RESOURCE_REQUESTS_FORMAT_VERSION),
        )
    }

    pub fn from_packed(bytes: &[u8]) -> Result<Self, crate::compositor::lower::PackedPlanError> {
        crate::compositor::lower::packed::decode(
            bytes,
            crate::compositor::lower::packed::Contract::requests(RESOURCE_REQUESTS_FORMAT_VERSION),
        )
    }
}

impl TryFrom<Vec<ResourceRequest>> for ResourceRequestSet {
    type Error = ResourceContractError;

    fn try_from(value: Vec<ResourceRequest>) -> Result<Self, Self::Error> {
        Self::try_from_requests(value)
    }
}

impl From<ResourceRequestSet> for Vec<ResourceRequest> {
    fn from(value: ResourceRequestSet) -> Self {
        value.requests
    }
}

fn request_shape_matches(
    interpretation: &ResourceInterpretation,
    sample: ResourceSample,
    expected: &ExternalResourceDesc,
    payload: Option<&ResourcePayload>,
) -> bool {
    match (interpretation, sample, expected, payload) {
        (
            ResourceInterpretation::Visual { .. },
            ResourceSample::Static | ResourceSample::SourceTime(_),
            ExternalResourceDesc::VisualFrame { .. },
            None,
        ) => true,
        (
            ResourceInterpretation::FontFace { .. },
            ResourceSample::Static,
            ExternalResourceDesc::FontBytes,
            None,
        ) => true,
        (
            ResourceInterpretation::RuntimeShader { .. },
            ResourceSample::Static,
            ExternalResourceDesc::RuntimeShader,
            None,
        ) => true,
        (
            ResourceInterpretation::Scene3d { .. },
            ResourceSample::Static,
            ExternalResourceDesc::Scene3d,
            Some(ResourcePayload::Scene3dFrame { canonical_request }),
        ) => !canonical_request.is_empty(),
        (
            ResourceInterpretation::AudioPcm {
                sample_rate,
                channels,
            },
            ResourceSample::SourceTime(_),
            ExternalResourceDesc::AudioBlock {
                sample_rate: actual_rate,
                channels: actual_channels,
                ..
            },
            None,
        ) => sample_rate == actual_rate && channels == actual_channels,
        _ => false,
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResourceContractError {
    #[error("external handle id zero is reserved for unbound resources")]
    InvalidHandle,
    #[error("external object table generation zero is reserved")]
    InvalidGeneration,
    #[error("logical texture must declare at least one usage")]
    EmptyTextureUsage,
    #[error("texture sample count must be one of 1, 2, 4 or 8, got {sample_count}")]
    InvalidSampleCount { sample_count: u8 },
    #[error("logical texture must use Linear Rec.2020 D65 premultiplied coverage")]
    InvalidWorkingContract,
    #[error("resource request sample/type does not match its interpretation")]
    RequestTypeMismatch,
    #[error("resource request cache identity could not be canonicalized")]
    CanonicalCacheIdentity,
    #[error("external handle {handle:?} is assigned to conflicting resource requests")]
    ConflictingHandle { handle: ExternalHandleId },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{ColorDescription, InputAlphaMode, PixelOrientation, SignalLuminance};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_hex(&byte.to_string().repeat(64)).unwrap()
    }

    fn request(handle: u32, orientation: PixelOrientation) -> ResourceRequest {
        let interpretation = VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::StraightCoverage,
        )
        .with_orientation(orientation);
        ResourceRequest::new(
            ExternalHandleId::new(handle).unwrap(),
            ResourceKey::new(
                digest('a'),
                ResourceInterpretation::Visual { interpretation },
            ),
            ResourceSample::Static,
            ExternalResourceDesc::VisualFrame {
                extent: Extent2d::new(10, 20).unwrap(),
                pixel_layout: ExternalPixelLayout::Rgba8,
            },
        )
        .unwrap()
    }

    #[test]
    fn visual_pixel_layout_falls_back_to_a_supported_import() {
        assert_eq!(
            admitted_visual_pixel_layout(
                ExternalPixelLayout::Nv12,
                &[ExternalPixelLayout::Nv12, ExternalPixelLayout::Rgba8]
            ),
            ExternalPixelLayout::Nv12
        );
        assert_eq!(
            admitted_visual_pixel_layout(
                ExternalPixelLayout::Nv12,
                &[ExternalPixelLayout::Rgba8, ExternalPixelLayout::Rgba16Float]
            ),
            ExternalPixelLayout::Rgba8
        );
        assert_eq!(
            admitted_visual_pixel_layout(
                ExternalPixelLayout::Rgba16Float,
                &[ExternalPixelLayout::Rgba8]
            ),
            ExternalPixelLayout::Rgba8
        );
        assert_eq!(
            admitted_visual_pixel_layout(ExternalPixelLayout::Nv12, &[ExternalPixelLayout::P010]),
            ExternalPixelLayout::Nv12
        );
    }

    #[test]
    fn request_sets_are_ordered_and_conflicts_fail_closed() {
        let first = request(1, PixelOrientation::Identity);
        let second = request(2, PixelOrientation::Identity);
        let set =
            ResourceRequestSet::try_from_requests([second.clone(), first.clone(), second.clone()])
                .unwrap();
        assert_eq!(
            set.requests()
                .iter()
                .map(|request| request.handle().get())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(matches!(
            ResourceRequestSet::try_from_requests([first, request(1, PixelOrientation::Rotate90)]),
            Err(ResourceContractError::ConflictingHandle { .. })
        ));
    }

    #[test]
    fn cache_identity_excludes_handle_but_includes_sample_and_interpretation() {
        let first = request(1, PixelOrientation::Identity);
        let second = request(2, PixelOrientation::Identity);
        let rotated = request(3, PixelOrientation::Rotate90);
        assert_eq!(
            first.cache_identity().unwrap(),
            second.cache_identity().unwrap()
        );
        assert_ne!(
            first.cache_identity().unwrap(),
            rotated.cache_identity().unwrap()
        );
    }

    #[test]
    fn wire_cannot_smuggle_an_invalid_working_texture() {
        let json = r#"{
          "extent":{"width":16,"height":9},
          "format":"rgba16Float",
          "workingSpace":"linearRec2020D65",
          "alpha":"premultipliedCoverage",
          "usages":[],
          "sampleCount":1
        }"#;
        assert!(serde_json::from_str::<LogicalTextureDesc>(json).is_err());
    }

    #[test]
    fn scene3d_payload_uses_the_outer_request_set_format() {
        let value = serde_json::to_value(ResourcePayload::Scene3dFrame {
            canonical_request: vec![1, 2, 3],
        })
        .unwrap();
        assert_eq!(value["kind"], "scene3dFrame");
        assert!(value.get("contract").is_none());
    }
}
