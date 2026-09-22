//! Whole-scene geometry reuse for fixed text/layout and dynamic 2D transforms/opacity only.
use super::scene::{LayoutTimings, build_tree_inner};
use super::{LayoutError, LayoutOptions, LayoutTree, PreparedScene, StyleCache};
use crate::{MotionContext, NodeKind, ResolvedProps, SceneArtifact, StyleValue, TextValue};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use takumi_core::{layout::tree::LayoutResults, resources::font::Fonts, viewport::Viewport};

pub(super) fn eligible(artifact: &SceneArtifact) -> bool {
    if artifact.camera.is_some()
        || crate::post_layout_dependent(&artifact.exprs)
            .into_iter()
            .any(|v| v)
    {
        return false;
    }
    // Props and layout environment are fixed for a GeometryKey. Time/cue/unit inputs are not.
    // Work on the compiled dependency graph so helpers cannot hide a time-dependent layout input.
    let mut stable = Vec::with_capacity(artifact.exprs.len());
    for expr in &artifact.exprs {
        let own = match expr {
            crate::Expr::Context { input } => matches!(
                input,
                crate::ContextInput::ViewportWidth
                    | crate::ContextInput::ViewportHeight
                    | crate::ContextInput::FpsNum
                    | crate::ContextInput::FpsDen
                    | crate::ContextInput::DurationFrames
            ),
            crate::Expr::NodeBounds { .. } | crate::Expr::Project3D { .. } => false,
            _ => true,
        };
        stable.push(own && expr.children().iter().all(|id| stable[id.0 as usize]));
    }
    artifact.nodes.iter().all(|node| {
        matches!(
            node.kind,
            NodeKind::Group
                | NodeKind::Box
                | NodeKind::Text {
                    text: TextValue::Static { .. },
                    per_unit: None,
                    path: None
                }
        ) && node.space.is_none()
            && node.visibility.is_none()
            && node.class_names.iter().all(|class| {
                crate::tailwind::explicit_property(class)
                    .is_none_or(|name| crate::style::property_spec(name).can_reuse_geometry(true))
            })
            && node
                .class_conditions
                .values()
                .all(|expr| stable[expr.0 as usize])
            && node.styles.iter().all(|s| {
                let frame_stable = match s.value {
                    StyleValue::Static { .. } => true,
                    StyleValue::Expr { expr } => stable[expr.0 as usize],
                };
                crate::style::property_spec(&s.property).can_reuse_geometry(frame_stable)
            })
    })
}

#[derive(Clone, PartialEq)]
pub(super) struct GeometryKey {
    viewport: (Option<u32>, Option<u32>, u32, u32),
    props: ResolvedProps,
    fps: valle_timeline::FrameRate,
    duration: u32,
}
impl GeometryKey {
    pub(super) fn new(viewport: Viewport, props: &ResolvedProps, ctx: &MotionContext) -> Self {
        Self {
            viewport: (
                viewport.size.width,
                viewport.size.height,
                viewport.font_size.to_bits(),
                viewport.device_pixel_ratio.to_bits(),
            ),
            props: props.clone(),
            fps: ctx.fps,
            duration: ctx.duration_frames,
        }
    }
}

#[derive(Clone)]
pub(super) struct GeometrySnapshot {
    pub key: GeometryKey,
    pub layout: Arc<LayoutResults>,
    pub keys: Arc<HashMap<u64, String>>,
    pub boxes: Arc<BTreeMap<String, valle_draw::Rect>>,
    pub scene3d_boxes: Arc<BTreeMap<String, valle_draw::Rect>>,
}

/// One admitted scene and immutable font registry with at most one geometry snapshot. It never
/// retains a previous RenderNode, style, expression value or DrawProgram. Changing fonts requires
/// a new cache, so a mutable external Fonts registry cannot silently invalidate existing geometry.
pub struct LayoutCache {
    prepared: PreparedScene,
    fonts: Fonts,
    geometry: RefCell<Option<GeometrySnapshot>>,
    #[cfg(target_arch = "wasm32")]
    formula_fonts: crate::math_formula::FormulaFontRegistry,
}
impl LayoutCache {
    pub fn new(prepared: &PreparedScene, fonts: &Fonts) -> Option<Self> {
        prepared.can_reuse_layout().then(|| Self {
            prepared: prepared.clone(),
            fonts: fonts.clone(),
            geometry: RefCell::new(None),
            #[cfg(target_arch = "wasm32")]
            formula_fonts: crate::math_formula::FormulaFontRegistry::new(),
        })
    }
    pub fn is_for(&self, prepared: &PreparedScene) -> bool {
        self.prepared.same_instance(prepared)
    }
    pub fn clear(&self) {
        self.geometry.replace(None);
    }
    pub fn build_tree(
        &self,
        ctx: &MotionContext,
        props: &ResolvedProps,
        viewport: Viewport,
        styles: Option<&StyleCache>,
    ) -> Result<LayoutTree, LayoutError> {
        self.build(ctx, props, viewport, styles, None)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn build_tree_profiled(
        &self,
        ctx: &MotionContext,
        props: &ResolvedProps,
        viewport: Viewport,
        styles: Option<&StyleCache>,
    ) -> Result<(LayoutTree, LayoutTimings), LayoutError> {
        let mut timings = LayoutTimings::default();
        let tree = self.build(ctx, props, viewport, styles, Some(&mut timings))?;
        Ok((tree, timings))
    }
    fn build(
        &self,
        ctx: &MotionContext,
        props: &ResolvedProps,
        viewport: Viewport,
        styles: Option<&StyleCache>,
        timings: Option<&mut LayoutTimings>,
    ) -> Result<LayoutTree, LayoutError> {
        let opts = LayoutOptions {
            viewport,
            fonts: &self.fonts,
            styles,
            #[cfg(target_arch = "wasm32")]
            formula_fonts: &self.formula_fonts,
        };
        build_tree_inner(
            &self.prepared,
            ctx,
            props,
            &opts,
            timings,
            Some(&self.geometry),
        )
    }
}
