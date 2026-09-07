use crate::resource::{ExternalPixelLayout, ExternalResourceDesc, SemanticAsset};

pub(crate) fn expected(asset: &SemanticAsset) -> ExternalResourceDesc {
    ExternalResourceDesc::VisualFrame {
        extent: asset
            .descriptor
            .extent()
            .expect("compile preflight guarantees image extent"),
        pixel_layout: ExternalPixelLayout::Rgba8,
    }
}
