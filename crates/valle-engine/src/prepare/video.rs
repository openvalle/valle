use crate::resource::{ExternalPixelLayout, ExternalResourceDesc, InputAlphaMode, SemanticAsset};

/// Preferred host-side layout, not a working-surface format.
///
/// Opaque video prefers NV12 so Native Metal can import a decoder CVPixelBuffer without a CPU
/// round-trip. CanvasKit cannot import NV12; the WASM product path rewrites unsupported layouts
/// with [`crate::prepare::PrepareOutput::adapt_external_pixel_layouts`] before
/// `resource_requests()` is observed. Do not change this preferred layout to RGBA just to
/// unstick Web — that would drop the Metal zero-copy path.
pub(crate) fn expected(asset: &SemanticAsset) -> ExternalResourceDesc {
    let interpretation = asset
        .descriptor
        .visual_interpretation()
        .expect("compile preflight guarantees video interpretation");
    let pixel_layout = if interpretation.alpha == InputAlphaMode::Opaque {
        ExternalPixelLayout::Nv12
    } else {
        ExternalPixelLayout::Rgba16Float
    };
    ExternalResourceDesc::VisualFrame {
        extent: asset
            .descriptor
            .extent()
            .expect("compile preflight guarantees video extent"),
        pixel_layout,
    }
}
