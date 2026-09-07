//! Validated ResourceManifest domain wrapper.
//!
//! A manifest can acquire canonical bytes only through the strict JSON,
//! wire-shape, and local-invariant pipeline in this file.

use std::collections::HashSet;

use crate::internal::{
    canonical_json::{self, CanonicalJsonError},
    wire::resource::{FontVariationAxisWire, ResourceEntryWire, ResourceManifestEnvelopeWire},
};

/// A locally valid, canonicalizable ResourceManifest.
///
/// Fields stay private and this type intentionally does not implement
/// `Deserialize`; untrusted input must go through [`decode_resource_manifest`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceManifest {
    wire: ResourceManifestEnvelopeWire,
    canonical_bytes: Vec<u8>,
}

impl ResourceManifest {
    /// Close local invariants for an already decoded wire value.
    pub fn try_from_wire(
        wire: ResourceManifestEnvelopeWire,
    ) -> Result<Self, ResourceManifestError> {
        validate_manifest(&wire)?;
        let canonical_bytes = canonical_json::to_canonical_bytes(&wire).map_err(map_json_error)?;
        Ok(Self {
            wire,
            canonical_bytes,
        })
    }

    pub fn wire(&self) -> &ResourceManifestEnvelopeWire {
        &self.wire
    }

    pub fn entries(&self) -> &std::collections::BTreeMap<String, ResourceEntryWire> {
        &self.wire.entries
    }

