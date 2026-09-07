use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::{
    canonical,
    prepare::bounds::{DeviceRect, DeviceTransform},
    resource::{
        ContentDigest, ExternalHandleId, ExternalPixelLayout, ExternalResourceDesc,
        ResourceContractError, ResourceInterpretation, ResourceKey, ResourcePayload,
        ResourceRequest, ResourceRequestSet, ResourceSample, SemanticAsset, SemanticAssetKind,
        admitted_visual_pixel_layout,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct DynamicBindingId(u32);

impl DynamicBindingId {
    fn new(value: u32) -> Result<Self, RequestError> {
        if value == 0 {
            Err(RequestError::BindingBudgetExceeded)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for DynamicBindingId {
    type Error = RequestError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DynamicBindingId> for u32 {
    fn from(value: DynamicBindingId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DynamicBindingKind {
    DeviceTransform,
    DeviceLength,
    Bounds,
    Opacity,
    TransitionProgress,
    BackdropSampleBounds,
    BackdropOutputBounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum DynamicValue {
    DeviceTransform(DeviceTransform),
    Bounds(DeviceRect),
    Scalar(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DynamicBinding {
    pub id: DynamicBindingId,
    pub semantic_path: String,
    pub binding_kind: DynamicBindingKind,
    pub value: DynamicValue,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(transparent)]
pub struct DynamicBindings {
    values: Vec<DynamicBinding>,
}

impl DynamicBindings {
    fn try_from_values(values: Vec<DynamicBinding>) -> Result<Self, RequestError> {
        validate_dynamic_values(&values)?;
        Ok(Self { values })
    }

    pub fn validate(&self) -> Result<(), RequestError> {
        validate_dynamic_values(&self.values)
    }

    pub fn values(&self) -> &[DynamicBinding] {
        &self.values
    }

    pub fn get(&self, id: DynamicBindingId) -> Option<&DynamicBinding> {
        self.values.get(usize::try_from(id.get() - 1).ok()?)
    }
}

fn validate_dynamic_values(values: &[DynamicBinding]) -> Result<(), RequestError> {
    for (index, binding) in values.iter().enumerate() {
        let expected = u32::try_from(index + 1).map_err(|_| RequestError::BindingBudgetExceeded)?;
        if binding.id.get() != expected
            || binding.semantic_path.is_empty()
            || validate_dynamic(binding.binding_kind, &binding.value).is_err()
        {
            return Err(RequestError::InvalidDynamicBinding);
        }
    }
    Ok(())
}

impl<'de> Deserialize<'de> for DynamicBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<DynamicBinding>::deserialize(deserializer)?;
        Self::try_from_values(values).map_err(de::Error::custom)
    }
}

#[derive(Debug, Default)]
pub(crate) struct DynamicAllocator {
    values: Vec<DynamicBinding>,
}

impl DynamicAllocator {
    pub(crate) fn push(
        &mut self,
        semantic_path: impl Into<String>,
        binding_kind: DynamicBindingKind,
        value: DynamicValue,
    ) -> Result<DynamicBindingId, RequestError> {
        validate_dynamic(binding_kind, &value)?;
        let next = self
            .values
            .len()
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(RequestError::BindingBudgetExceeded)?;
        let id = DynamicBindingId::new(next)?;
        self.values.push(DynamicBinding {
            id,
            semantic_path: semantic_path.into(),
            binding_kind,
            value,
        });
        Ok(id)
    }

    pub(crate) fn finish(self) -> DynamicBindings {
        DynamicBindings {
            values: self.values,
        }
    }
}

pub(crate) fn validate_dynamic(
    kind: DynamicBindingKind,
    value: &DynamicValue,
) -> Result<(), RequestError> {
    let valid = match (kind, value) {
        (DynamicBindingKind::DeviceTransform, DynamicValue::DeviceTransform(transform)) => {
            transform.matrix().iter().all(|value| value.is_finite())
        }
        (DynamicBindingKind::DeviceLength, DynamicValue::Scalar(value)) => {
            value.is_finite() && (0.0..=super::bounds::MAX_DEVICE_COORDINATE).contains(value)
        }
        (
            DynamicBindingKind::Bounds
            | DynamicBindingKind::BackdropSampleBounds
            | DynamicBindingKind::BackdropOutputBounds,
            DynamicValue::Bounds(_),
        ) => true,
        (
            DynamicBindingKind::Opacity | DynamicBindingKind::TransitionProgress,
            DynamicValue::Scalar(value),
        ) => value.is_finite() && (0.0..=1.0).contains(value),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(RequestError::InvalidDynamicBinding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedResource {
    pub request: ResourceRequest,
    pub semantic_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceManifest {
    resources: Vec<PreparedResource>,
}

impl ResourceManifest {
    pub fn resources(&self) -> &[PreparedResource] {
        &self.resources
    }

    pub fn adapt_visual_pixel_layouts(
        &mut self,
        supported: &[ExternalPixelLayout],
    ) -> Result<(), ResourceContractError> {
        for resource in &mut self.resources {
            let ExternalResourceDesc::VisualFrame {
                extent,
                pixel_layout,
            } = *resource.request.expected()
            else {
                continue;
            };
            let admitted = admitted_visual_pixel_layout(pixel_layout, supported);
            if admitted == pixel_layout {
                continue;
            }
            resource.request =
                resource
                    .request
                    .with_expected(ExternalResourceDesc::VisualFrame {
                        extent,
                        pixel_layout: admitted,
                    })?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct RequestAllocator {
    by_identity: BTreeMap<Vec<u8>, usize>,
    resources: Vec<PreparedResource>,
}

impl RequestAllocator {
    pub(crate) fn asset(
        &mut self,
        asset: &SemanticAsset,
        sample: ResourceSample,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        let interpretation = match asset.kind {
            SemanticAssetKind::Video | SemanticAssetKind::Image | SemanticAssetKind::Lottie => {
                ResourceInterpretation::Visual {
                    interpretation: asset
                        .descriptor
                        .visual_interpretation()
                        .ok_or(RequestError::InvalidVisualDescriptor)?,
                }
            }
            SemanticAssetKind::Audio => return Err(RequestError::VisualRequestedAsAudio),
        };
        let expected = match asset.kind {
            SemanticAssetKind::Video => super::video::expected(asset),
            SemanticAssetKind::Image | SemanticAssetKind::Lottie => super::image::expected(asset),
            SemanticAssetKind::Audio => unreachable!("rejected above"),
        };
        self.allocate(
            ResourceKey::new(asset.digest.clone(), interpretation),
            sample,
            expected,
            path,
        )
    }

    pub(crate) fn font(
        &mut self,
        digest: ContentDigest,
        face_index: u32,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        self.allocate(
            ResourceKey::new(digest, ResourceInterpretation::FontFace { face_index }),
            ResourceSample::Static,
            ExternalResourceDesc::FontBytes,
            path,
        )
    }

    pub(crate) fn runtime_shader(
        &mut self,
        content: ContentDigest,
        abi_digest: ContentDigest,
        color_domain: crate::resource::OperatorColorDomain,
        alpha_behavior: crate::resource::OperatorAlphaBehavior,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        self.allocate(
            ResourceKey::new(
                content,
                ResourceInterpretation::RuntimeShader {
                    abi_digest,
                    color_domain,
                    alpha_behavior,
                },
            ),
            ResourceSample::Static,
            ExternalResourceDesc::RuntimeShader,
            path,
        )
    }

    pub(crate) fn scene3d(
        &mut self,
        content: ContentDigest,
        topology_digest: ContentDigest,
        canonical_request: Vec<u8>,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        self.allocate_with_payload(
            ResourceKey::new(content, ResourceInterpretation::Scene3d { topology_digest }),
            ResourceSample::Static,
            ExternalResourceDesc::Scene3d,
            Some(ResourcePayload::Scene3dFrame { canonical_request }),
            path,
        )
    }

    fn allocate(
        &mut self,
        key: ResourceKey,
        sample: ResourceSample,
        expected: ExternalResourceDesc,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        self.allocate_with_payload(key, sample, expected, None, path)
    }

    fn allocate_with_payload(
        &mut self,
        key: ResourceKey,
        sample: ResourceSample,
        expected: ExternalResourceDesc,
        payload: Option<ResourcePayload>,
        path: &str,
    ) -> Result<ExternalHandleId, RequestError> {
        let identity = canonical::bytes(&(&key, sample, &expected, &payload))
            .map_err(|_| RequestError::CanonicalIdentity)?;
        if let Some(index) = self.by_identity.get(&identity).copied() {
            let resource = &mut self.resources[index];
            if !resource
                .semantic_paths
                .iter()
                .any(|existing| existing == path)
            {
                resource.semantic_paths.push(path.to_owned());
            }
            return Ok(resource.request.handle());
        }
        let value = self
            .resources
            .len()
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(RequestError::ResourceBudgetExceeded)?;
        let handle = ExternalHandleId::new(value)?;
        let request = ResourceRequest::new_with_payload(handle, key, sample, expected, payload)?;
        let index = self.resources.len();
        self.resources.push(PreparedResource {
            request,
            semantic_paths: vec![path.to_owned()],
        });
        self.by_identity.insert(identity, index);
        Ok(handle)
    }

    pub(crate) fn finish(self) -> Result<(ResourceManifest, ResourceRequestSet), RequestError> {
        let requests = ResourceRequestSet::try_from_requests(
            self.resources
                .iter()
                .map(|resource| resource.request.clone()),
        )?;
        Ok((
            ResourceManifest {
                resources: self.resources,
            },
            requests,
        ))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RequestError {
    #[error("dynamic binding id budget exceeded")]
    BindingBudgetExceeded,
    #[error("execution resource handle budget exceeded")]
    ResourceBudgetExceeded,
    #[error("dynamic binding kind and value do not match or the value is outside its domain")]
    InvalidDynamicBinding,
    #[error("visual semantic asset has no visual descriptor")]
    InvalidVisualDescriptor,
    #[error("an audio asset was requested by the visual compositor")]
    VisualRequestedAsAudio,
    #[error("resource identity could not be canonicalized")]
    CanonicalIdentity,
    #[error(transparent)]
    Contract(#[from] ResourceContractError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{InputAlphaMode, MediaDescriptor, SemanticAsset, SemanticAssetKind};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_hex(&byte.to_string().repeat(64)).unwrap()
    }

    #[test]
    fn requests_deduplicate_by_frozen_identity_not_semantic_path() {
        let asset = SemanticAsset::new(
            "image",
            SemanticAssetKind::Image,
            digest('a'),
            MediaDescriptor::image_srgb(2, 2, InputAlphaMode::StraightCoverage).unwrap(),
        );
        let mut allocator = RequestAllocator::default();
        let first = allocator
            .asset(&asset, ResourceSample::Static, "visual[0]")
            .unwrap();
        let second = allocator
            .asset(&asset, ResourceSample::Static, "visual[1]")
            .unwrap();
        assert_eq!(first, second);
        let (manifest, requests) = allocator.finish().unwrap();
        assert_eq!(requests.requests().len(), 1);
        assert_eq!(manifest.resources()[0].semantic_paths.len(), 2);
    }
}
