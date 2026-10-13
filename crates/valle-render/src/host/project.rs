//! Fixed-render Native project ownership.

use std::sync::Arc;

use valle_engine::{
    product::EngineRender,
    render::{CompiledAudioItem, CompiledRender, CompiledVisualItem, RenderId},
};

use super::{NativeResourceCatalog, NativeResourceProvider};

/// One admitted immutable render plus its explicit host fulfillment catalog.
///
/// The [`EngineRender`] is the sole owner of the compiled graph; no Native operation can reopen
/// authoring data or select a different render after admission.
#[derive(Debug, Clone)]
pub struct NativeProject {
    render: EngineRender,
    catalog: Arc<NativeResourceCatalog>,
}

impl NativeProject {
    pub fn from_render(render: EngineRender, catalog: Arc<NativeResourceCatalog>) -> Self {
        Self { render, catalog }
    }

    pub fn compiled(&self) -> &CompiledRender {
        self.render.compiled()
    }

    pub fn compiled_arc(&self) -> Arc<CompiledRender> {
        self.render.compiled_arc()
    }

    pub const fn render(&self) -> &EngineRender {
        &self.render
    }

    pub fn render_id(&self) -> RenderId {
        self.compiled().render_id()
    }

    pub const fn catalog(&self) -> &Arc<NativeResourceCatalog> {
        &self.catalog
    }

    pub fn has_visual_material(&self) -> bool {
        self.compiled().visual().tracks().iter().any(|track| {
            track.items().iter().any(|item| {
                matches!(
                    item,
                    CompiledVisualItem::Clip { .. } | CompiledVisualItem::Transition { .. }
                )
            })
        }) || !self.compiled().captions().tracks().is_empty()
            || !self.compiled().adjustments().clips().is_empty()
    }

    pub fn has_audio(&self) -> bool {
        self.compiled().audio().tracks().iter().any(|track| {
            track.items().iter().any(|item| {
                matches!(
                    item,
                    CompiledAudioItem::Clip { .. } | CompiledAudioItem::Crossfade { .. }
                )
            })
        })
    }

    pub fn frame_runner(
        &self,
        backend: crate::executor::skia::SkiaBackendKind,
    ) -> Result<super::FrameRunner<NativeResourceProvider>, super::FrameRenderError> {
        super::FrameRunner::for_backend(
            self.render.clone(),
            NativeResourceProvider::new(Arc::clone(&self.catalog))
                .with_render_resources(self.compiled()),
            backend,
        )
    }
}