    pub fn to_wire(&self) -> ResourceManifestEnvelopeWire {
        self.wire.clone()
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

/// Stable failures for the complete manifest construction pipeline.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceManifestError {
    #[error("UTF-8 BOM is not allowed")]
    Utf8Bom,
    #[error("resource manifest contains invalid Unicode")]
    InvalidUnicode,
    #[error("resource manifest contains duplicate object key `{key}`")]
    DuplicateObjectKey { key: String },
    #[error("resource manifest contains unsafe integer `{value}`")]
    UnsafeInteger { value: String },
    #[error("resource manifest contains a non-finite number")]
    NonFiniteNumber,
    #[error("resource manifest is not valid JSON")]
    MalformedJson,
    #[error("resource manifest wire shape is invalid")]
    InvalidWireShape,
    #[error("resource id must not be empty")]
    EmptyResourceId,
    #[error("resource id `{resource_id}` is not a valid namespaced ResourceId")]
    InvalidResourceId { resource_id: String },
    #[error("duplicate resource id `{resource_id}`")]
    DuplicateResourceId { resource_id: String },
    #[error("resource `{resource_id}` has an invalid `{field}` descriptor field")]
    InvalidDescriptor {
        resource_id: String,
        field: &'static str,
    },
    #[error("resource manifest canonical encoding failed")]
    CanonicalEncoding,
}

/// Decode, validate, and canonicalize one complete manifest.
pub fn decode_resource_manifest(input: &[u8]) -> Result<ResourceManifest, ResourceManifestError> {
    let strict_value = canonical_json::parse_strict(input).map_err(map_json_error)?;

    let wire: ResourceManifestEnvelopeWire = serde_json::from_value(strict_value)
        .map_err(|_| ResourceManifestError::InvalidWireShape)?;
    ResourceManifest::try_from_wire(wire)
}

fn validate_manifest(wire: &ResourceManifestEnvelopeWire) -> Result<(), ResourceManifestError> {
    let mut resource_ids = HashSet::with_capacity(wire.entries.len());

    for (resource_id, entry) in &wire.entries {
        if resource_id.is_empty() {
            return Err(ResourceManifestError::EmptyResourceId);
        }
        if !is_valid_resource_id(resource_id) {
            return Err(ResourceManifestError::InvalidResourceId {
                resource_id: resource_id.clone(),
            });
        }
        if !resource_ids.insert(resource_id.as_str()) {
            return Err(ResourceManifestError::DuplicateResourceId {
                resource_id: resource_id.clone(),
            });
        }

        validate_entry(resource_id, entry)?;
    }

    Ok(())
}

fn is_valid_resource_id(id: &str) -> bool {
    id.len() <= 256
        && id.split_once(':').is_some_and(|(namespace, name)| {
            !namespace.is_empty()
                && !name.is_empty()
                && !namespace.contains('/')
                && !id.contains("://")
                && !id.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        })
}

fn validate_entry(
    resource_id: &str,
    entry: &ResourceEntryWire,
) -> Result<(), ResourceManifestError> {
    match entry {
        ResourceEntryWire::Video { descriptor, .. } => {
            require_positive_time(resource_id, "duration", descriptor.duration.is_positive())?;
            require_positive_time(resource_id, "timeBase", descriptor.time_base.is_positive())?;
            require_positive_u32(resource_id, "width", descriptor.width)?;
            require_positive_u32(resource_id, "height", descriptor.height)?;
        }
        ResourceEntryWire::Audio { descriptor, .. } => {
            require_positive_time(resource_id, "duration", descriptor.duration.is_positive())?;
            require_positive_time(resource_id, "timeBase", descriptor.time_base.is_positive())?;
            require_positive_u32(resource_id, "sampleRate", descriptor.sample_rate)?;
        }
        ResourceEntryWire::Image { descriptor, .. } => {
            require_positive_u32(resource_id, "width", descriptor.width)?;
            require_positive_u32(resource_id, "height", descriptor.height)?;
        }
        ResourceEntryWire::Lottie { descriptor, .. } => {
            require_positive_time(resource_id, "duration", descriptor.duration.is_positive())?;
            require_positive_time(resource_id, "timeBase", descriptor.time_base.is_positive())?;
            require_positive_u32(resource_id, "width", descriptor.width)?;
            require_positive_u32(resource_id, "height", descriptor.height)?;
        }
        ResourceEntryWire::Font { descriptor, .. } => {
            for (axis_tag, axis) in &descriptor.variation_axes {
                if !is_valid_axis_tag(axis_tag) || !is_valid_axis(axis) {
                    return invalid_descriptor(resource_id, "variationAxes");
                }
            }
        }
        ResourceEntryWire::MotionArtifact { .. } | ResourceEntryWire::Shader { .. } => {}
    }

    Ok(())
}

fn require_positive_time(
    resource_id: &str,
    field: &'static str,
    positive: bool,
) -> Result<(), ResourceManifestError> {
    if positive {
        Ok(())
    } else {
        invalid_descriptor(resource_id, field)
    }
}

fn require_positive_u32(
    resource_id: &str,
    field: &'static str,
    value: u32,
) -> Result<(), ResourceManifestError> {
    if value > 0 {
        Ok(())
    } else {
        invalid_descriptor(resource_id, field)
    }
}

fn is_valid_axis_tag(tag: &str) -> bool {
    tag.len() == 4 && tag.bytes().all(|byte| byte.is_ascii_graphic())
}

fn is_valid_axis(axis: &FontVariationAxisWire) -> bool {
    axis.minimum.is_finite()
        && axis.default.is_finite()
        && axis.maximum.is_finite()
        && axis.minimum <= axis.default
        && axis.default <= axis.maximum
}

fn invalid_descriptor<T>(
    resource_id: &str,
    field: &'static str,
) -> Result<T, ResourceManifestError> {
    Err(ResourceManifestError::InvalidDescriptor {
        resource_id: resource_id.to_owned(),
        field,
    })
}

fn map_json_error(error: CanonicalJsonError) -> ResourceManifestError {
    match error {
        CanonicalJsonError::Utf8Bom => ResourceManifestError::Utf8Bom,
        CanonicalJsonError::InvalidUnicode => ResourceManifestError::InvalidUnicode,
        CanonicalJsonError::DuplicateObjectKey { key } => {
            ResourceManifestError::DuplicateObjectKey { key }
        }
        CanonicalJsonError::UnsafeInteger { value } => {
            ResourceManifestError::UnsafeInteger { value }
        }
        CanonicalJsonError::NonFiniteNumber => ResourceManifestError::NonFiniteNumber,
        CanonicalJsonError::MalformedJson => ResourceManifestError::MalformedJson,
        CanonicalJsonError::Encode => ResourceManifestError::CanonicalEncoding,
    }
}
