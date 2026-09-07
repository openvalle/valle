use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    canonical,
    compositor::graph::GraphCapability,
    resource::{ContentDigest, Extent2d, ExternalPixelLayout, TextureFormat, TextureUsage},
};

/// A backend may advertise framebuffer fetch only when it is coherent with Valle's working
/// linear/premultiplied contract. Other API-specific fetch modes are intentionally inexpressible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FramebufferFetchSemantics {
    CoherentWorkingPremultiplied,
}

/// Pure, backend-independent proof inputs for lowering.
///
/// There is deliberately no backend name or quality fallback flag: two implementations with the
/// same exact abilities must produce the same fingerprint and physical plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BackendCapabilitiesWire", rename_all = "camelCase")]
pub struct BackendCapabilities {
    max_extent: Extent2d,
    formats: Vec<TextureFormat>,
    texture_usages: Vec<TextureUsage>,
    sample_counts: Vec<u8>,
    external_pixel_layouts: Vec<ExternalPixelLayout>,
    graph_capabilities: Vec<GraphCapability>,
    sampleable_render_target: bool,
    framebuffer_fetch: Option<FramebufferFetchSemantics>,
    max_surface_bytes: u64,
    max_frame_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackendCapabilitiesWire {
    max_extent: Extent2d,
    formats: Vec<TextureFormat>,
    texture_usages: Vec<TextureUsage>,
    sample_counts: Vec<u8>,
    external_pixel_layouts: Vec<ExternalPixelLayout>,
    graph_capabilities: Vec<GraphCapability>,
    sampleable_render_target: bool,
    framebuffer_fetch: Option<FramebufferFetchSemantics>,
    max_surface_bytes: u64,
    max_frame_bytes: u64,
}

impl BackendCapabilities {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_extent: Extent2d,
        formats: impl IntoIterator<Item = TextureFormat>,
        texture_usages: impl IntoIterator<Item = TextureUsage>,
        sample_counts: impl IntoIterator<Item = u8>,
        external_pixel_layouts: impl IntoIterator<Item = ExternalPixelLayout>,
        graph_capabilities: impl IntoIterator<Item = GraphCapability>,
        sampleable_render_target: bool,
        framebuffer_fetch: Option<FramebufferFetchSemantics>,
        max_surface_bytes: u64,
        max_frame_bytes: u64,
    ) -> Result<Self, CapabilityContractError> {
        let formats = canonical_set(formats);
        let texture_usages = canonical_set(texture_usages);
        let sample_counts = canonical_set(sample_counts);
        let external_pixel_layouts = canonical_set(external_pixel_layouts);
        let graph_capabilities = canonical_set(graph_capabilities);
        if formats.is_empty() {
            return Err(CapabilityContractError::EmptyFormats);
        }
        if texture_usages.is_empty() {
            return Err(CapabilityContractError::EmptyTextureUsages);
        }
        if sample_counts.is_empty() {
            return Err(CapabilityContractError::EmptySampleCounts);
        }
        if let Some(sample_count) = sample_counts
            .iter()
            .copied()
            .find(|value| !matches!(value, 1 | 2 | 4 | 8))
        {
            return Err(CapabilityContractError::InvalidSampleCount { sample_count });
        }
        if graph_capabilities.contains(&GraphCapability::ExternalImport)
            && external_pixel_layouts.is_empty()
        {
            return Err(CapabilityContractError::ExternalImportWithoutLayouts);
        }
        if max_surface_bytes == 0 || max_frame_bytes == 0 || max_surface_bytes > max_frame_bytes {
            return Err(CapabilityContractError::InvalidMemoryBudget {
                max_surface_bytes,
                max_frame_bytes,
            });
        }
        Ok(Self {
            max_extent,
            formats,
            texture_usages,
            sample_counts,
            external_pixel_layouts,
            graph_capabilities,
            sampleable_render_target,
            framebuffer_fetch,
            max_surface_bytes,
            max_frame_bytes,
        })
    }

    pub const fn max_extent(&self) -> Extent2d {
        self.max_extent
    }

