//! Sealed Skia implementation of the Product Compositor execution contract.
//!
//! The public boundary is deliberately narrow: discover capabilities, construct immutable
//! backend objects, admit a complete frame, and execute it transactionally. Authoring data, Motion
//! artifacts and host scheduling never enter this module.

mod blend;
mod bloom_gpu;
mod blur;
mod cache;
mod chromatic;
mod clip_coverage;
mod color_matrix;
#[cfg(all(target_os = "macos", feature = "native"))]
mod decoded_yuv;
mod draw;
mod effect;
mod f16_gpu;
mod film_gpu;
mod gaussian_gpu;
mod gradient_fill;
mod import;
mod lens_gpu;
mod noise;
mod output;
mod pass;
mod radial_gpu;
mod raster_cache_placement;
mod scene3d_dof;
#[cfg(feature = "native")]
mod scene3d_frames;
#[cfg(feature = "native")]
mod scene3d_gpu;
mod scene3d_region;
#[cfg(all(
    feature = "native",
    any(target_os = "macos", target_os = "linux", target_os = "windows")
))]
mod shader_errors;
mod surface;
mod svg;
mod target;
mod velocity;
mod working_capture;
pub use working_capture::WorkingF16Capture;
#[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
mod vulkan;
#[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
pub(crate) mod vulkan_delivery;

#[cfg(feature = "native")]
pub(crate) use cache::SkiaCacheLimits;
pub(crate) use output::rgba8_target_info;
pub(crate) use pass::planned_frame_bytes;
pub use pass::{
    AdmittedSkiaFrame, SkiaExecuteError, SkiaExecutionReport, SkiaExecutor, SkiaSurfaceReport,
    admit_skia_frame, execute_skia,
};
pub use surface::{
    GlyphCoverageProfile, GlyphEdgingKind, GlyphHintingKind, GlyphLibraryKind, GlyphRasterizerKind,
    SkiaBackendKind, SkiaExecutionProfile,
};
#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) use surface::{SubmittedSharedMetalFrame, draw_shared_raster_frame};
pub use target::{
    SkiaExternalObject, SkiaObjectError, SkiaObjectTable, SkiaTarget, SkiaTargetError,
};

use valle_engine::resource::{Extent2d, ExternalPixelLayout, TextureFormat, TextureUsage};

/// Exact CPU-Skia abilities. GPU discovery uses the same semantic set with backend-specific
/// limits and external import support; neither path leaks a backend-name discriminator into
/// Engine lowering.
pub fn cpu_capabilities(
    max_extent: Extent2d,
    max_surface_bytes: u64,
    max_frame_bytes: u64,
) -> Result<
    valle_engine::compositor::lower::BackendCapabilities,
    valle_engine::compositor::lower::CapabilityContractError,
> {
    use valle_engine::compositor::{graph::GraphCapability, lower::BackendCapabilities};

    BackendCapabilities::new(
        max_extent,
        [TextureFormat::Rgba16Float],
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
            TextureUsage::CopySource,
            TextureUsage::CopyDestination,
        ],
        [1],
        [
            ExternalPixelLayout::Rgba8,
            ExternalPixelLayout::Rgba16Float,
            ExternalPixelLayout::Nv12,
            ExternalPixelLayout::P010,
            ExternalPixelLayout::I420,
        ],
        [
            GraphCapability::Clear,
            GraphCapability::ExternalImport,
            GraphCapability::SourcePipeline,
            GraphCapability::DrawProgram,
            GraphCapability::BackdropRead,
            GraphCapability::Group,
            GraphCapability::Filter,
            GraphCapability::Mask,
            GraphCapability::Blend,
            GraphCapability::Transition,
            GraphCapability::AdjustmentEffect,
            GraphCapability::Caption,
            GraphCapability::OutputTransform,
            GraphCapability::HostFrame,
        ],
        true,
        None,
        max_surface_bytes,
        max_frame_bytes,
    )
}