    pub fn formats(&self) -> &[TextureFormat] {
        &self.formats
    }

    pub fn texture_usages(&self) -> &[TextureUsage] {
        &self.texture_usages
    }

    pub fn sample_counts(&self) -> &[u8] {
        &self.sample_counts
    }

    pub fn external_pixel_layouts(&self) -> &[ExternalPixelLayout] {
        &self.external_pixel_layouts
    }

    pub fn graph_capabilities(&self) -> &[GraphCapability] {
        &self.graph_capabilities
    }

    pub const fn sampleable_render_target(&self) -> bool {
        self.sampleable_render_target
    }

    pub const fn framebuffer_fetch(&self) -> Option<FramebufferFetchSemantics> {
        self.framebuffer_fetch
    }

    pub const fn max_surface_bytes(&self) -> u64 {
        self.max_surface_bytes
    }

    pub const fn max_frame_bytes(&self) -> u64 {
        self.max_frame_bytes
    }

    pub fn fingerprint(&self) -> Result<ContentDigest, CapabilityContractError> {
        self.validate()?;
        let bytes = canonical::bytes(self)
            .map_err(|error| CapabilityContractError::Canonical(error.to_string()))?;
        Ok(ContentDigest::of_bytes(&bytes))
    }

    pub fn validate(&self) -> Result<(), CapabilityContractError> {
        let canonical = Self::new(
            self.max_extent,
            self.formats.iter().copied(),
            self.texture_usages.iter().copied(),
            self.sample_counts.iter().copied(),
            self.external_pixel_layouts.iter().copied(),
            self.graph_capabilities.iter().copied(),
            self.sampleable_render_target,
            self.framebuffer_fetch,
            self.max_surface_bytes,
            self.max_frame_bytes,
        )?;
        if &canonical != self {
            return Err(CapabilityContractError::NonCanonicalSet);
        }
        Ok(())
    }
}

impl TryFrom<BackendCapabilitiesWire> for BackendCapabilities {
    type Error = CapabilityContractError;

    fn try_from(value: BackendCapabilitiesWire) -> Result<Self, Self::Error> {
        let original_formats = value.formats.clone();
        let original_usages = value.texture_usages.clone();
        let original_sample_counts = value.sample_counts.clone();
        let original_external_layouts = value.external_pixel_layouts.clone();
        let original_graph = value.graph_capabilities.clone();
        let result = Self::new(
            value.max_extent,
            value.formats,
            value.texture_usages,
            value.sample_counts,
            value.external_pixel_layouts,
            value.graph_capabilities,
            value.sampleable_render_target,
            value.framebuffer_fetch,
            value.max_surface_bytes,
            value.max_frame_bytes,
        )?;
        if result.formats != original_formats
            || result.texture_usages != original_usages
            || result.sample_counts != original_sample_counts
            || result.external_pixel_layouts != original_external_layouts
            || result.graph_capabilities != original_graph
        {
            return Err(CapabilityContractError::NonCanonicalSet);
        }
        Ok(result)
    }
}

fn canonical_set<T: Ord>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort_unstable();
    values.dedup();
    values
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CapabilityContractError {
    #[error("backend capability contract must support at least one texture format")]
    EmptyFormats,
    #[error("backend capability contract must support at least one texture usage")]
    EmptyTextureUsages,
    #[error("backend capability contract must support at least one texture sample count")]
    EmptySampleCounts,
    #[error("backend capability contract declares invalid sample count {sample_count}")]
    InvalidSampleCount { sample_count: u8 },
    #[error("ExternalImport capability requires at least one external pixel layout")]
    ExternalImportWithoutLayouts,
    #[error(
        "invalid memory budget: max surface {max_surface_bytes} bytes, max frame {max_frame_bytes} bytes"
    )]
    InvalidMemoryBudget {
        max_surface_bytes: u64,
        max_frame_bytes: u64,
    },
    #[error("capability sets must be unique and in canonical enum order")]
    NonCanonicalSet,
    #[error("capability fingerprint canonicalization failed: {0}")]
    Canonical(String),
}
