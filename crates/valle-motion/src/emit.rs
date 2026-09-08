//! Translate Takumi stacking contexts into backend-neutral recording commands. Reuse upstream paint
//! order and shaping. Paint outer shadows, background, inner shadows, borders, content, and outline
//! in CSS order. Apply filters inside group opacity/blending and report unsupported features
//! explicitly rather than silently omitting them.

use std::collections::{BTreeMap, HashSet};

use takumi_core::geometry::NodeId;
use takumi_core::layout::background::{
    ResolveBackgroundLayersInput, background_origin_box, resolve_background_layers,
};
use takumi_core::layout::inline::{
    InlineBoxItem, InlineLayoutMode, InlineLayoutRequest, PositionedInlineRun, ProcessedInlineSpan,
    ShapedRun, VisualInlineBox, collect_inline_items, create_inline_layout, resolve_inline_runs,
};
use takumi_core::layout::tree::RenderNode;
use takumi_core::resources::glyph::ResolvedGlyph;
use takumi_core::scene::{NodePaint, PaintItemKind, StackingContextNode, build_stacking_contexts};
use takumi_core::style::{
    Affine as TAffine, Color, ComputedStyle, TextTransform, ToCss, WhiteSpaceCollapse,
};
use valle_draw::draw::Span;
use valle_draw::program::recording::{
    Affine, BatchGeometry, BatchInstance, FillRule, FontFace, Glyph, GlyphSource, Homography,
    MaskSource, Paint, ProgramRecording, RecordCmd, RoundRect, ShaderTextureInput, Stroke,
};
use valle_draw::{Cap, Join, Point, Rect, Rgba};

use crate::layout::bridge::{LayoutTree, PerspectiveLength, ResolvedPaint, ResolvedStroke};

/// Structured local drawing program plus explicit unsupported-feature accounting.
pub struct EmitReport {
    pub program: valle_draw::program::DrawProgram,
    /// Complete frame-local Scene3D requests referenced by typed `Scene3d` program nodes. Hosts
    /// fulfill these like any other content-addressed resource; the executor sees only the bound
    /// raster object.
    pub scene3d_frames: Vec<crate::Scene3DFrameRequest>,
    /// Unsupported features encountered during emission, paired with node keys for caller
    /// diagnostics.
    pub unsupported: Vec<(String, &'static str)>,
}

struct EmissionState {
    recording: ProgramRecording,
    scene3d_frames: Vec<crate::Scene3DFrameRequest>,
    unsupported: Vec<(String, &'static str)>,
}

/// Emission failure. Reject inconsistent layout or scene state rather than returning an empty
/// recording that appears successful.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitError {
    /// Root layout results are unavailable.
    RootLayout(String),
    /// Stacking-context construction failed.
    Scene(String),
    /// A per-node paint extra could not be resolved against the layout context.
    BadStyle {
        node: String,
        reason: String,
    },
    Program(String),
}

impl core::fmt::Display for EmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EmitError::RootLayout(m) => write!(f, "root layout unavailable: {m}"),
            EmitError::Scene(m) => write!(f, "stacking contexts failed: {m}"),
            EmitError::BadStyle { node, reason } => {
                write!(f, "node `{node}`: {reason}")
            }
            EmitError::Program(reason) => write!(f, "DrawProgram emission failed: {reason}"),
        }
    }
}

impl std::error::Error for EmitError {}

/// Host callback mapping shaped font bytes, face index, and size to semantic font identity using
/// the host registry.
pub type FontNaming<'a> = &'a dyn Fn(&[u8], u32, f32) -> FontFace;

/// Shared stable font naming from content hash and face index for native/web parity.
pub fn default_font_naming(bytes: &[u8], index: u32, size: f32) -> FontFace {
    let digest = crate::ContentDigest::of_bytes(bytes).as_hex();
    FontFace {
        family: format!("valle-face-{digest}-{index}"),
        weight: 400,
        italic: false,
        size: f64::from(size),
    }
}

/// Cross-frame font-name cache keyed by stable blob ID, face index, and size bits. Stable IDs avoid
/// pointer-reuse collisions; size remains part of the callback contract. Caching must preserve
/// identical FontFace results.
#[derive(Default)]
pub struct FaceCache {
    inner: std::cell::RefCell<std::collections::HashMap<(u64, u32, u32), FontFace>>,
}

impl FaceCache {
    /// Bound pathological cache growth; clearing entries only causes recomputation.
    const MAX_ENTRIES: usize = 256;

    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_insert(&self, key: (u64, u32, u32), build: impl FnOnce() -> FontFace) -> FontFace {
        if let Some(face) = self.inner.borrow().get(&key) {
            return face.clone();
        }
        let face = build();
        let mut inner = self.inner.borrow_mut();
        if inner.len() >= Self::MAX_ENTRIES {
            inner.clear();
        }
        inner.insert(key, face.clone());
        face
    }
}

/// FNV-1a 64 used only as a deterministic, non-security seed for procedural paper grain.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

fn grain_unit(state: &mut u64) -> f64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    ((*state >> 33) as f64) / f64::from(u32::MAX)
}

const PAPER_GRAIN_MAX_SPECKS: usize = 2800;
const PAPER_GRAIN_MAX_FIBERS: usize = 700;

/// Desired paper-grain instance counts, clipped to the remaining display-wide
/// GeometryBatch budget so grain cannot push `canonical_bytes()` over the limit.
fn paper_grain_counts(area: f64, amount: f64, remaining: usize) -> (usize, usize) {
    if remaining == 0 || !area.is_finite() || area <= 0.0 {
        return (0, 0);
    }
    let amount = amount.clamp(0.0, 1.0);
    let mut specks = ((area / 110.0) * amount).round() as usize;
    let mut fibers = ((area / 380.0) * amount).round() as usize;
    specks = specks.min(PAPER_GRAIN_MAX_SPECKS);
    fibers = fibers.min(PAPER_GRAIN_MAX_FIBERS);
    let desired = specks + fibers;
    if desired == 0 {
        return (0, 0);
    }
    if desired > remaining {
        specks = specks.saturating_mul(remaining) / desired;
        fibers = remaining - specks;
    }
    (specks, fibers)
}

/// Emit one frame using the host's semantic font naming callback.
pub fn emit(tree: &LayoutTree, naming: FontNaming<'_>) -> Result<EmitReport, EmitError> {
    emit_with_faces(tree, naming, None)
}

/// Emit with optional cross-frame font-name caching; cache hits preserve identical output.
pub fn emit_with_faces(
    tree: &LayoutTree,
    naming: FontNaming<'_>,
    faces: Option<&FaceCache>,
) -> Result<EmitReport, EmitError> {
    emit_with_faces_and_catalog(tree, naming, faces, None)
}

/// Product emission with immutable interpreted-content descriptors. This is required for
/// object-fit because layout intentionally does not decode media and DrawProgram stores the final
/// source/destination mapping rather than executor policy.
pub fn emit_program_with_faces(
    tree: &LayoutTree,
    naming: FontNaming<'_>,
    faces: Option<&FaceCache>,
    catalog: &dyn valle_draw::program::ProgramResourceCatalog,
) -> Result<EmitReport, EmitError> {
    emit_with_faces_and_catalog(tree, naming, faces, Some(catalog))
}

fn emit_with_faces_and_catalog(
    tree: &LayoutTree,
    naming: FontNaming<'_>,
    faces: Option<&FaceCache>,
    catalog: Option<&dyn valle_draw::program::ProgramResourceCatalog>,
) -> Result<EmitReport, EmitError> {
    let root_layout = tree
        .layout
        .layout(NodeId::ROOT)
        .map_err(|e| EmitError::RootLayout(e.to_string()))?;
    let mut out = EmissionState {
        recording: ProgramRecording::new(),
        scene3d_frames: Vec::new(),
        unsupported: Vec::new(),
    };
    let contexts = build_stacking_contexts(
        &tree.root,
        &tree.layout,
        NodeId::ROOT,
        TAffine::IDENTITY,
        (Some(root_layout.size.width), Some(root_layout.size.height)),
    )
    .map_err(|e| EmitError::Scene(e.to_string()))?;
    let reserved_batch_instances = tree
        .batches
        .values()
        .map(|batch| batch.instances.len())
        .fold(0usize, usize::saturating_add);
    let mut e = Emitter {
        tree,
        contexts: &contexts,
        naming,
        faces,
        out: &mut out,
        fonts: BTreeMap::new(),
        cur: TAffine::IDENTITY,
        reserved_batch_instances,
        grain_instances: 0,
        painted_formulas: HashSet::new(),
        resources: catalog,
        material_stack: Vec::new(),
    };
    e.context(0)?;
    e.account_invisible_formulas();
    e.report_unplaced_formulas();
    // Backfill filter content bounds after the complete recording exposes all subtree geometry.
    out.recording.backfill_filter_bounds();
    let viewport = Rect::new(
        0.0,
        0.0,
        f64::from(root_layout.size.width),
        f64::from(root_layout.size.height),
    );
    let program = match catalog {
        Some(textures) => {
            let catalog = EmissionResourceCatalog::new(textures, &out.scene3d_frames)
                .map_err(|error| EmitError::Program(error.to_string()))?;
            valle_draw::program::compile_recording_with_catalog(viewport, &out.recording, &catalog)
        }
        None => valle_draw::program::compile_recording(viewport, &out.recording),
    }
    .map_err(|error| EmitError::Program(error.to_string()))?;
    Ok(EmitReport {
        program,
        scene3d_frames: out.scene3d_frames,
        unsupported: out.unsupported,
    })
}

struct EmissionResourceCatalog<'a> {
    textures: &'a dyn valle_draw::program::ProgramResourceCatalog,
    scenes: BTreeMap<String, valle_draw::requirements::Scene3dKey>,
}

impl<'a> EmissionResourceCatalog<'a> {
    fn new(
        textures: &'a dyn valle_draw::program::ProgramResourceCatalog,
        scenes: &[crate::Scene3DFrameRequest],
    ) -> Result<Self, crate::CanonicalError> {
        let mut scene_keys = BTreeMap::new();
        for scene in scenes {
            let content_digest = scene.content_digest()?;
            let uri = format!("scene3d-frame://{}", content_digest.as_hex());
            scene_keys.insert(
                uri,
                valle_draw::requirements::Scene3dKey {
                    content_hash: valle_draw::requirements::DigestBytes::from_bytes(
                        *content_digest.as_bytes(),
                    ),
                    topology_hash: valle_draw::requirements::DigestBytes::from_bytes(
                        *scene.topology_digest()?.as_bytes(),
                    ),
                },
            );
        }
        Ok(Self {
            textures,
            scenes: scene_keys,
        })
    }
}

impl valle_draw::program::ProgramResourceCatalog for EmissionResourceCatalog<'_> {
    fn texture_extent(&self, key: &str) -> Option<valle_draw::program::ProgramTextureExtent> {
        self.textures.texture_extent(key)
    }

    fn scene3d_key(&self, key: &str) -> Option<valle_draw::requirements::Scene3dKey> {
        self.scenes.get(key).cloned()
    }

    fn scene3d_resource_digest(&self, control: &str) -> Option<[u8; 32]> {
        self.textures.scene3d_resource_digest(control)
    }
}

struct Emitter<'a> {
    tree: &'a LayoutTree,
    contexts: &'a [StackingContextNode],
    naming: FontNaming<'a>,
    /// Optional cross-frame font naming cache.
    faces: Option<&'a FaceCache>,
    out: &'a mut EmissionState,
    /// Per-frame font deduplication keyed by stable blob ID, face index, and size. Recording table
    /// indices cannot be reused across frames.
    fonts: BTreeMap<(u64, u32, u32), valle_draw::program::recording::FontId>,
    /// Current absolute transform composed from emitted transform groups, used to derive relative
    /// transforms from scene positions.
    cur: TAffine,
    /// Authored GeometryBatch instances that will occupy the display budget,
    /// whether or not they have been interned yet. Paper grain shares what is
    /// left of [`valle_draw::program::recording::MAX_BATCH_INSTANCES_PER_RECORDING`].
    reserved_batch_instances: usize,
    grain_instances: usize,
    painted_formulas: HashSet<String>,
    resources: Option<&'a dyn valle_draw::program::ProgramResourceCatalog>,
    /// Material owners currently enclosing the recording cursor. This is semantic state, not a
    /// rendering shortcut: field members must never be emitted without their one shared owner,
    /// and nested fields may begin only inside an existing member foreground.
    material_stack: Vec<String>,
}

/// Open node groups with the end count and transform state to restore.
struct Open {
    depth: usize,
    prev: TAffine,
    material_stack_len: usize,
    /// A culled CSS 3D plane suppresses its complete stacking context. Treating it as merely
    /// "no perspective wrapper" would paint the untransformed source plane for scale(0) and
    /// backface-visibility:hidden.
    hidden: bool,
}

impl Emitter<'_> {
    /// Paint a stacking context's root and ordered child buckets using upstream scene ordering.
    fn context(&mut self, index: usize) -> Result<(), EmitError> {
        let Some(cx) = self.contexts.get(index) else {
            return Ok(());
        };
        let open = if let Some(root) = cx.root() {
            self.begin_node(root)?
        } else {
            self.nothing_open()
        };
        if open.hidden {
            self.end(open);
            return Ok(());
        }
        if let Some(root) = cx.root() {
            self.node_content(root);
        }
        let inherited_material = self.material_stack.last().cloned();
        let mut field_open: Option<(String, Open)> = None;
        let mut closed_fields = HashSet::<String>::new();
        for bucket in cx.in_paint_order() {
            for item in bucket {
                let desired_field = self.field_for_item(item);
                if field_open
                    .as_ref()
                    .is_some_and(|(field, _)| desired_field.as_deref() != Some(field.as_str()))
                {
                    let (field, open) = field_open.take().expect("checked above");
                    self.end(open);
                    closed_fields.insert(field);
                }
                if field_open.is_none()
                    && let Some(field) = desired_field.as_deref()
                    && inherited_material.as_deref() != Some(field)
                {
                    if closed_fields.contains(field) {
                        return Err(EmitError::Program(format!(
                            "GlassField '{field}' is split into discontiguous painter-order runs"
                        )));
                    }
                    field_open = Some((field.to_owned(), self.begin_field(field)?));
                }
                self.paint_item(item)?;
            }
        }
        if let Some((_field, open)) = field_open {
            self.end(open);
        }
        self.end(open);
        Ok(())
    }

    fn paint_item(&mut self, item: &takumi_core::scene::PaintItem) -> Result<(), EmitError> {
        match &item.kind {
            PaintItemKind::Node(np) => {
                let open = self.begin_node(np)?;
                if !open.hidden {
                    self.node_content(np);
                }
                self.end(open);
                Ok(())
            }
            PaintItemKind::Context(id) => self.context(*id),
        }
    }

    fn field_for_item(&self, item: &takumi_core::scene::PaintItem) -> Option<String> {
        let paint = match &item.kind {
            PaintItemKind::Node(paint) => Some(paint),
            PaintItemKind::Context(index) => self.contexts.get(*index)?.root(),
        }?;
        let key = self.node_key(paint)?;
        self.tree.glass.node_fields.get(key).cloned()
    }

    fn begin_field(&mut self, field_id: &str) -> Result<Open, EmitError> {
        let field = self
            .tree
            .glass
            .fields
            .iter()
            .find(|field| field.field_id == field_id)
            .ok_or_else(|| {
                EmitError::Program(format!("GlassField '{field_id}' has no layout sidecar"))
            })?;
        let target = affine_from_glass_matrix(field.owner_to_viewport).ok_or_else(|| {
            EmitError::Program(format!(
                "GlassField '{field_id}' has a non-affine or non-finite owner transform"
            ))
        })?;
        let program = self
            .tree
            .glass
            .material_programs
            .get(field_id)
            .ok_or_else(|| {
                EmitError::Program(format!(
                    "GlassField '{field_id}' has no temporally prepared material program"
                ))
            })?;
        if program.owner_kind != valle_draw::program::glass::GlassOwnerKind::Field
            || program.owner_id != field_id
        {
            return Err(EmitError::Program(format!(
                "GlassField '{field_id}' material program has the wrong owner identity"
            )));
        }
        let prev = self.cur;
        let material_stack_len = self.material_stack.len();
        let inverse = self.cur.invert().ok_or_else(|| {
            EmitError::Program(format!(
                "GlassField '{field_id}' has a non-invertible parent transform"
            ))
        })?;
        let relative = inverse * target;
        let mut depth = 0;
        if !relative.is_identity() {
            self.push(RecordCmd::BeginTransform {
                transform: affine_of(relative),
            });
            self.cur = target;
            depth += 1;
        }
        self.push(RecordCmd::BeginMotionGlass {
            program: Box::new(program.clone()),
        });
        self.material_stack.push(field_id.to_owned());
        depth += 1;
        Ok(Open {
            depth,
            prev,
            material_stack_len,
            hidden: false,
        })
    }

    fn nothing_open(&self) -> Open {
        Open {
            depth: 0,
            prev: self.cur,
            material_stack_len: self.material_stack.len(),
            hidden: false,
        }
    }

    /// Open a node's groups and paint its shadows, background, and borders; return state needed to
    /// close it.
    fn begin_node(&mut self, np: &NodePaint) -> Result<Open, EmitError> {
        let none = self.nothing_open();
        let Some(node) = self.tree.root.node_at_path(&np.path) else {
            return Ok(none);
        };
        let style = &node.context.style;
        let Ok(layout) = self.tree.layout.layout(np.node_id) else {
            return Ok(none);
        };
        if style.is_invisible() {
            return Ok(none);
        }

        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
            && self
                .tree
                .css_3d_planes
                .get(key)
                .is_some_and(|plane| plane.hidden)
        {
            return Ok(Open {
                hidden: true,
                ..none
            });
        }

        let mut depth = 0usize;
        let prev = self.cur;
        let material_stack_len = self.material_stack.len();

        // Scene transforms are absolute, while recording transforms concatenate. Emit
        // inverse(current) * absolute to avoid applying parent transforms twice.
        if np.transform != self.cur {
            let Some(inv) = self.cur.invert() else {
                // Report and skip a noninvertible parent transform instead of silently misplacing
                // content.
                self.unsupported(np, "degenerate parent transform (not invertible)");
                return Ok(none);
            };
            let rel = inv * np.transform;
            if !rel.is_identity() {
                self.push(RecordCmd::BeginTransform {
                    transform: affine_of(rel),
                });
                depth += 1;
                self.cur = np.transform;
            }
        }

        // Glass is a semantic material shell, not a painted CSS box. The material owner and its
        // foreground marker sit immediately inside the shell transform so every ordinary child
        // remains real DrawProgram content and no effect-bearing wrapper can enter between them.
        if let Some(key) = self.node_key(np).map(str::to_owned)
            && let Some(surface) = self
                .tree
                .glass
                .surfaces
                .iter()
                .find(|surface| surface.node_key == key)
                .cloned()
        {
            if let Some(field_id) = surface.field_id.as_deref() {
                if self.material_stack.last().map(String::as_str) != Some(field_id) {
                    return Err(EmitError::Program(format!(
                        "Glass member '{}' is outside its field owner '{field_id}'",
                        surface.surface_id
                    )));
                }
            } else {
                let program = self
                    .tree
                    .glass
                    .material_programs
                    .get(&surface.surface_id)
                    .ok_or_else(|| {
                        EmitError::Program(format!(
                            "Glass '{}' has no temporally prepared material program",
                            surface.surface_id
                        ))
                    })?;
                if program.owner_kind != valle_draw::program::glass::GlassOwnerKind::Independent
                    || program.owner_id != surface.surface_id
                {
                    return Err(EmitError::Program(format!(
                        "Glass '{}' material program has the wrong owner identity",
                        surface.surface_id
                    )));
                }
                self.push(RecordCmd::BeginMotionGlass {
                    program: Box::new(program.clone()),
                });
                self.material_stack.push(surface.surface_id.clone());
                depth += 1;
            }
            let foreground = self
                .tree
                .glass
                .foreground_programs
                .get(&key)
                .ok_or_else(|| {
                    EmitError::Program(format!(
                        "Glass '{}' has no prepared foreground marker",
                        surface.surface_id
                    ))
                })?;
            let expected_owner = surface.field_id.as_deref().unwrap_or(&surface.surface_id);
            if foreground.owner_id != expected_owner || foreground.surface_id != surface.surface_id
            {
                return Err(EmitError::Program(format!(
                    "Glass '{}' foreground marker has the wrong owner identity",
                    surface.surface_id
                )));
            }
            self.push(RecordCmd::BeginMotionGlassForeground {
                program: Box::new(foreground.clone()),
            });
            depth += 1;
        }

        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id)) {
            if let Some(clip) = self.tree.clips.get(key).cloned()
                && let Some(path) = self.path_ref(&clip.path)
            {
                self.push(RecordCmd::BeginClipPath {
                    path,
                    fill_rule: clip.fill_rule,
                });
                depth += 1;
            }
            if let Some(mask) = self.tree.masks.get(key).cloned() {
                let source = match mask.source {
                    crate::layout::bridge::ResolvedMaskSource::Paint(paint) => {
                        Some(MaskSource::Paint {
                            paint: self.resolved_paint(&paint),
                        })
                    }
                    crate::layout::bridge::ResolvedMaskSource::Image(asset) => {
                        Some(MaskSource::Image {
                            image: self.out.recording.intern_image(
                                valle_draw::program::recording::ImageSource {
                                    asset,
                                    width: mask.rect.width.round().max(0.0) as u32,
                                    height: mask.rect.height.round().max(0.0) as u32,
                                    source_time_s: None,
                                },
                            ),
                        })
                    }
                    crate::layout::bridge::ResolvedMaskSource::Subtree(_) => {
                        self.push(RecordCmd::BeginSaveLayer {
                            bounds: Some(mask.rect),
                            alpha: 1.0,
                        });
                        depth += 1;
                        None
                    }
                };
                if let Some(source) = source {
                    self.push(RecordCmd::BeginMask {
                        source,
                        mode: mask.mode,
                        rect: mask.rect,
                    });
                    depth += 1;
                }
            }
            if self.tree.masks.values().any(|mask| {
                matches!(&mask.source, crate::layout::bridge::ResolvedMaskSource::Subtree(source) if source == key)
            }) {
                self.push(RecordCmd::BeginBlend {
                    mode: valle_draw::program::recording::BlendMode::DestinationIn,
                });
                depth += 1;
            }
        }

        let border_box = Rect::new(
            0.0,
            0.0,
            layout.size.width.into(),
            layout.size.height.into(),
        );

        // Apply backdrop filters before this node paints. Record explicit sampling bounds so
        // backends do not infer different full-canvas regions.
        let backdrop_advanced = self
            .tree
            .keys
            .get(&u64::from(np.node_id))
            .and_then(|key| self.tree.backdrop_advanced_filters.get(key));
        if !style.backdrop_filter.is_empty() || backdrop_advanced.is_some_and(|f| !f.is_empty()) {
            let mut ops = self.filter_ops(np, &style.backdrop_filter, &node.context);
            if let Some(advanced) = backdrop_advanced {
                ops.extend_from_slice(advanced);
            }
            let filters = self.out.recording.intern_filters(&ops);
            self.push(RecordCmd::BeginBackdropFilter {
                filters,
                bounds: Some(border_box),
            });
            depth += 1;
        }

        // Apply opacity and blending to the composited group rather than each leaf.
        let alpha = f64::from(style.opacity.0);
        let blend = blend_of(style);
        if alpha < 1.0 {
            self.push(RecordCmd::BeginSaveLayer {
                bounds: None,
                alpha,
            });
            depth += 1;
        }
        if let Some(mode) = blend {
            self.push(RecordCmd::BeginBlend { mode });
            depth += 1;
        }

        // Apply standard CSS filters, then displacement, then velocity blur in a fixed semantic
        // order.
        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
            && let Some(filters) = self.tree.advanced_filters.get(key)
            && !filters.is_empty()
        {
            let filters = self.out.recording.intern_filters(filters);
            self.push(RecordCmd::BeginFilter {
                filters,
                bounds: None,
            });
            depth += 1;
        }

        // Keep CSS filters inside the opacity/blend group.
        if !style.filter.is_empty() {
            let filters = self.filters(np, &style.filter, &node.context);
            self.push(RecordCmd::BeginFilter {
                filters,
                // Defer filter bounds until the complete recording is available.
                bounds: None,
            });
            depth += 1;
        }

        // 2.5D extras live after 2D layout/opacity and before the painted shell:
        // contact shadow stays on the desk (no rotateY), then the homography wraps the card.
        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id)) {
            let fx = self.tree.layer_fx.get(key).copied().unwrap_or_default();
            if fx.contact_shadow > 0.0 {
                self.contact_shadow(border_box, fx.contact_shadow);
            }
            if let Some(plane) = self.tree.css_3d_planes.get(key)
                && !plane.hidden
                && plane.project
            {
                let inverse =
                    affine_of(np.transform)
                        .inverse()
                        .ok_or_else(|| EmitError::BadStyle {
                            node: key.clone(),
                            reason: "CSS 3D plane has a non-invertible 2D layout transform".into(),
                        })?;
                let local = plane.quad.map(|point| inverse.apply(point));
                let matrix =
                    Homography::map_rect(border_box, local).ok_or_else(|| EmitError::BadStyle {
                        node: key.clone(),
                        reason: "CSS 3D plane is degenerate or crosses the projection plane".into(),
                    })?;
                self.push(RecordCmd::BeginPerspective { matrix });
                depth += 1;
            } else if fx.rotate_x_deg.abs() > 1e-6 || fx.rotate_y_deg.abs() > 1e-6 {
                let perspective = fx
                    .perspective
                    .unwrap_or(PerspectiveLength::DEFAULT_PX)
                    .to_px(&node.context.sizing)
                    .map_err(|reason| EmitError::BadStyle {
                        node: key.clone(),
                        reason,
                    })?;
                let origin = node.context.style.transform_origin;
                let cx = f64::from(origin.0.x.resolve(&node.context, border_box.width as f32));
                let cy = f64::from(origin.0.y.resolve(&node.context, border_box.height as f32));
                if let Some(matrix) =
                    Homography::layer(cx, cy, fx.rotate_y_deg, fx.rotate_x_deg, perspective)
                {
                    self.push(RecordCmd::BeginPerspective { matrix });
                    depth += 1;
                }
            }
        }

        // ShaderLayer sits inside CSS filter/clip/mask/opacity/blend but outside the complete
        // local subtree. Its mandatory bounds are this node's border box in the current local
        // coordinate system; substituting the canvas would violate both isolation and budget.
        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
            && let Some(shader) = self.tree.shaders.get(key).cloned()
        {
            let program = self.out.recording.intern_shader_program(shader.program);
            let uniforms = self.out.recording.intern_shader_uniforms(&shader.uniforms);
            let inputs = shader
                .inputs
                .into_iter()
                .map(|(name, asset)| ShaderTextureInput {
                    name,
                    image: self.out.recording.intern_image(
                        valle_draw::program::recording::ImageSource {
                            asset,
                            width: border_box.width.ceil().max(0.0) as u32,
                            height: border_box.height.ceil().max(0.0) as u32,
                            source_time_s: None,
                        },
                    ),
                })
                .collect::<Vec<_>>();
            let inputs = self.out.recording.intern_shader_inputs(&inputs);
            self.push(RecordCmd::BeginShaderLayer {
                program,
                uniforms,
                inputs,
                bounds: border_box,
            });
            depth += 1;
        }

        let radii = radii_of(&node.context, layout.size.width, layout.size.height);

        let is_glass_shell = self
            .node_key(np)
            .is_some_and(|key| self.tree.glass.foreground_programs.contains_key(key));

        // Paint the shell in CSS order. Glass defines material shape; ordinary pixels come from its
        // foreground subtree.
        // Match upstream shell paint order.
        if !is_glass_shell {
            self.box_shadows(np, node, border_box, radii, false);
            self.background(np, style, node, &layout, border_box, radii);
            if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
                && let Some(fx) = self.tree.layer_fx.get(key).copied()
                && fx.paper_grain > 0.0
            {
                self.paper_grain(border_box, fx.paper_grain, key);
            }
            self.box_shadows(np, node, border_box, radii, true);
            self.border(np, style, node, border_box, radii, &layout);
            self.outline(np, node, &layout);
        }

        // Clip overflow around content using the padding box.
        // Subtract border widths from the border box and corner radii.
        // Use CSS padding-box clipping, including padding when borders are absent.
        if style.clips_overflow() {
            let (inner, clip_radii) = inner_clip(&layout, radii);
            if clip_radii.iter().any(radius_is_visible) {
                self.push(RecordCmd::BeginClipRoundRect {
                    rrect: RoundRect {
                        rect: inner,
                        radii: clip_radii,
                    },
                });
            } else {
                self.push(RecordCmd::BeginClipRect { rect: inner });
            }
            depth += 1;
        }

        if style.has_shape_mask() {
            self.unsupported(np, "CSS clip-path / mask-image properties");
        }

        Ok(Open {
            depth,
            prev,
            material_stack_len,
            hidden: false,
        })
    }

    fn end(&mut self, open: Open) {
        for _ in 0..open.depth {
            self.push(RecordCmd::End);
        }
        self.material_stack.truncate(open.material_stack_len);
        self.cur = open.prev;
    }

    /// Emit paths, images, and glyph runs. Report unsupported inline-block content; leave pixel
    /// resolution to native/web resource tables.
    fn node_content(&mut self, np: &NodePaint) {
        let Some(node) = self.tree.root.node_at_path(&np.path) else {
            return;
        };
        let Ok(layout) = self.tree.layout.layout(np.node_id) else {
            return;
        };
        if node.context.style.is_invisible() {
            return;
        }
        let formula_key = self
            .tree
            .keys
            .get(&u64::from(np.node_id))
            .or_else(|| self.tree.render_keys.get(&np.path));
        if let Some(key) = formula_key
            && let Some(fragment) = self.tree.formulas.get(key)
        {
            let dst = content_box(&layout);
            self.blit_formula(key, fragment, valle_draw::Point::new(dst.x, dst.y));
            return;
        }
        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
            && let Some(batch) = self.tree.batches.get(key).cloned()
        {
            if batch.instances.is_empty() {
                return;
            }
            let instances = self.out.recording.intern_batch_instances(&batch.instances);
            self.push(RecordCmd::GeometryBatch {
                geometry: batch.geometry,
                instances,
            });
            return;
        }
        // Paths share CSS groups and stacking order while retaining node-local coordinates.
        if let Some(key) = self.tree.keys.get(&u64::from(np.node_id))
            && let Some(path) = self.tree.paths.get(key).cloned()
        {
            if path.verbs.is_empty() {
                return;
            }
            let geometry = valle_motion::PathData {
                verbs: path.verbs.clone(),
                points: path.points.clone(),
            };
            let Some(path_ref) = self.path_ref(&geometry) else {
                return;
            };
            let fill = path.fill.as_ref().map(|paint| self.resolved_paint(paint));
            let stroke = path
                .stroke
                .as_ref()
                .map(|stroke| self.resolved_stroke(stroke));
            self.push(RecordCmd::Path {
                path: path_ref,
                fill_rule: FillRule::NonZero,
                fill,
                stroke: stroke.clone(),
            });
            if let (Some(stroke), Some(arrow)) = (&stroke, &path.arrow_start) {
                self.path_arrow(&geometry, arrow, false, stroke);
            }
            if let (Some(stroke), Some(arrow)) = (&stroke, &path.arrow_end) {
                self.path_arrow(&geometry, arrow, true, stroke);
            }
            return;
        }
        // Emit image references into the content box, leaving padding as background and pixel
        // resolution to the backend.
        //
        // Image source dimensions describe the destination size; layout remains independent of
        // decoders.
        if let Some(inner) = &node.node
            && let takumi_core::layout::node::NodeKind::Image(img) = &inner.kind
        {
            // Use local coordinates because the node transform is already active.
            let dst = content_box(&layout);
            // Record object-fit for executor-side application with decoded intrinsic dimensions.
            let fit = match node.context.style.object_fit {
                takumi_core::style::ObjectFit::Fill => {
                    valle_draw::program::recording::ObjectFit::Fill
                }
                takumi_core::style::ObjectFit::Contain => {
                    valle_draw::program::recording::ObjectFit::Contain
                }
                takumi_core::style::ObjectFit::Cover => {
                    valle_draw::program::recording::ObjectFit::Cover
                }
                takumi_core::style::ObjectFit::ScaleDown => {
                    valle_draw::program::recording::ObjectFit::ScaleDown
                }
                takumi_core::style::ObjectFit::None => {
                    valle_draw::program::recording::ObjectFit::None
                }
            };
            let image_asset = image_asset_of(img);
            let scene3d = image_asset
                .strip_prefix("generated://scene3d/")
                .and_then(|key| self.tree.scene3d.get(key))
                .cloned();
            if let Some(key) = image_asset.strip_prefix("generated://formula/") {
                let Some(fragment) = self.tree.formulas.get(key) else {
                    self.unsupported(np, "formula fragment missing");
                    return;
                };
                let dst = content_box(&layout);
                self.blit_formula(key, fragment, valle_draw::Point::new(dst.x, dst.y));
                return;
            }
            if scene3d.is_some() && !fit.is_fill() {
                self.unsupported(
                    np,
                    "Scene3D generated textures require object-fit: fill; raster dimensions already match the content box",
                );
                return;
            }
            // Video uses the image path with a sample time already resolved during tree
            // construction.
            let video = self
                .tree
                .keys
                .get(&u64::from(np.node_id))
                .and_then(|key| self.tree.videos.get(key))
                .cloned();
            if dst.width > 0.0 && dst.height > 0.0 {
                // Do not multiply image alpha again; node opacity is already applied to the entire
                // group.
                //
                // Clip replaced image content to border radii independently of overflow settings.
                let radii = radii_of(&node.context, layout.size.width, layout.size.height);
                let (clip_rect, clip_radii) = inner_clip(&layout, radii);
                let rounded = clip_radii.iter().any(radius_is_visible);
                // Cover and None can exceed the destination and must clip to the content box. Other
                // fit modes stay inside it.
                let overflowable = scene3d.is_none()
                    && matches!(
                        fit,
                        valle_draw::program::recording::ObjectFit::Cover
                            | valle_draw::program::recording::ObjectFit::None
                    );
                if rounded {
                    self.push(RecordCmd::BeginClipRoundRect {
                        rrect: RoundRect {
                            rect: clip_rect,
                            radii: clip_radii,
                        },
                    });
                } else if overflowable {
                    self.push(RecordCmd::BeginClipRect { rect: dst });
                }
                if let Some(request) = scene3d {
                    let mut resource_bindings = BTreeMap::new();
                    for mesh in &request.scene.meshes {
                        let controls = core::iter::once(&mesh.model_control)
                            .chain(mesh.material.texture_control.iter());
                        for control in controls {
                            if let Some(digest_bytes) = self
                                .resources
                                .and_then(|resources| resources.scene3d_resource_digest(control))
                            {
                                resource_bindings.insert(
                                    control.clone(),
                                    crate::ContentDigest::from_bytes(digest_bytes),
                                );
                            }
                        }
                    }
                    let frame = crate::Scene3DFrameRequest::new(
                        &request,
                        dst.width.ceil() as u32,
                        dst.height.ceil() as u32,
                        resource_bindings,
                    )
                    .expect("validated Scene3D layout state is canonically serializable");
                    let content_digest = frame
                        .content_digest()
                        .expect("validated Scene3D frame has a stable digest");
                    let provider_key = format!("scene3d-frame://{}", content_digest.as_hex());
                    self.out.scene3d_frames.push(frame);
                    let texture = self.out.recording.intern_generated_texture(
                        valle_draw::program::recording::GeneratedTextureSource {
                            provider_key,
                            width: dst.width.ceil() as u32,
                            height: dst.height.ceil() as u32,
                        },
                    );
                    self.push(RecordCmd::Texture {
                        texture,
                        dst,
                        alpha: 1.0,
                    });
                } else {
                    let image = self.out.recording.intern_image(
                        valle_draw::program::recording::ImageSource {
                            asset: video
                                .as_ref()
                                .map(|v| v.asset.clone())
                                .unwrap_or(image_asset),
                            width: dst.width.round().max(0.0) as u32,
                            height: dst.height.round().max(0.0) as u32,
                            source_time_s: video.as_ref().map(|v| v.source_time_s),
                        },
                    );
                    self.push(RecordCmd::Image {
                        image,
                        dst,
                        src: None,
                        alpha: 1.0,
                        fit,
                    });
                }
                if rounded || overflowable {
                    self.push(RecordCmd::End);
                }
            }
            return;
        }
        // Handle nodes whose own content is text.
        //
        // Child-text predicates do not cover root Text nodes; include the node's own inline text
        // when deciding whether to emit glyphs.
        //
        // Root text must enter emission even without child nodes.
        if has_own_text(node)
            || node.should_create_inline_layout()
            || node.has_anonymous_text_item_child()
        {
            self.text(np, node, layout);
        }
    }

    fn resolved_paint(&mut self, paint: &ResolvedPaint) -> Paint {
        match paint {
            ResolvedPaint::Solid(color) => Paint::Solid(*color),
            ResolvedPaint::Linear {
                start,
                end,
                stops,
                spread,
            } => {
                let stops = self.out.recording.intern_stops(stops);
                Paint::Linear(valle_draw::program::recording::LinearGradient {
                    start: *start,
                    end: *end,
                    stops,
                    spread: *spread,
                    alpha: 1.0,
                })
            }
            ResolvedPaint::Radial {
                center,
                radius,
                stops,
                spread,
            } => {
                let stops = self.out.recording.intern_stops(stops);
                Paint::Radial(valle_draw::program::recording::RadialGradient {
                    center: *center,
                    radii: Point::new(*radius, *radius),
                    stops,
                    spread: *spread,
                    alpha: 1.0,
                })
            }
            ResolvedPaint::Conic {
                center,
                start_angle,
                stops,
                spread,
            } => {
                let stops = self.out.recording.intern_stops(stops);
                Paint::Conic(valle_draw::program::recording::ConicGradient {
                    center: *center,
                    start_angle: *start_angle,
                    sweep_angle: 360.0,
                    stops,
                    spread: *spread,
                    alpha: 1.0,
                })
            }
        }
    }

    fn path_ref(&mut self, path: &valle_motion::PathData) -> Option<valle_draw::PathRef> {
        if path.verbs.is_empty() {
            return None;
        }
        let mut builder = self.out.recording.begin_path();
        let mut point = 0usize;
        for verb in &path.verbs {
            builder = match verb {
                valle_draw::PathVerb::Move => {
                    let p = path.points[point];
                    point += 1;
                    builder.move_to(p)
                }
                valle_draw::PathVerb::Line => {
                    let p = path.points[point];
                    point += 1;
                    builder.line_to(p)
                }
                valle_draw::PathVerb::Quad => {
                    let c = path.points[point];
                    let p = path.points[point + 1];
                    point += 2;
                    builder.quad_to(c, p)
                }
                valle_draw::PathVerb::Cubic => {
                    let c1 = path.points[point];
                    let c2 = path.points[point + 1];
                    let p = path.points[point + 2];
                    point += 3;
                    builder.cubic_to(c1, c2, p)
                }
                valle_draw::PathVerb::Close => builder.close(),
            };
        }
        Some(builder.finish())
    }

    fn resolved_stroke(&mut self, stroke: &ResolvedStroke) -> Stroke {
        Stroke {
            paint: self.resolved_paint(&stroke.paint),
            width: stroke.width,
            dash: stroke.dash.clone(),
            dash_offset: stroke.dash_offset,
            cap: stroke.cap,
            join: stroke.join,
            miter_limit: stroke.miter_limit,
        }
    }

    fn path_arrow(
        &mut self,
        path: &valle_motion::PathData,
        arrow: &valle_motion::ArrowSpec,
        at_end: bool,
        stroke: &Stroke,
    ) {
        let progress = if at_end { 1.0 } else { 0.0 };
        let (Ok(tip), Ok(mut tangent)) = (path.point_at(progress), path.tangent_at(progress))
        else {
            return;
        };
        if !at_end {
            tangent.x = -tangent.x;
            tangent.y = -tangent.y;
        }
        let normal = valle_draw::Vec2::new(-tangent.y, tangent.x);
        let base = valle_draw::Point::new(
            tip.x - tangent.x * arrow.size,
            tip.y - tangent.y * arrow.size,
        );
        let half_width = arrow.size * 0.45;
        let left = valle_draw::Point::new(
            base.x + normal.x * half_width,
            base.y + normal.y * half_width,
        );
        let right = valle_draw::Point::new(
            base.x - normal.x * half_width,
            base.y - normal.y * half_width,
        );
        let mut builder = self.out.recording.begin_path();
        let (fill, arrow_stroke) = match arrow.kind {
            valle_motion::ArrowKind::Triangle => {
                builder = builder.move_to(tip).line_to(left).line_to(right).close();
                (Some(stroke.paint), None)
            }
            valle_motion::ArrowKind::Open => {
                builder = builder.move_to(left).line_to(tip).line_to(right);
                let mut arrow_stroke = stroke.clone();
                arrow_stroke.dash = None;
                (None, Some(arrow_stroke))
            }
        };
        let path = builder.finish();
        self.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::NonZero,
            fill,
            stroke: arrow_stroke,
        });
    }

    /// Resolve upstream shaped runs and emit glyph IDs, positions, sizes, and font identities.
    /// Backends retain glyph rasterization, atlas, and hinting ownership.
    fn text(
        &mut self,
        np: &NodePaint,
        node: &RenderNode,
        layout: takumi_core::geometry::ComputedLayout,
    ) {
        let ctx = &node.context;
        let font_style = takumi_core::font_style::SizedFontStyle::from_style(&ctx.style, ctx);
        let items = collect_inline_items(node);
        // Capture authored contexts before layout inserts synthetic direction spans.
        let item_contexts: Vec<Option<&takumi_core::context::RenderContext>> = items
            .iter()
            .map(|item| match item {
                takumi_core::layout::inline::InlineItem::Text { context, .. } => Some(*context),
                _ => None,
            })
            .collect();
        let built = create_inline_layout(InlineLayoutRequest::in_content_box(
            items,
            takumi_core::geometry::Size {
                width: layout.content_box_width(),
                height: layout.content_box_height(),
            },
            &font_style,
            ctx,
            InlineLayoutMode::Draw,
        ));
        let Ok(resolved) = resolve_inline_runs(&built, ctx, layout) else {
            self.unsupported(np, "text shaping failed");
            return;
        };
        let anchors = self.span_anchors(np, &built, &item_contexts);
        // Report text that produces no glyphs, including missing-font cases that would otherwise
        // look like successful empty output.
        let empty = resolved.runs.iter().all(|r| r.glyph_run.glyphs.is_empty());
        if empty {
            // Use the same text predicate for emission and missing-glyph reporting.
            if has_own_text(node)
                || node.has_anonymous_text_item_child()
                || node.should_create_inline_layout()
            {
                self.unsupported(np, "text produced no glyphs (no usable font?)");
            }
            return;
        }
        // Compute source ranges once per glyph run and share them between text and shadow layers.
        let source_ranges: Vec<Option<GlyphSource>> = resolved
            .runs
            .iter()
            .map(|run| self.glyph_source(np, &run.glyph_run, &anchors, &built.text))
            .collect();
        // Compute text-path arc length once and reuse placements for text and shadows.
        //
        // Resolve text nodes through text_sources; inline text may share a parent paint node.
        //
        // Resolve identity per run because one inline layout may span multiple Text nodes.
        let run_keys: Vec<Option<String>> = source_ranges
            .iter()
            .map(|source| {
                let source = (*source)?;
                self.out
                    .recording
                    .text_sources
                    .get(source.node.index())
                    .cloned()
            })
            .collect();
        // Cache each node's arc length and initial advance so its text starts at its own path
        // origin.
        let mut plans: std::collections::HashMap<String, (TextPathPlan, f64)> =
            std::collections::HashMap::new();
        let mut degenerate = false;
        for (run, key) in resolved.runs.iter().zip(&run_keys) {
            let Some(key) = key else { continue };
            let Some(path) = self.tree.text_paths.get(key).cloned() else {
                continue;
            };
            let first_x = run
                .glyph_run
                .glyphs
                .first()
                .map_or(0.0, |glyph| f64::from(glyph.x));
            match plans.get_mut(key) {
                Some((_, origin)) => *origin = origin.min(first_x),
                None => match path.path_length() {
                    Ok(length) if length > 0.0 => {
                        plans.insert(key.clone(), (TextPathPlan { path, length }, first_x));
                    }
                    _ => degenerate = true,
                },
            }
        }
        if degenerate {
            self.unsupported(np, "text path has zero length or cannot be flattened");
        }
        let placements: Vec<Option<Vec<Option<Affine>>>> = resolved
            .runs
            .iter()
            .zip(&run_keys)
            .map(|(run, key)| {
                let (plan, origin) = plans.get(key.as_ref()?)?;
                path_placements(plan, *origin, run, layout)
            })
            .collect();
        if !plans.is_empty() {
            // Reject wrapped text on a single path; later lines would otherwise overlap at the path
            // origin.
            let mut baselines: Vec<u32> = resolved
                .runs
                .iter()
                .map(|run| run.glyph_run.baseline.to_bits())
                .collect();
            baselines.sort_unstable();
            baselines.dedup();
            if baselines.len() > 1 {
                self.unsupported(np, "text path requires one line, but the text wrapped");
            }
            if resolved.runs.iter().any(|run| run.baseline_shift != 0.0) {
                self.unsupported(
                    np,
                    "text path does not preserve superscript or subscript baseline offsets",
                );
            }
            // Report missing placements only for runs that actually requested a path.
            if resolved
                .runs
                .iter()
                .zip(&run_keys)
                .zip(&placements)
                .any(|((_, key), placement)| {
                    key.as_ref().is_some_and(|key| plans.contains_key(key)) && placement.is_none()
                })
            {
                self.unsupported(np, "text path sampling failed");
            }
            if placements.iter().flatten().flatten().any(Option::is_none) {
                self.unsupported(np, "text path omitted glyphs beyond the path length");
            }
        }
        let placement_of =
            |at: usize| -> Option<&[Option<Affine>]> { placements.get(at)?.as_deref() };
        // Paint text shadows in reverse declaration order below the main text. Bake offsets into
        // glyph positions and use existing blur groups with shadow sigma equal to half the radius.
        if let Some(shadows) = &ctx.style.text_shadow {
            for sh in shadows.iter().rev() {
                // Text-shadow lengths do not accept percentage bases.
                let px = |l: takumi_core::style::Length| f64::from(l.to_px(&ctx.sizing, 0.0));
                let (dx, dy, blur) = (
                    px(sh.offset_x),
                    px(sh.offset_y),
                    px(sh.blur_radius).max(0.0),
                );
                let color = rgba_of(sh.color.resolve(ctx.current_color));
                let filtered = blur > 0.0;
                if filtered {
                    // Shadow Gaussian sigma is half the CSS blur radius.
                    let ops = self.out.recording.intern_filters(&[
                        valle_draw::program::recording::FilterOp::Blur { sigma: blur / 2.0 },
                    ]);
                    self.push(RecordCmd::BeginFilter {
                        filters: ops,
                        bounds: None,
                    });
                }
                for (at, (run, sr)) in resolved.runs.iter().zip(&source_ranges).enumerate() {
                    self.glyph_run_at(
                        np,
                        run,
                        layout,
                        Some((dx, dy, color)),
                        *sr,
                        placement_of(at),
                    );
                }
                if filtered {
                    self.push(RecordCmd::End);
                }
            }
        }
        for (at, (run, sr)) in resolved.runs.iter().zip(&source_ranges).enumerate() {
            self.glyph_run_at(np, run, layout, None, *sr, placement_of(at));
        }
        for inline_box in &resolved.inline_boxes {
            let Some(ProcessedInlineSpan::Box(item)) = built.spans.get(inline_box.id as usize)
            else {
                self.unsupported(np, "inline box cannot be matched to its layout span");
                continue;
            };
            self.inline_image(np, item, inline_box, layout);
        }
        if !resolved.outline_rects.is_empty() {
            // Inline CSS outline islands are separate from supported glyph strokes.
            self.unsupported(np, "inline CSS outline islands are not emitted");
        }
    }

    /// Inline image placement and baseline come from the inline formatter. Emit the shared backend-
    /// neutral Image command; report unsupported inline containers explicitly.
    fn inline_image(
        &mut self,
        np: &NodePaint,
        item: &InlineBoxItem<'_>,
        visual: &VisualInlineBox,
        parent_layout: takumi_core::geometry::ComputedLayout,
    ) {
        let node = item.render_node;
        let Some(inner) = &node.node else {
            self.unsupported(np, "inline box has no drawable node");
            return;
        };
        let takumi_core::layout::node::NodeKind::Image(image) = &inner.kind else {
            self.unsupported(np, "inline containers and non-image boxes are unsupported");
            return;
        };

        let offset = parent_layout.content_box_offset();
        let margin = item.margin;
        let width = (visual.width - margin.left - margin.right).max(0.0);
        let height = (visual.height - margin.top - margin.bottom).max(0.0);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let dst = Rect::new(
            f64::from(offset.x + visual.x + margin.left),
            f64::from(offset.y + visual.y + margin.top),
            f64::from(width),
            f64::from(height),
        );
        let local = node
            .context
            .style
            .local_transform(width, height, &node.context.sizing);
        if !local.is_identity() {
            self.unsupported(
                np,
                "inline image transforms are unsupported by shared placement",
            );
        }
        let fit = match node.context.style.object_fit {
            takumi_core::style::ObjectFit::Fill => valle_draw::program::recording::ObjectFit::Fill,
            takumi_core::style::ObjectFit::Contain => {
                valle_draw::program::recording::ObjectFit::Contain
            }
            takumi_core::style::ObjectFit::Cover => {
                valle_draw::program::recording::ObjectFit::Cover
            }
            takumi_core::style::ObjectFit::ScaleDown => {
                valle_draw::program::recording::ObjectFit::ScaleDown
            }
            takumi_core::style::ObjectFit::None => valle_draw::program::recording::ObjectFit::None,
        };
        let radii = radii_of(&node.context, width, height);
        let rounded = radii.iter().any(radius_is_visible);
        let overflowable = matches!(
            fit,
            valle_draw::program::recording::ObjectFit::Cover
                | valle_draw::program::recording::ObjectFit::None
        );
        if rounded {
            self.push(RecordCmd::BeginClipRoundRect {
                rrect: RoundRect { rect: dst, radii },
            });
        } else if overflowable {
            self.push(RecordCmd::BeginClipRect { rect: dst });
        }
        let alpha = f64::from(node.context.style.opacity.0).clamp(0.0, 1.0);
        if alpha < 1.0 {
            self.push(RecordCmd::BeginOpacity { alpha });
        }
        let image = self
            .out
            .recording
            .intern_image(valle_draw::program::recording::ImageSource {
                asset: image_asset_of(image),
                width: width.round().max(0.0) as u32,
                height: height.round().max(0.0) as u32,
                source_time_s: None,
            });
        self.push(RecordCmd::Image {
            image,
            dst,
            src: None,
            alpha: 1.0,
            fit,
        });
        if alpha < 1.0 {
            self.push(RecordCmd::End);
        }
        if rounded || overflowable {
            self.push(RecordCmd::End);
        }
    }

    /// Map inline spans back to Scene nodes and node-relative byte offsets so changes to siblings
    /// do not shift source addresses.
    fn span_anchors(
        &self,
        np: &NodePaint,
        built: &takumi_core::layout::inline::BuiltInlineLayout<'_>,
        item_contexts: &[Option<&takumi_core::context::RenderContext>],
    ) -> Vec<Result<SpanAnchor, ClusterReject>> {
        let Some(node) = self.tree.root.node_at_path(&np.path) else {
            return built
                .spans
                .iter()
                .map(|_| Err(ClusterReject::Unattributed))
                .collect();
        };
        let mut paths = Vec::new();
        context_paths(node, &np.path, &mut paths);
        let mut contexts = item_contexts.iter();
        built
            .spans
            .iter()
            .map(|span| {
                // Synthetic bidi markers have no authored item or source context.
                if matches!(span, ProcessedInlineSpan::DirectionMark { .. }) {
                    return Err(ClusterReject::Unattributed);
                }
                let context = contexts.next().copied().flatten();
                let ProcessedInlineSpan::Text {
                    byte_range, text, ..
                } = span
                else {
                    return Err(ClusterReject::Unattributed);
                };
                let context = context.ok_or(ClusterReject::Unattributed)?;
                // Context pointer identity is valid within this borrowed, unmoved render subtree.
                let key = paths
                    .iter()
                    .find(|(ctx, _)| std::ptr::eq(*ctx, context))
                    .and_then(|(_, path)| self.tree.render_keys.get(path))
                    .ok_or(ClusterReject::Unattributed)?;
                let source = self
                    .tree
                    .node_texts
                    .get(key)
                    .ok_or(ClusterReject::NotVerbatim)?;
                let projection = text_source_projection(
                    source,
                    text,
                    context.style.white_space_collapse,
                    context.style.text_transform,
                    tab_spaces(&context.style.tab_size),
                )?;
                Ok(SpanAnchor {
                    key: key.clone(),
                    range: byte_range.clone(),
                    projection,
                })
            })
            .collect()
    }

    /// Validate each glyph cluster against both the shaping input range and its owning span before
    /// exposing source addresses. Upstream glyph-ID matching may select a repeated sequence from
    /// the wrong span; report unavailable addressing instead of returning incorrect offsets.
    fn glyph_source(
        &mut self,
        np: &NodePaint,
        shaped: &ShapedRun,
        anchors: &[Result<SpanAnchor, ClusterReject>],
        inline_text: &str,
    ) -> Option<GlyphSource> {
        // Empty runs emit no command or side-table entry; missing glyphs have a separate
        // diagnostic.
        if shaped.glyphs.is_empty() {
            return None;
        }
        // Ellipsis is an intentionally synthetic run created by Takumi and has no authored span.
        // It remains drawable but must not pretend to point at source text or create an
        // unsupported bill entry.
        let Some(span_id) = shaped.brush.source_span_id else {
            return None;
        };
        let anchor = anchors
            .get(span_id as usize)
            .ok_or(ClusterReject::Unattributed)
            .and_then(|anchor| anchor.as_ref().map_err(|reject| *reject));
        let source = anchor.and_then(|anchor| {
            let mut ranges = checked_cluster_ranges(
                &shaped.cluster_ranges,
                shaped.glyphs.len(),
                &shaped.text_range,
                &anchor.range,
            )?;
            if let Some(projection) = &anchor.projection {
                ranges = project_cluster_ranges(&ranges, projection)?;
            }
            Ok((anchor.key.to_owned(), ranges))
        });
        match source {
            Ok((key, ranges)) => {
                let node = self.out.recording.intern_text_source(&key);
                let ranges = self.out.recording.intern_glyph_source_ranges(&ranges);
                Some(GlyphSource { node, ranges })
            }
            Err(reject) => {
                // Takumi matches repeated glyph IDs against the whole shaping run. An empty
                // glyph can therefore point at a synthetic bidi marker instead of its space.
                // Such a glyph has no pixels or authored source; never relax checks for ink.
                if reject == ClusterReject::OutOfRun
                    && !shaped.cluster_ranges.is_empty()
                    && shaped.cluster_ranges.iter().all(|range| {
                        inline_text.get(range.clone()).is_some_and(|text| {
                            !text.is_empty()
                                && text.chars().all(|c| {
                                    matches!(c,
                                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                                })
                        })
                    })
                    && ttf_parser::Face::parse(shaped.font_data(), shaped.font_index).is_ok_and(
                        |face| {
                            shaped.glyphs.iter().all(|glyph| {
                                let id = ttf_parser::GlyphId(glyph.id as u16);
                                !face.is_color_glyph(id)
                                    && face.glyph_bounding_box(id).is_none()
                                    && face.glyph_raster_image(id, u16::MAX).is_none()
                                    && face.glyph_svg_image(id).is_none()
                            })
                        },
                    )
                {
                    return None;
                }
                self.unsupported(np, reject.reason());
                None
            }
        }
    }

    /// Emit shadow layers with offset glyph positions and shadow paint, sharing the run's
    /// source-range span with the main text.
    fn color_outline_run_at(
        &mut self,
        run: &PositionedInlineRun,
        layout: takumi_core::geometry::ComputedLayout,
        shadow: Option<(f64, f64, Rgba)>,
        placements: Option<&[Option<Affine>]>,
    ) -> bool {
        // Color glyph palettes belong to the foreground fill only. CSS text-shadow deliberately
        // remains a monochrome silhouette, and along-path placement still uses GlyphRun source
        // addressing until colored paths gain the same per-glyph contract.
        if shadow.is_some() || placements.is_some() {
            return false;
        }
        let shaped = &run.glyph_run;
        let has_color = shaped.glyphs.iter().any(|glyph| {
            run.resolved_glyphs
                .get(&glyph.id)
                .is_some_and(|resolved| match resolved.as_ref() {
                    ResolvedGlyph::Outline(outline) => outline.color_layers().is_some(),
                    ResolvedGlyph::Bitmap(_) => false,
                })
        });
        if !has_color {
            return false;
        }
        // Font fallback splits ordinary text and emoji into separate shaped runs. Requiring every
        // glyph here to be a COLR outline prevents a mixed or partially resolved run from losing
        // its non-color glyphs.
        if !shaped.glyphs.iter().all(|glyph| {
            run.resolved_glyphs
                .get(&glyph.id)
                .is_some_and(|resolved| match resolved.as_ref() {
                    ResolvedGlyph::Outline(outline) => outline.color_layers().is_some(),
                    ResolvedGlyph::Bitmap(_) => false,
                })
        }) {
            return false;
        }

        let text_fit_transform = affine_of(run.transform(TAffine::IDENTITY));
        if text_fit_transform != Affine::IDENTITY {
            self.push(RecordCmd::BeginTransform {
                transform: text_fit_transform,
            });
        }
        let run_opacity = f64::from(shaped.brush.opacity).clamp(0.0, 1.0);
        if run_opacity < 1.0 {
            self.push(RecordCmd::BeginOpacity { alpha: run_opacity });
        }

        let offset = run.glyph_offset(layout);
        for glyph in &shaped.glyphs {
            let Some(ResolvedGlyph::Outline(outline)) = run
                .resolved_glyphs
                .get(&glyph.id)
                .map(|resolved| resolved.as_ref())
            else {
                continue;
            };
            let layers = run.resolve_color_layers(outline, shaped.brush.color);
            if layers.is_empty() {
                continue;
            }
            self.push(RecordCmd::BeginTransform {
                transform: Affine::translate(
                    f64::from(offset.x + glyph.x),
                    f64::from(offset.y + glyph.y),
                ),
            });
            for (color, commands) in layers {
                let Some(path) = self.path_of(commands) else {
                    continue;
                };
                self.push(RecordCmd::Path {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(Paint::Solid(rgba_of(color))),
                    stroke: None,
                });
            }
            self.push(RecordCmd::End);
        }

        if run_opacity < 1.0 {
            self.push(RecordCmd::End);
        }
        if text_fit_transform != Affine::IDENTITY {
            self.push(RecordCmd::End);
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph_run_at(
        &mut self,
        np: &NodePaint,
        run: &PositionedInlineRun,
        layout: takumi_core::geometry::ComputedLayout,
        shadow: Option<(f64, f64, Rgba)>,
        source_ranges: Option<GlyphSource>,
        placements: Option<&[Option<Affine>]>,
    ) {
        match crate::colr::emit_run(&mut self.out.recording, run, layout, shadow, placements) {
            Ok(true) => return,
            Err(error) => {
                self.unsupported(np, error);
                return;
            }
            Ok(false) => {}
        }
        let shaped = &run.glyph_run;
        let size = shaped.font_size;
        let key = (shaped.font_id(), shaped.font_index, size.to_bits());
        // Call font naming only on cache misses to avoid repeated full-font hashing.
        let font = if let Some(id) = self.fonts.get(&key) {
            *id
        } else {
            let bytes = shaped.font_data();
            let index = shaped.font_index;
            let build = || (self.naming)(bytes, index, size);
            let face = match self.faces {
                Some(cache) => cache.get_or_insert(key, build),
                None => build(),
            };
            let id = self.out.recording.intern_font(face);
            self.fonts.insert(key, id);
            id
        };

        // Glyph offsets already include the content-box origin and baseline shift.
        let g_off = run.glyph_offset(layout);
        let glyphs: Vec<Glyph> = shaped
            .glyphs
            .iter()
            .map(|g| Glyph {
                id: g.id,
                x: f64::from(g_off.x + g.x) + shadow.map_or(0.0, |(dx, _, _)| dx),
                y: f64::from(g_off.y + g.y) + shadow.map_or(0.0, |(_, dy, _)| dy),
            })
            .collect();
        if glyphs.is_empty() {
            return;
        }
        if self.color_outline_run_at(run, layout, shadow, placements) {
            return;
        }
        // Embedded bitmap glyphs still need an image side-table contract. COLR outline glyphs use
        // ordinary backend-neutral Path commands above, so native and CanvasKit consume identical
        // palette geometry without teaching GlyphRun a second paint model.
        if run
            .resolved_glyphs
            .values()
            .any(|glyph| match glyph.as_ref() {
                ResolvedGlyph::Bitmap(_) => true,
                ResolvedGlyph::Outline(_) => false,
            })
        {
            self.unsupported(np, "bitmap emoji glyphs");
        }
        let span = self.out.recording.intern_glyphs(&glyphs);
        // Read glyph stroke width and color from the node style.
        let ctx_style = &self.tree.root.node_at_path(&np.path).map(|n| &n.context);
        let declared_stroke = ctx_style.as_ref().and_then(|c| {
            let w = f64::from(c.style.webkit_text_stroke_width?.to_px(&c.sizing, 0.0));
            let color = c
                .style
                .webkit_text_stroke_color
                .map_or(c.current_color, |ci| ci.resolve(c.current_color));
            (w > 0.0).then(|| (w, rgba_of(color)))
        });
        // Shadow the combined fill and stroke shape, preserving each component's alpha. Transparent
        // fills therefore produce outline-shaped shadows.
        let (paint, stroke) = match shadow {
            None => (
                Paint::Solid(rgba_of(shaped.brush.color)),
                declared_stroke.map(|(w, c)| valle_draw::program::recording::Stroke::solid(c, w)),
            ),
            Some((_, _, sc)) => {
                let tint = |a: u8| {
                    Rgba::new(
                        sc.r,
                        sc.g,
                        sc.b,
                        ((u16::from(sc.a) * u16::from(a)) / 255) as u8,
                    )
                };
                (
                    Paint::Solid(tint(rgba_of(shaped.brush.color).a)),
                    declared_stroke
                        .map(|(w, c)| valle_draw::program::recording::Stroke::solid(tint(c.a), w)),
                )
            }
        };
        // Split runs only when per-unit styles are present.
        let units = source_ranges.and_then(|source| {
            let key = self
                .out
                .recording
                .text_sources
                .get(source.node.index())?
                .clone();
            let units = std::rc::Rc::clone(&self.tree.units);
            units.get(&key).cloned()
        });
        // Takumi carries inline `<Span>` opacity on the shaped run rather than multiplying it into
        // the fill color. Rich-text children do not become independent paint nodes, so the normal
        // `begin_node` opacity path never sees this value. Wrap every emitted layer of the run
        // (fill/stroke and text-shadow alike) with the existing ProgramRecording opacity primitive.
        let run_opacity = f64::from(shaped.brush.opacity).clamp(0.0, 1.0);
        let text_fit_transform = affine_of(run.transform(TAffine::IDENTITY));
        if text_fit_transform != Affine::IDENTITY {
            self.push(RecordCmd::BeginTransform {
                transform: text_fit_transform,
            });
        }
        if run_opacity < 1.0 {
            self.push(RecordCmd::BeginOpacity { alpha: run_opacity });
        }
        self.glyph_chunks(
            font,
            span,
            source_ranges,
            units.as_deref(),
            &glyphs,
            &paint,
            &stroke,
            placements,
        );
        if run_opacity < 1.0 {
            self.push(RecordCmd::End);
        }
        if text_fit_transform != Affine::IDENTITY {
            self.push(RecordCmd::End);
        }
    }

    /// Emit run segments using existing opacity and transform groups for per-unit animation and
    /// text paths. Use each unit's horizontal glyph midpoint and baseline as its transform pivot.
    #[allow(clippy::too_many_arguments)]
    fn glyph_chunks(
        &mut self,
        font: valle_draw::program::recording::FontId,
        span: Span,
        source: Option<GlyphSource>,
        units: Option<&[crate::layout::bridge::ResolvedUnit]>,
        glyphs: &[Glyph],
        paint: &Paint,
        stroke: &Option<Stroke>,
        placements: Option<&[Option<Affine>]>,
    ) {
        let ranges = source.map(|source| {
            self.out.recording.glyph_source_ranges
                [source.ranges.start as usize..source.ranges.end as usize]
                .to_vec()
        });
        // Assign each glyph to the unit containing its cluster's first byte; ligature clusters may
        // leave other units without glyphs.
        let unit_of = |at: usize| -> Option<usize> {
            let start = ranges.as_ref()?.get(at)?.start;
            units?
                .iter()
                .position(|unit| unit.start <= start && start < unit.end)
        };
        let sub_span = |from: usize, to: usize| Span {
            start: span.start + from as u32,
            end: span.start + to as u32,
        };
        let sub_source = |from: usize, to: usize| {
            source.map(|source| GlyphSource {
                node: source.node,
                ranges: Span {
                    start: source.ranges.start + from as u32,
                    end: source.ranges.start + to as u32,
                },
            })
        };
        let mut at = 0usize;
        while at < glyphs.len() {
            let current = unit_of(at);
            let mut end = at + 1;
            while end < glyphs.len() && unit_of(end) == current {
                end += 1;
            }
            let unit = current
                .and_then(|index| units.and_then(|units| units.get(index)))
                .copied();
            let color = unit
                .and_then(|unit| unit.color)
                .map_or(*paint, Paint::Solid);
            let unit_affine = unit.and_then(|unit| unit_transform(unit, &glyphs[at..end]));
            // Apply the same unit decoration inside any path transform.
            let decorate = |emitter: &mut Self| -> usize {
                let mut depth = 0;
                if let Some(alpha) = unit.and_then(|unit| unit.opacity) {
                    emitter.push(RecordCmd::BeginOpacity {
                        alpha: alpha.clamp(0.0, 1.0),
                    });
                    depth += 1;
                }
                if let Some(transform) = unit_affine {
                    emitter.push(RecordCmd::BeginTransform { transform });
                    depth += 1;
                }
                depth
            };
            match placements {
                None => {
                    let depth = decorate(self);
                    self.push(RecordCmd::GlyphRun {
                        font,
                        glyphs: sub_span(at, end),
                        paint: color,
                        stroke: stroke.clone(),
                        source: sub_source(at, end),
                    });
                    for _ in 0..depth {
                        self.push(RecordCmd::End);
                    }
                }
                Some(placements) => {
                    for index in at..end {
                        // Keep the path transform outside the unit transform so glyph-local
                        // rotation remains local.
                        let Some(transform) = placements.get(index).copied().flatten() else {
                            continue;
                        };
                        self.push(RecordCmd::BeginTransform { transform });
                        let depth = 1 + decorate(self);
                        self.push(RecordCmd::GlyphRun {
                            font,
                            glyphs: sub_span(index, index + 1),
                            paint: color,
                            stroke: stroke.clone(),
                            source: sub_source(index, index + 1),
                        });
                        for _ in 0..depth {
                            self.push(RecordCmd::End);
                        }
                    }
                }
            }
            at = end;
        }
    }

    fn background(
        &mut self,
        np: &NodePaint,
        style: &ComputedStyle,
        node: &RenderNode,
        layout: &takumi_core::geometry::ComputedLayout,
        border_box: Rect,
        radii: [Point; 4],
    ) {
        // Paint the background color beneath background images.
        let color = style.background_color.resolve(node.context.current_color);
        if color.0[3] != 0 {
            self.fill_box(border_box, radii, rgba_of(color));
        }
        // Use upstream gradient tiling, sizing, and positioning geometry.
        // Tiles already arrive in CSS paint order, bottom first.
        let Some(images) = &style.background_image else {
            return;
        };

        let origin = background_origin_box(style.background_origin, *layout);
        let layers = resolve_background_layers(ResolveBackgroundLayersInput {
            images,
            positions: &style.background_position,
            sizes: &style.background_size,
            repeats: &style.background_repeat,
            blend_modes: &style.background_blend_mode,
            context: &node.context,
            area: takumi_core::geometry::Size {
                width: origin.size.width.max(0.0).round() as u32,
                height: origin.size.height.max(0.0).round() as u32,
            },
            paint: takumi_core::geometry::Size {
                width: border_box.width.max(0.0).round() as u32,
                height: border_box.height.max(0.0).round() as u32,
            },
            origin_offset: takumi_core::geometry::Point {
                x: origin.offset.x.round() as i32,
                y: origin.offset.y.round() as i32,
            },
        });

        // Tile rectangles can extend past the painting area. Clip once to the real rounded
        // border box, then each tile can stay a plain rectangle with its own gradient geometry.
        if radii.iter().any(radius_is_visible) {
            self.push(RecordCmd::BeginClipRoundRect {
                rrect: RoundRect {
                    rect: border_box,
                    radii,
                },
            });
        } else {
            self.push(RecordCmd::BeginClipRect { rect: border_box });
        }
        for (index, geometry) in layers {
            let image = &images[index];
            for &y in &geometry.ys {
                for &x in &geometry.xs {
                    let tile = Rect::new(
                        border_box.x + f64::from(x),
                        border_box.y + f64::from(y),
                        f64::from(geometry.tile_width),
                        f64::from(geometry.tile_height),
                    );
                    match self.gradient_paint(image, node, tile) {
                        Some(paint) => self.fill_box_with(tile, [Point::new(0.0, 0.0); 4], paint),
                        None => {
                            self.unsupported(np, "background-image (URL or unsupported gradient)");
                            break;
                        }
                    }
                }
            }
        }
        self.push(RecordCmd::End);
    }

    /// Translate upstream gradient geometry and resolved stops into recording paint without
    /// duplicating CSS calculations. Return None for unsupported forms.
    fn gradient_paint(
        &mut self,
        image: &takumi_core::style::BackgroundImage,
        node: &RenderNode,
        border_box: Rect,
    ) -> Option<Paint> {
        use takumi_core::style::BackgroundImage as BG;
        let ctx = &node.context;
        let (w, h) = (border_box.width as f32, border_box.height as f32);
        let (cx, cy) = (
            border_box.x + border_box.width / 2.0,
            border_box.y + border_box.height / 2.0,
        );
        // Repeating gradients use Repeat; others extend endpoint colors with Pad.
        let spread = |repeating: bool| {
            if repeating {
                valle_draw::program::recording::SpreadMode::Repeat
            } else {
                valle_draw::program::recording::SpreadMode::Pad
            }
        };

        match image {
            BG::Linear(g) => {
                // Do not call Takumi's host `f32::sin/cos/hypot` direction path here: canonical
                // ProgramRecording bytes must agree between Native and wasm for repeating and regular
                // gradients alike.
                let (dir_x, dir_y) = deterministic_linear_direction(g.direction, w, h)?;
                let axis_f32 =
                    (f64::from(w) * dir_x.abs() + f64::from(h) * dir_y.abs()).max(1e-6) as f32;
                let resolved = takumi_core::paint::resolve_stops_along_axis(
                    &g.stops,
                    axis_f32,
                    &ctx.sizing,
                    ctx.current_color,
                );
                if resolved.is_empty() {
                    return None;
                }
                let axis = f64::from(axis_f32);
                // Use the first-to-last stop period as the repeating gradient axis, taking its
                // start and extent from upstream geometry.
                let (t0, extent) = if g.repeating {
                    let repeat_start = resolved.first()?.position;
                    let period = f64::from(resolved.last()?.position - repeat_start);
                    if period <= 0.0 {
                        // Report zero-period repeating gradients as unsupported by the executor's
                        // stop representation.
                        return None;
                    }
                    (f64::from(repeat_start), period)
                } else {
                    (0.0, axis)
                };
                // Map axis positions around the box center into canvas points.
                let half = axis / 2.0;
                let point_at = |t: f64| {
                    valle_draw::Point::new(cx + (t - half) * dir_x, cy + (t - half) * dir_y)
                };
                let stops: Vec<(f64, Rgba)> = resolved
                    .iter()
                    .map(|s| ((f64::from(s.position) - t0) / extent, rgba_of(s.color)))
                    .collect();
                let span = self.out.recording.intern_stops(&stops);
                Some(Paint::Linear(
                    valle_draw::program::recording::LinearGradient {
                        start: point_at(t0),
                        end: point_at(t0 + extent),
                        stops: span,
                        spread: spread(g.repeating),
                        alpha: 1.0,
                    },
                ))
            }
            BG::Radial(g) => {
                let tile = takumi_core::paint::RadialGradientTile::new(
                    g,
                    w.max(1.0) as u32,
                    h.max(1.0) as u32,
                    &ctx.sizing,
                    ctx.current_color,
                );
                let resolved = takumi_core::paint::resolve_stops_along_axis(
                    &g.stops,
                    tile.radius_scale.max(1e-6),
                    &ctx.sizing,
                    ctx.current_color,
                );
                let (t0, extent) = if g.repeating {
                    if tile.repeat_period <= 1e-6 {
                        return None;
                    }
                    (tile.repeat_start, tile.repeat_period)
                } else {
                    (0.0, tile.radius_scale.max(1e-6))
                };
                let stops = resolved
                    .iter()
                    .map(|stop| {
                        (
                            f64::from((stop.position - t0) / extent),
                            rgba_of(stop.color),
                        )
                    })
                    .collect::<Vec<_>>();
                let span = self.out.recording.intern_stops(&stops);
                let full_radius = f64::from(tile.radius_scale.max(1e-6));
                let tile_scale = f64::from(extent) / full_radius;
                Some(Paint::Radial(
                    valle_draw::program::recording::RadialGradient {
                        center: Point::new(
                            border_box.x + f64::from(tile.cx),
                            border_box.y + f64::from(tile.cy),
                        ),
                        radii: Point::new(
                            f64::from(tile.inv_radius_x.recip()) * tile_scale,
                            f64::from(tile.inv_radius_y.recip()) * tile_scale,
                        ),
                        stops: span,
                        spread: spread(g.repeating),
                        alpha: 1.0,
                    },
                ))
            }
            BG::Conic(g) => {
                let tile = takumi_core::paint::ConicGradientTile::new(
                    g,
                    w.max(1.0) as u32,
                    h.max(1.0) as u32,
                    &ctx.sizing,
                    ctx.current_color,
                );
                let resolved = takumi_core::paint::resolve_stops_along_axis(
                    &g.stops,
                    360.0,
                    &ctx.sizing,
                    ctx.current_color,
                );
                let (t0, extent) = if g.repeating {
                    if tile.repeat_period_deg <= 1e-6 {
                        return None;
                    }
                    (tile.repeat_start_deg, tile.repeat_period_deg)
                } else {
                    (0.0, 360.0)
                };
                let stops = resolved
                    .iter()
                    .map(|stop| {
                        (
                            f64::from((stop.position - t0) / extent),
                            rgba_of(stop.color),
                        )
                    })
                    .collect::<Vec<_>>();
                let span = self.out.recording.intern_stops(&stops);
                Some(Paint::Conic(
                    valle_draw::program::recording::ConicGradient {
                        center: Point::new(
                            border_box.x + f64::from(tile.cx),
                            border_box.y + f64::from(tile.cy),
                        ),
                        // CSS angles start at twelve o'clock; Skia/ProgramRecording zero is three o'clock.
                        start_angle: f64::from(*g.from_angle) - 90.0 + f64::from(t0),
                        sweep_angle: f64::from(extent),
                        stops: span,
                        spread: spread(g.repeating),
                        alpha: 1.0,
                    },
                ))
            }
            BG::None | BG::Url(_) => None,
        }
    }

    fn border(
        &mut self,
        np: &NodePaint,
        style: &ComputedStyle,
        node: &RenderNode,
        border_box: Rect,
        radii: [Point; 4],
        layout: &takumi_core::geometry::ComputedLayout,
    ) {
        let w = layout.border;
        if w.left <= 0.0 && w.right <= 0.0 && w.top <= 0.0 && w.bottom <= 0.0 {
            return;
        }
        let c = node.context.current_color;
        let (tl, tr, br, bl) = (
            style.border_top_color.resolve(c),
            style.border_right_color.resolve(c),
            style.border_bottom_color.resolve(c),
            style.border_left_color.resolve(c),
        );
        use takumi_core::style::BorderStyle;
        let styles = [
            style.border_top_style,
            style.border_right_style,
            style.border_bottom_style,
            style.border_left_style,
        ];

        // Uniform dashed/dotted borders use strokes; double borders use two rings each one-third of
        // the total width.
        let uniform = tl == tr
            && tr == br
            && br == bl
            && w.left == w.right
            && w.top == w.bottom
            && w.left == w.top
            && styles.iter().all(|style| *style == styles[0]);
        if uniform {
            if !self.styled_ring(border_box, radii, f64::from(w.left), rgba_of(tl), styles[0]) {
                self.unsupported(np, "groove/ridge/inset/outset border styles");
            }
            return;
        }

        // Support differing edge geometry only for solid, none, and hidden styles; do not silently
        // replace per-edge dashed borders with solid ones.
        if !styles.iter().all(|style| {
            matches!(
                style,
                BorderStyle::Solid | BorderStyle::None | BorderStyle::Hidden
            )
        }) {
            self.unsupported(
                np,
                "non-solid borders with different per-edge widths or colors",
            );
            return;
        }

        // Use upstream side polygons for differing border widths and colors.
        //
        // For rounded borders, clip each side and fill its ring; unrounded sides use direct
        // polygons.
        use takumi_core::layout::border::BorderSide;
        let props = takumi_core::layout::border::BorderProperties::from_context(
            &node.context,
            layout.size,
            w,
        );
        let rounded = radii.iter().any(radius_is_visible);
        let sides = [
            (BorderSide::Top, w.top, tl, style.border_top_style),
            (BorderSide::Right, w.right, tr, style.border_right_style),
            (BorderSide::Bottom, w.bottom, br, style.border_bottom_style),
            (BorderSide::Left, w.left, bl, style.border_left_style),
        ];
        for (side, width, color, side_style) in sides {
            // Skip invisible edges.
            if !(side_style.is_rendered() && width > 0.0) || color.0[3] == 0 {
                continue;
            }
            let mut cmds = Vec::new();
            if rounded {
                props.append_side_clip_polygon_commands_at(
                    side,
                    &mut cmds,
                    layout.size,
                    takumi_core::geometry::Point { x: 0.0, y: 0.0 },
                );
                let Some(path) = self.path_of(&cmds) else {
                    continue;
                };
                self.push(RecordCmd::BeginClipPath {
                    path,
                    fill_rule: FillRule::NonZero,
                });
                // Use each side's own ring width and clipping polygon.
                self.ring(border_box, radii, f64::from(width), rgba_of(color));
                self.push(RecordCmd::End);
            } else {
                props.append_side_polygon_commands_at(
                    side,
                    &mut cmds,
                    layout.size,
                    takumi_core::geometry::Point { x: 0.0, y: 0.0 },
                );
                let Some(path) = self.path_of(&cmds) else {
                    continue;
                };
                self.push(RecordCmd::Path {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(Paint::Solid(rgba_of(color))),
                    stroke: None,
                });
            }
        }
    }

    /// Convert upstream path commands into recording buffers; empty paths return None.
    fn path_of(
        &mut self,
        cmds: &[takumi_core::geometry::PathCommand],
    ) -> Option<valle_draw::PathRef> {
        use takumi_core::geometry::PathCommand as C;
        if cmds.is_empty() {
            return None;
        }
        let mut p = self.out.recording.begin_path();
        for c in cmds {
            let pt = |q: &takumi_core::geometry::Point<f32>| {
                valle_draw::Point::new(f64::from(q.x), f64::from(q.y))
            };
            p = match c {
                C::MoveTo(a) => p.move_to(pt(a)),
                C::LineTo(a) => p.line_to(pt(a)),
                C::QuadTo(a, b) => p.quad_to(pt(a), pt(b)),
                C::CubicTo(a, b, c2) => p.cubic_to(pt(a), pt(b), pt(c2)),
                C::Close => p.close(),
            };
        }
        Some(p.finish())
    }

    /// Emit CSS outlines using upstream BoxPainter geometry, including rounded corners, offsets,
    /// and width clamping.
    fn outline(
        &mut self,
        np: &NodePaint,
        node: &RenderNode,
        layout: &takumi_core::geometry::ComputedLayout,
    ) {
        let style = &node.context.style;
        if !style.outline_style.is_rendered() {
            return;
        }
        let Some(geo) = takumi_core::painter::BoxPainter::new(&node.context, *layout).outline()
        else {
            return;
        };
        let width = f64::from(geo.border.width.top);
        let color = geo.border.color.top;
        if width <= 0.0 || color.0[3] == 0 {
            return;
        }
        let grow = f64::from(geo.grow);
        let outer = Rect::new(
            -grow,
            -grow,
            f64::from(geo.size.width),
            f64::from(geo.size.height),
        );
        let mut radii = [Point::default(); 4];
        for (i, p) in geo.border.radius.0.iter().enumerate() {
            radii[i] = Point::new(f64::from(p.x), f64::from(p.y));
        }
        if !self.styled_ring(outer, radii, width, rgba_of(color), style.outline_style) {
            self.unsupported(np, "groove/ridge/inset/outset outline styles");
        }
    }

    /// Uniform CSS border/outline style over one rounded rectangle.
    fn styled_ring(
        &mut self,
        outer: Rect,
        radii: [Point; 4],
        width: f64,
        color: Rgba,
        style: takumi_core::style::BorderStyle,
    ) -> bool {
        use takumi_core::style::BorderStyle;
        if width <= 0.0 || color.a == 0 {
            return true;
        }
        match style {
            BorderStyle::None | BorderStyle::Hidden => {}
            BorderStyle::Solid => self.ring(outer, radii, width, color),
            BorderStyle::Dashed => self.stroke_ring(
                outer,
                radii,
                width,
                color,
                vec![3.0 * width, 3.0 * width],
                Cap::Butt,
            ),
            BorderStyle::Dotted => {
                // Skia requires strictly positive dash intervals. A tiny on-segment plus round cap
                // is a stable dot whose visible diameter is the authored border width.
                self.stroke_ring(
                    outer,
                    radii,
                    width,
                    color,
                    vec![1e-6, 2.0 * width],
                    Cap::Round,
                )
            }
            BorderStyle::Double => {
                let third = width / 3.0;
                self.ring(outer, radii, third, color);
                let inset = 2.0 * third;
                let inner_outer = Rect::new(
                    outer.x + inset,
                    outer.y + inset,
                    (outer.width - 2.0 * inset).max(0.0),
                    (outer.height - 2.0 * inset).max(0.0),
                );
                let inner_radii = radii.map(|radius| {
                    Point::new((radius.x - inset).max(0.0), (radius.y - inset).max(0.0))
                });
                self.ring(inner_outer, inner_radii, third, color);
            }
            BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
                // These styles require light/dark color derivation that is not in ProgramRecording.
                // Artifact admission keeps them out of the public Tailwind catalog; raw CSS is
                // still reported by the caller's unsupported ledger.
                return false;
            }
        }
        true
    }

    fn stroke_ring(
        &mut self,
        outer: Rect,
        radii: [Point; 4],
        width: f64,
        color: Rgba,
        dash: Vec<f64>,
        cap: Cap,
    ) {
        self.painted_stroke_ring(outer, radii, width, Paint::Solid(color), Some(dash), cap);
    }

    fn painted_stroke_ring(
        &mut self,
        outer: Rect,
        radii: [Point; 4],
        width: f64,
        paint: Paint,
        dash: Option<Vec<f64>>,
        cap: Cap,
    ) {
        let half = width / 2.0;
        let center = Rect::new(
            outer.x + half,
            outer.y + half,
            (outer.width - width).max(0.0),
            (outer.height - width).max(0.0),
        );
        let center_radii =
            radii.map(|radius| Point::new((radius.x - half).max(0.0), (radius.y - half).max(0.0)));
        let mut path = self.out.recording.begin_path();
        path = push_rrect(path, center, center_radii);
        let path = path.finish();
        self.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::NonZero,
            fill: None,
            stroke: Some(Stroke {
                paint,
                width,
                dash,
                dash_offset: 0.0,
                cap,
                join: Join::Round,
                miter_limit: 4.0,
            }),
        });
    }

    /// Build a uniform ring from outer and inner rounded rectangles using even-odd fill.
    fn ring(&mut self, outer: Rect, radii: [Point; 4], width: f64, color: Rgba) {
        let inner = Rect::new(
            outer.x + width,
            outer.y + width,
            (outer.width - 2.0 * width).max(0.0),
            (outer.height - 2.0 * width).max(0.0),
        );
        let inner_radii = radii.map(|r| Point::new((r.x - width).max(0.0), (r.y - width).max(0.0)));
        let mut p = self.out.recording.begin_path();
        p = push_rrect(p, outer, radii);
        p = push_rrect(p, inner, inner_radii);
        let path = p.finish();
        self.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::EvenOdd,
            fill: Some(Paint::Solid(color)),
            stroke: None,
        });
    }

    /// Resolve shadow lengths and colors through upstream sizing rules.
    fn contact_shadow(&mut self, border_box: Rect, amount: f64) {
        let amount = amount.clamp(0.0, 1.0);
        // One desk-space oval. Two stacked blobs read as stripes under a pile.
        let width = border_box.width * (0.78 + amount * 0.06);
        let height = 18.0 + amount * 16.0;
        self.push_desk_shadow(
            Rect::new(
                (border_box.width - width) * 0.5,
                border_box.height - height * 0.35,
                width,
                height,
            ),
            6.0 + amount * 14.0,
            8.0 + amount * 12.0,
            -2.0,
            (40.0 + amount * 38.0) as u8,
        );
    }

    fn push_desk_shadow(&mut self, rect: Rect, dy: f64, blur_sigma: f64, spread: f64, alpha: u8) {
        if alpha == 0 {
            return;
        }
        let radius = rect.height * 0.5;
        self.push(RecordCmd::Shadow {
            rrect: RoundRect {
                rect,
                radii: [Point::new(radius, radius); 4],
            },
            dx: 0.0,
            dy,
            blur_sigma,
            spread,
            color: Rgba::new(0, 0, 0, alpha),
            inset: false,
        });
    }

    fn paper_grain(&mut self, border_box: Rect, amount: f64, key: &str) {
        let remaining = valle_draw::program::recording::MAX_BATCH_INSTANCES_PER_RECORDING
            .saturating_sub(self.reserved_batch_instances)
            .saturating_sub(self.grain_instances);
        let (specks, fibers) =
            paper_grain_counts(border_box.width * border_box.height, amount, remaining);
        if specks + fibers == 0 {
            return;
        }
        let mut state = fnv1a(key.as_bytes());
        if specks > 0 {
            let mut instances = Vec::with_capacity(specks);
            for _ in 0..specks {
                let x = border_box.x + 2.0 + grain_unit(&mut state) * (border_box.width - 4.0);
                let y = border_box.y + 2.0 + grain_unit(&mut state) * (border_box.height - 4.0);
                let dark = grain_unit(&mut state) > 0.38;
                let size = 0.7 + grain_unit(&mut state) * 0.9;
                let alpha = if dark {
                    18.0 + amount * 36.0
                } else {
                    12.0 + amount * 22.0
                };
                instances.push(BatchInstance {
                    position: Point::new(x, y),
                    size: Point::new(size, size),
                    color: if dark {
                        Rgba::new(96, 82, 64, alpha as u8)
                    } else {
                        Rgba::new(255, 252, 246, alpha as u8)
                    },
                });
            }
            let instances = self.out.recording.intern_batch_instances(&instances);
            self.grain_instances += specks;
            self.push(RecordCmd::GeometryBatch {
                geometry: BatchGeometry::Circle,
                instances,
            });
        }
        if fibers > 0 {
            let mut instances = Vec::with_capacity(fibers);
            for _ in 0..fibers {
                let x = border_box.x + 4.0 + grain_unit(&mut state) * (border_box.width - 8.0);
                let y = border_box.y + 4.0 + grain_unit(&mut state) * (border_box.height - 8.0);
                let w = 2.2 + grain_unit(&mut state) * 4.5;
                let h = 0.55 + grain_unit(&mut state) * 0.4;
                let alpha = 14.0 + amount * 28.0;
                instances.push(BatchInstance {
                    position: Point::new(x, y),
                    size: Point::new(w, h),
                    color: Rgba::new(120, 104, 84, alpha as u8),
                });
            }
            let instances = self.out.recording.intern_batch_instances(&instances);
            self.grain_instances += fibers;
            self.push(RecordCmd::GeometryBatch {
                geometry: BatchGeometry::Rect,
                instances,
            });
        }
    }

    fn box_shadows(
        &mut self,
        _np: &NodePaint,
        node: &RenderNode,
        border_box: Rect,
        radii: [Point; 4],
        inset: bool,
    ) {
        let ctx = &node.context;
        let Some(shadows) = ctx.style.box_shadow.clone() else {
            return;
        };
        let size = takumi_core::geometry::Size {
            width: border_box.width as f32,
            height: border_box.height as f32,
        };
        for s in shadows.iter() {
            if s.inset != inset {
                continue;
            }
            let sized = takumi_core::shadow::SizedShadow {
                offset_x: s.offset_x.to_px(&ctx.sizing, size.width),
                offset_y: s.offset_y.to_px(&ctx.sizing, size.height),
                blur_radius: s.blur_radius.to_px(&ctx.sizing, 1.0),
                spread_radius: s.spread_radius.to_px(&ctx.sizing, 1.0),
                color: s.color.resolve(ctx.current_color),
            };
            self.push(RecordCmd::Shadow {
                rrect: RoundRect {
                    rect: border_box,
                    radii,
                },
                dx: f64::from(sized.offset_x),
                dy: f64::from(sized.offset_y),
                // Convert shadow blur radius to Gaussian sigma by dividing by two.
                blur_sigma: f64::from(sized.blur_radius) / 2.0,
                spread: f64::from(sized.spread_radius),
                color: rgba_of(sized.color),
                inset,
            });
        }
    }

    /// Translate filter chains into a side-table span; report unsupported URL filters.
    fn filters(
        &mut self,
        np: &NodePaint,
        filters: &takumi_core::style::Filters,
        ctx: &takumi_core::context::RenderContext,
    ) -> valle_draw::Span {
        let ops = self.filter_ops(np, filters, ctx);
        self.out.recording.intern_filters(&ops)
    }

    fn filter_ops(
        &mut self,
        np: &NodePaint,
        filters: &takumi_core::style::Filters,
        ctx: &takumi_core::context::RenderContext,
    ) -> Vec<valle_draw::program::recording::FilterOp> {
        use takumi_core::style::Filter as TF;
        use valle_draw::program::recording::FilterOp as F;
        let mut ops = Vec::new();
        for f in filters.iter() {
            let op = match f {
                // Use the upstream filter blur-to-sigma conversion convention.
                TF::Blur(v) => F::Blur {
                    sigma: f64::from(v.to_px(&ctx.sizing, 1.0)) / 2.0,
                },
                TF::Brightness(v) => F::Brightness {
                    amount: f64::from(v.0),
                },
                TF::Contrast(v) => F::Contrast {
                    amount: f64::from(v.0),
                },
                TF::Grayscale(v) => F::Grayscale {
                    amount: f64::from(v.0),
                },
                TF::HueRotate(v) => F::HueRotate {
                    degrees: f64::from(**v),
                },
                TF::Invert(v) => F::Invert {
                    amount: f64::from(v.0),
                },
                TF::Opacity(v) => F::Opacity {
                    amount: f64::from(v.0),
                },
                TF::Saturate(v) => F::Saturate {
                    amount: f64::from(v.0),
                },
                TF::Sepia(v) => F::Sepia {
                    amount: f64::from(v.0),
                },
                TF::DropShadow(s) => {
                    // Drop shadows use the painted alpha contour, while box shadows use box
                    // geometry. Resolve lengths upstream and convert radius to sigma consistently.
                    F::DropShadow {
                        dx: f64::from(s.offset_x.to_px(&ctx.sizing, 1.0)),
                        dy: f64::from(s.offset_y.to_px(&ctx.sizing, 1.0)),
                        sigma: f64::from(s.blur_radius.to_px(&ctx.sizing, 1.0)) / 2.0,
                        color: rgba_of(s.color.resolve(ctx.current_color)),
                    }
                }
                // URL filters use an incompatible upstream CPU pipeline and are reported as
                // unsupported.
                _ => {
                    self.unsupported(np, "filter: url()（SVG filter）");
                    continue;
                }
            };
            ops.push(op);
        }
        ops
    }

    fn fill_box(&mut self, rect: Rect, radii: [Point; 4], color: Rgba) {
        self.fill_box_with(rect, radii, Paint::Solid(color));
    }

    fn fill_box_with(&mut self, rect: Rect, radii: [Point; 4], paint: Paint) {
        let p = self.out.recording.begin_path();
        let path = push_rrect(p, rect, radii).finish();
        self.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::NonZero,
            fill: Some(paint),
            stroke: None,
        });
    }

    fn push(&mut self, cmd: RecordCmd) {
        self.out.recording.push(cmd);
    }

    fn node_key<'a>(&'a self, paint: &NodePaint) -> Option<&'a str> {
        self.tree
            .keys
            .get(&u64::from(paint.node_id))
            .or_else(|| self.tree.render_keys.get(&paint.path))
            .map(String::as_str)
    }

    /// Account for formula leaves skipped because their nodes are transparent or hidden,
    /// distinguishing intentional non-paint from missing scene pairing.
    fn account_invisible_formulas(&mut self) {
        let mut path = Vec::new();
        Self::walk_invisible_formulas(
            &self.tree.root,
            &mut path,
            &self.tree.render_keys,
            &self.tree.formulas,
            &mut self.painted_formulas,
        );
    }

    fn walk_invisible_formulas(
        node: &RenderNode,
        path: &mut Vec<usize>,
        render_keys: &std::collections::HashMap<Vec<usize>, String>,
        formulas: &std::collections::HashMap<String, crate::math_formula::FormulaFragment>,
        painted: &mut HashSet<String>,
    ) {
        if node.context.style.is_invisible() {
            Self::mark_formulas_in_subtree(node, path, render_keys, formulas, painted);
            return;
        }
        let Some(children) = node.children.as_deref() else {
            return;
        };
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            Self::walk_invisible_formulas(child, path, render_keys, formulas, painted);
            path.pop();
        }
    }

    fn mark_formulas_in_subtree(
        node: &RenderNode,
        path: &[usize],
        render_keys: &std::collections::HashMap<Vec<usize>, String>,
        formulas: &std::collections::HashMap<String, crate::math_formula::FormulaFragment>,
        painted: &mut HashSet<String>,
    ) {
        if let Some(key) = render_keys.get(path)
            && formulas.contains_key(key)
        {
            painted.insert(key.clone());
        }
        let Some(children) = node.children.as_deref() else {
            return;
        };
        let mut child_path = path.to_vec();
        for (index, child) in children.iter().enumerate() {
            child_path.push(index);
            Self::mark_formulas_in_subtree(child, &child_path, render_keys, formulas, painted);
            child_path.pop();
        }
    }

    fn report_unplaced_formulas(&mut self) {
        for key in self.tree.formulas.keys() {
            if !self.painted_formulas.contains(key) {
                self.out.unsupported.push((
                    key.clone(),
                    "formula fragment not placed by Takumi replaced leaf",
                ));
            }
        }
    }

    fn blit_formula(
        &mut self,
        key: &str,
        fragment: &crate::math_formula::FormulaFragment,
        origin: valle_draw::Point,
    ) {
        self.painted_formulas.insert(key.to_owned());
        use valle_draw::program::recording::{Glyph, Paint, RecordCmd};
        for cmd in &fragment.list.cmds {
            match cmd {
                RecordCmd::GlyphRun {
                    font,
                    glyphs,
                    paint,
                    stroke,
                    source: _,
                } => {
                    let face = fragment.list.fonts[font.index()].clone();
                    let interned = self.out.recording.intern_font(face);
                    let shifted: Vec<Glyph> = fragment.list.glyphs[glyphs.range()]
                        .iter()
                        .map(|glyph| Glyph {
                            id: glyph.id,
                            x: glyph.x + origin.x,
                            y: glyph.y + origin.y,
                        })
                        .collect();
                    let span = self.out.recording.intern_glyphs(&shifted);
                    self.push(RecordCmd::GlyphRun {
                        font: interned,
                        glyphs: span,
                        paint: paint.clone(),
                        stroke: stroke.clone(),
                        source: None,
                    });
                }
                RecordCmd::Path {
                    path,
                    fill_rule,
                    fill,
                    stroke,
                } => {
                    let verbs = &fragment.list.verbs[path.verbs.range()];
                    let points: Vec<valle_draw::Point> = fragment.list.points[path.points.range()]
                        .iter()
                        .map(|point| valle_draw::Point::new(point.x + origin.x, point.y + origin.y))
                        .collect();
                    let mut builder = self.out.recording.begin_path();
                    let mut pi = 0usize;
                    for verb in verbs {
                        builder = match verb {
                            valle_draw::PathVerb::Move => {
                                let p = points[pi];
                                pi += 1;
                                builder.move_to(p)
                            }
                            valle_draw::PathVerb::Line => {
                                let p = points[pi];
                                pi += 1;
                                builder.line_to(p)
                            }
                            valle_draw::PathVerb::Quad => {
                                let c = points[pi];
                                let p = points[pi + 1];
                                pi += 2;
                                builder.quad_to(c, p)
                            }
                            valle_draw::PathVerb::Cubic => {
                                let c1 = points[pi];
                                let c2 = points[pi + 1];
                                let p = points[pi + 2];
                                pi += 3;
                                builder.cubic_to(c1, c2, p)
                            }
                            valle_draw::PathVerb::Close => builder.close(),
                        };
                    }
                    let path_ref = builder.finish();
                    self.push(RecordCmd::Path {
                        path: path_ref,
                        fill_rule: *fill_rule,
                        fill: fill.clone(),
                        stroke: stroke.clone(),
                    });
                }
                _ => {}
            }
        }
        let _ = Paint::Solid;
    }

    fn unsupported(&mut self, np: &NodePaint, what: &'static str) {
        let key = self
            .tree
            .keys
            .get(&u64::from(np.node_id))
            .cloned()
            .unwrap_or_else(|| format!("{:?}", np.path));
        if !self
            .out
            .unsupported
            .iter()
            .any(|(k, w)| k == &key && *w == what)
        {
            self.out.unsupported.push((key, what));
        }
    }
}

/// Read the stable asset ID stored in an image node's URL representation.
fn image_asset_of(img: &takumi_core::layout::node::ImageData) -> String {
    match &img.src {
        takumi_core::layout::node::ImageSourceInput::Url(u) => u.to_string(),
        // Unexpected image representations produce an unresolved ID rather than selecting incorrect
        // content.
        _ => String::new(),
    }
}

/// Shared padding-box clipping geometry for emission and location queries.
pub(crate) fn padding_box(layout: &takumi_core::geometry::ComputedLayout) -> Rect {
    let b = layout.border;
    Rect::new(
        f64::from(b.left),
        f64::from(b.top),
        f64::from((layout.size.width - b.left - b.right).max(0.0)),
        f64::from((layout.size.height - b.top - b.bottom).max(0.0)),
    )
}

/// Content box equals border box minus borders and padding; replaced image content belongs here.
fn content_box(layout: &takumi_core::geometry::ComputedLayout) -> Rect {
    Rect::new(
        f64::from(layout.border.left + layout.padding.left),
        f64::from(layout.border.top + layout.padding.top),
        f64::from(layout.content_box_width().max(0.0)),
        f64::from(layout.content_box_height().max(0.0)),
    )
}

/// Shared inner-clip geometry with independent horizontal and vertical corner radii reduced by
/// their corresponding borders.
fn inner_clip(
    layout: &takumi_core::geometry::ComputedLayout,
    radii: [Point; 4],
) -> (Rect, [Point; 4]) {
    let b = layout.border;
    let clip_radii = [
        Point::new(
            (radii[0].x - f64::from(b.left)).max(0.0),
            (radii[0].y - f64::from(b.top)).max(0.0),
        ),
        Point::new(
            (radii[1].x - f64::from(b.right)).max(0.0),
            (radii[1].y - f64::from(b.top)).max(0.0),
        ),
        Point::new(
            (radii[2].x - f64::from(b.right)).max(0.0),
            (radii[2].y - f64::from(b.bottom)).max(0.0),
        ),
        Point::new(
            (radii[3].x - f64::from(b.left)).max(0.0),
            (radii[3].y - f64::from(b.bottom)).max(0.0),
        ),
    ];
    (padding_box(layout), clip_radii)
}

/// Shared predicate for own-text emission and missing-glyph reporting.
fn has_own_text(node: &RenderNode) -> bool {
    node.node
        .as_ref()
        .is_some_and(|inner| matches!(inner.kind, takumi_core::layout::node::NodeKind::Text(_)))
}

/// Build rounded-rectangle paths with corners ordered top-left, top-right, bottom-right,
/// bottom-left.
fn push_rrect<S: valle_draw::PathSink>(
    p: valle_draw::PathBuilder<'_, S>,
    rect: Rect,
    radii: [Point; 4],
) -> valle_draw::PathBuilder<'_, S> {
    if radii.iter().all(|r| !radius_is_visible(r)) {
        p.rect(rect)
    } else {
        p.elliptical_rrect(rect, radii)
    }
}

/// Convert Takumi affine components to the recording's matching six-element f64 representation.
fn affine_of(a: TAffine) -> Affine {
    Affine([
        f64::from(a.a),
        f64::from(a.b),
        f64::from(a.c),
        f64::from(a.d),
        f64::from(a.x),
        f64::from(a.y),
    ])
}

fn affine_from_glass_matrix(matrix: [f64; 9]) -> Option<TAffine> {
    if !matrix.into_iter().all(f64::is_finite)
        || matrix[6].abs() > 1e-9
        || matrix[7].abs() > 1e-9
        || (matrix[8] - 1.0).abs() > 1e-9
    {
        return None;
    }
    let values = [
        matrix[0], matrix[3], matrix[1], matrix[4], matrix[2], matrix[5],
    ];
    if values
        .into_iter()
        .any(|value| value < f64::from(f32::MIN) || value > f64::from(f32::MAX))
    {
        return None;
    }
    Some(TAffine {
        a: matrix[0] as f32,
        b: matrix[3] as f32,
        c: matrix[1] as f32,
        d: matrix[4] as f32,
        x: matrix[2] as f32,
        y: matrix[5] as f32,
    })
}

fn rgba_of(c: Color) -> Rgba {
    Rgba::new(c.0[0], c.0[1], c.0[2], c.0[3])
}

/// CSS linear-gradient direction using Valle's fixed math narrow waist. Cardinal directions stay
/// exact, while arbitrary angles and diagonal keyword normalization use the same libm-rs code on
/// Native and wasm.
fn deterministic_linear_direction(
    direction: takumi_core::style::LinearGradientDirection,
    width: f32,
    height: f32,
) -> Option<(f64, f64)> {
    use takumi_core::style::{HorizontalKeyword, LinearGradientDirection, VerticalKeyword};

    match direction {
        LinearGradientDirection::Angle(angle) => {
            let degrees = f64::from(*angle).rem_euclid(360.0);
            let exact = match degrees {
                0.0 => Some((0.0, -1.0)),
                90.0 => Some((1.0, 0.0)),
                180.0 => Some((0.0, 1.0)),
                270.0 => Some((-1.0, 0.0)),
                _ => None,
            };
            exact.or_else(|| {
                let radians = degrees * core::f64::consts::PI / 180.0;
                Some((
                    valle_draw::math::sin(radians),
                    -valle_draw::math::cos(radians),
                ))
            })
        }
        LinearGradientDirection::Keyword(keyword) => {
            let horizontal = match keyword.horizontal {
                Some(HorizontalKeyword::Left) => Some(-1.0),
                Some(HorizontalKeyword::Right) => Some(1.0),
                Some(_) => return None,
                None => None,
            };
            let vertical = match keyword.vertical {
                Some(VerticalKeyword::Top) => Some(-1.0),
                Some(VerticalKeyword::Bottom) => Some(1.0),
                Some(_) => return None,
                None => None,
            };
            match (horizontal, vertical) {
                (Some(x), None) => Some((x, 0.0)),
                (None, Some(y)) => Some((0.0, y)),
                (Some(x), Some(y)) => {
                    let dx = x * f64::from(height);
                    let dy = y * f64::from(width);
                    let magnitude = valle_draw::math::sqrt(dx * dx + dy * dy);
                    (magnitude > f64::EPSILON).then_some((dx / magnitude, dy / magnitude))
                }
                (None, None) => None,
            }
        }
    }
}

/// Resolve corner radii through upstream sizing in top-left, top-right, bottom-right, bottom-left
/// order, preserving independent horizontal and vertical radii.
fn radii_of(ctx: &takumi_core::context::RenderContext, w: f32, h: f32) -> [Point; 4] {
    let sides = takumi_core::layout::border::BorderProperties::from_context(
        ctx,
        takumi_core::geometry::Size {
            width: w,
            height: h,
        },
        Default::default(),
    )
    .radius;
    let mut out = [Point::default(); 4];
    for (i, p) in sides.0.iter().enumerate() {
        out[i] = Point::new(f64::from(p.x), f64::from(p.y));
    }
    out
}

fn radius_is_visible(radius: &Point) -> bool {
    radius.x > 0.0 && radius.y > 0.0
}

/// Map CSS blending to recording modes; Normal requires no blend group.
fn blend_of(style: &ComputedStyle) -> Option<valle_draw::program::recording::BlendMode> {
    use takumi_core::style::BlendMode as T;
    use valle_draw::program::recording::BlendMode as D;
    Some(match style.mix_blend_mode {
        T::Normal => return None,
        T::Multiply => D::Multiply,
        T::Screen => D::Screen,
        T::Overlay => D::Overlay,
        T::Darken => D::Darken,
        T::Lighten => D::Lighten,
        T::ColorDodge => D::ColorDodge,
        T::ColorBurn => D::ColorBurn,
        T::HardLight => D::HardLight,
        T::SoftLight => D::SoftLight,
        T::Difference => D::Difference,
        T::Exclusion => D::Exclusion,
        T::Hue => D::Hue,
        T::Saturation => D::Saturation,
        T::Color => D::Color,
        T::Luminosity => D::Luminosity,
        // Unknown upstream blend variants fall back to Normal.
        _ => return None,
    })
}

/// Text path with arc length cached per node for all runs and shadows.
struct TextPathPlan {
    path: valle_motion::geometry::PathData,
    length: f64,
}

/// Place each glyph at the midpoint of its advance along the path. Derive advances from successive
/// glyph positions and the run endpoint. A missing plan indicates sampling failure; a missing
/// placement indicates a glyph beyond the path.
fn path_placements(
    plan: &TextPathPlan,
    origin_x: f64,
    run: &PositionedInlineRun,
    layout: takumi_core::geometry::ComputedLayout,
) -> Option<Vec<Option<Affine>>> {
    let shaped = &run.glyph_run;
    let g_off = run.glyph_offset(layout);
    let run_end = f64::from(shaped.offset + shaped.advance);
    let advance_of = |index: usize| -> f64 {
        let x = f64::from(shaped.glyphs[index].x);
        let next = shaped
            .glyphs
            .get(index + 1)
            .map_or(run_end, |glyph| f64::from(glyph.x));
        (next - x).max(0.0)
    };
    // Subtract the node's initial line advance so its text starts at the path origin.
    let progresses: Vec<f64> = (0..shaped.glyphs.len())
        .map(|index| {
            (f64::from(shaped.glyphs[index].x) - origin_x + advance_of(index) / 2.0) / plan.length
        })
        .collect();
    let samples = plan.path.samples_at(&progresses).ok()?;
    Some(
        progresses
            .iter()
            .zip(samples)
            .enumerate()
            .map(|(index, (progress, (point, tangent)))| {
                // Do not clamp out-of-path glyphs onto an endpoint, where they would overlap.
                if !(0.0..=1.0).contains(progress) {
                    return None;
                }
                // Use unshadowed glyph positions as pivots so shadow offsets rotate with the glyph.
                let pivot_x = f64::from(g_off.x + shaped.glyphs[index].x) + advance_of(index) / 2.0;
                let pivot_y = f64::from(g_off.y + shaped.glyphs[index].y);
                let degrees = valle_draw::math::atan2(tangent.y, tangent.x).to_degrees();
                Some(
                    Affine::translate(-pivot_x, -pivot_y)
                        .then(Affine::rotate(degrees))
                        .then(Affine::translate(point.x, point.y)),
                )
            })
            .collect(),
    )
}

/// Compose unit translation, scale, and rotation into one affine transform; omit it when no
/// transform is bound.
fn unit_transform(unit: crate::layout::bridge::ResolvedUnit, glyphs: &[Glyph]) -> Option<Affine> {
    let (translate, scale, rotate) = (unit.translate, unit.scale, unit.rotate);
    if translate.is_none() && scale.is_none() && rotate.is_none() {
        return None;
    }
    // Pivot at the unit's horizontal glyph midpoint on its baseline.
    let first = glyphs.first()?;
    let last = glyphs.last()?;
    let pivot = valle_draw::Point::new((first.x + last.x) / 2.0, first.y);
    let mut affine = Affine::translate(-pivot.x, -pivot.y);
    if let Some(scale) = scale {
        affine = affine.then(Affine::scale(scale.x, scale.y));
    }
    if let Some(degrees) = rotate {
        affine = affine.then(Affine::rotate(degrees));
    }
    affine = affine.then(Affine::translate(pivot.x, pivot.y));
    if let Some(translate) = translate {
        affine = affine.then(Affine::translate(translate.x, translate.y));
    }
    Some(affine)
}

/// Scene-node identity resolved for one inline span.
struct SpanAnchor {
    /// Scene node key.
    key: String,
    /// Span byte range in concatenated shaping input; subtract its start for node-relative offsets.
    range: core::ops::Range<usize>,
    /// Processed local byte ranges -> authored local byte ranges. Present only when deterministic
    /// text processing changed bytes (currently preserved-tab expansion or an ellipsized prefix).
    projection: Option<Vec<TextSourceSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextSourceSegment {
    processed: core::ops::Range<usize>,
    source: core::ops::Range<usize>,
}

/// Reason source addressing is unavailable. Report it as unsupported while retaining visual output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterReject {
    /// Upstream could not align the glyph run to its cluster sequence.
    Unaligned,
    /// Source range count differs from glyph count.
    Arity,
    /// Cluster falls outside the owning run or span.
    OutOfRun,
    /// Byte offset exceeds the u32 side-table representation.
    Overflow,
    /// Source text cannot be attributed to a Scene node.
    Unattributed,
    /// CSS text transformation or whitespace processing prevents verbatim source offsets.
    NotVerbatim,
}

impl ClusterReject {
    fn reason(self) -> &'static str {
        match self {
            ClusterReject::Unaligned => "glyph source address: upstream cluster alignment failed",
            ClusterReject::Arity => "glyph source address: cluster and glyph counts differ",
            ClusterReject::OutOfRun => "glyph source address: cluster exceeds the run byte range",
            ClusterReject::Overflow => "glyph source address: byte offset exceeds u32",
            ClusterReject::Unattributed => "glyph source address: Scene node is unidentified",
            ClusterReject::NotVerbatim => {
                "glyph source address: CSS processing changed the source text"
            }
        }
    }
}

/// Collect render contexts and their paths within the subtree.
fn context_paths<'n>(
    node: &'n RenderNode,
    base: &[usize],
    out: &mut Vec<(&'n takumi_core::context::RenderContext, Vec<usize>)>,
) {
    out.push((&node.context, base.to_vec()));
    if let Some(children) = &node.children {
        for (at, child) in children.iter().enumerate() {
            let mut path = base.to_vec();
            path.push(at);
            context_paths(child, &path, out);
        }
    }
}

/// Validate cluster ranges against shaping input and owner span, then convert to node-relative
/// offsets.
fn checked_cluster_ranges(
    cluster_ranges: &[core::ops::Range<usize>],
    glyph_count: usize,
    run_range: &core::ops::Range<usize>,
    span_range: &core::ops::Range<usize>,
) -> Result<Vec<Span>, ClusterReject> {
    if cluster_ranges.is_empty() {
        return Err(ClusterReject::Unaligned);
    }
    if cluster_ranges.len() != glyph_count {
        return Err(ClusterReject::Arity);
    }
    // Check each cluster, not containment of the entire run range: multiple spans can share one
    // shaping-input range. Every addressed byte must belong to both the input and the owning node.
    let lo = run_range.start.max(span_range.start);
    let hi = run_range.end.min(span_range.end);
    if lo >= hi {
        return Err(ClusterReject::OutOfRun);
    }
    cluster_ranges
        .iter()
        .map(|r| {
            if r.start > r.end || r.start < lo || r.end > hi {
                return Err(ClusterReject::OutOfRun);
            }
            let rebased = (r.start - span_range.start, r.end - span_range.start);
            let (start, end) = (u32::try_from(rebased.0), u32::try_from(rebased.1));
            match (start, end) {
                (Ok(start), Ok(end)) => Ok(Span { start, end }),
                _ => Err(ClusterReject::Overflow),
            }
        })
        .collect()
}

fn tab_spaces(tab_size: &impl ToCss) -> usize {
    tab_size
        .to_css_string()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value.round().clamp(0.0, 512.0) as usize)
        .unwrap_or(8)
}

/// Map Takumi's deterministic preprocessing back to authored bytes. Identity text keeps the old
/// zero-allocation path. Ellipsis truncation is a verbatim prefix; collapsed CSS whitespace and
/// preserved tabs carry an explicit byte projection back to the authored source.
fn text_source_projection(
    source: &str,
    processed: &str,
    collapse: WhiteSpaceCollapse,
    transform: TextTransform,
    tab_spaces: usize,
) -> Result<Option<Vec<TextSourceSegment>>, ClusterReject> {
    if source == processed {
        return Ok(None);
    }
    if transform != TextTransform::None {
        return Err(ClusterReject::NotVerbatim);
    }
    if source.starts_with(processed) {
        return Ok(None);
    }
    if collapse == WhiteSpaceCollapse::Collapse {
        return collapsed_text_projection(source, processed).map(Some);
    }
    if collapse != WhiteSpaceCollapse::Preserve {
        return Err(ClusterReject::NotVerbatim);
    }

    let mut expanded = String::new();
    let mut projection = Vec::new();
    for (source_start, ch) in source.char_indices() {
        if expanded.len() >= processed.len() {
            break;
        }
        let source_end = source_start + ch.len_utf8();
        let processed_start = expanded.len();
        if ch == '\t' {
            expanded.extend(std::iter::repeat_n(' ', tab_spaces));
        } else {
            expanded.push(ch);
        }
        let processed_end = expanded.len().min(processed.len());
        if processed_start < processed_end {
            projection.push(TextSourceSegment {
                processed: processed_start..processed_end,
                source: source_start..source_end,
            });
        }
    }
    if !expanded.starts_with(processed) {
        return Err(ClusterReject::NotVerbatim);
    }
    Ok(Some(projection))
}

fn is_css_collapsible_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}')
}

/// Align a CSS-collapsed string with the authored string. A generated single space owns the whole
/// authored whitespace run, so a glyph cluster remains attributable even when `"a  b"` became
/// `"a b"`. Leading/trailing whitespace may disappear at a line boundary; non-whitespace bytes
/// must still match verbatim.
fn collapsed_text_projection(
    source: &str,
    processed: &str,
) -> Result<Vec<TextSourceSegment>, ClusterReject> {
    let source_chars: Vec<(usize, char)> = source.char_indices().collect();
    let processed_chars: Vec<(usize, char)> = processed.char_indices().collect();
    let mut source_at = 0usize;
    let mut projection = Vec::with_capacity(processed_chars.len());

    for (processed_at, &(processed_start, processed_ch)) in processed_chars.iter().enumerate() {
        let processed_end = processed_chars
            .get(processed_at + 1)
            .map(|(at, _)| *at)
            .unwrap_or(processed.len());
        if is_css_collapsible_whitespace(processed_ch) {
            let source_start = source_chars
                .get(source_at)
                .filter(|(_, ch)| is_css_collapsible_whitespace(*ch))
                .map(|(at, _)| *at)
                .ok_or(ClusterReject::NotVerbatim)?;
            while source_chars
                .get(source_at)
                .is_some_and(|(_, ch)| is_css_collapsible_whitespace(*ch))
            {
                source_at += 1;
            }
            let source_end = source_chars
                .get(source_at)
                .map(|(at, _)| *at)
                .unwrap_or(source.len());
            projection.push(TextSourceSegment {
                processed: processed_start..processed_end,
                source: source_start..source_end,
            });
            continue;
        }

        // CSS may trim whitespace at the start of an inline/line. It must not silently remove an
        // internal separator: after any processed character, a missing whitespace glyph is a
        // mapping failure rather than a guessed address.
        if processed_at == 0 {
            while source_chars
                .get(source_at)
                .is_some_and(|(_, ch)| is_css_collapsible_whitespace(*ch))
            {
                source_at += 1;
            }
        }
        let (source_start, source_ch) = source_chars
            .get(source_at)
            .copied()
            .ok_or(ClusterReject::NotVerbatim)?;
        if source_ch != processed_ch {
            return Err(ClusterReject::NotVerbatim);
        }
        source_at += 1;
        let source_end = source_chars
            .get(source_at)
            .map(|(at, _)| *at)
            .unwrap_or(source.len());
        projection.push(TextSourceSegment {
            processed: processed_start..processed_end,
            source: source_start..source_end,
        });
    }
    Ok(projection)
}

fn project_cluster_ranges(
    ranges: &[Span],
    projection: &[TextSourceSegment],
) -> Result<Vec<Span>, ClusterReject> {
    ranges
        .iter()
        .map(|range| {
            let start = range.start as usize;
            let end = range.end as usize;
            let mut covered = projection
                .iter()
                .filter(|segment| segment.processed.start < end && start < segment.processed.end);
            let first = covered.next().ok_or(ClusterReject::NotVerbatim)?;
            let (mut source_start, mut source_end) = (first.source.start, first.source.end);
            for segment in covered {
                source_start = source_start.min(segment.source.start);
                source_end = source_end.max(segment.source.end);
            }
            Ok(Span {
                start: u32::try_from(source_start).map_err(|_| ClusterReject::Overflow)?,
                end: u32::try_from(source_end).map_err(|_| ClusterReject::Overflow)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ClusterReject, PAPER_GRAIN_MAX_FIBERS, PAPER_GRAIN_MAX_SPECKS, checked_cluster_ranges,
        default_font_naming, paper_grain_counts, project_cluster_ranges, text_source_projection,
    };
    use takumi_core::style::{TextTransform, WhiteSpaceCollapse};
    use valle_draw::draw::Span;

    fn span(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    #[test]
    fn default_font_family_carries_the_exact_font_content_digest() {
        let bytes = b"font bytes";
        let face = default_font_naming(bytes, 3, 24.0);
        assert_eq!(
            face.family,
            format!(
                "valle-face-{}-3",
                crate::ContentDigest::of_bytes(bytes).as_hex()
            )
        );
    }

    #[test]
    fn cjk_clusters_pass_through_as_node_local_byte_spans() {
        // A standalone text span uses identical concatenated and node-relative offsets.
        assert_eq!(
            checked_cluster_ranges(&[0..3, 3..6], 2, &(0..6), &(0..6)),
            Ok(vec![span(0, 3), span(3, 6)])
        );
    }

    #[test]
    fn a_sibling_span_is_rebased_onto_its_own_node() {
        // Sibling text shares one shaping-input range, but the second node's first character must
        // still begin at its own byte zero.
        assert_eq!(
            checked_cluster_ranges(&[6..9, 9..12, 12..15], 3, &(0..15), &(6..15)),
            Ok(vec![span(0, 3), span(3, 6), span(6, 9)])
        );
    }

    #[test]
    fn sibling_runs_sharing_one_shaping_range_each_rebase_onto_their_own_node() {
        // Anonymous inline blocks may contain several spans sharing one shaping-input range.
        for (span_range, cluster) in [(0..1, 0..1), (1..2, 1..2), (2..3, 2..3)] {
            assert_eq!(
                checked_cluster_ranges(core::slice::from_ref(&cluster), 1, &(0..3), &span_range),
                Ok(vec![span(0, 1)]),
                "span {span_range:?} bytes must map to its own 0..1 range"
            );
        }
    }

    #[test]
    fn a_cluster_leaking_into_the_neighbouring_node_is_still_rejected() {
        // Reject clusters extending into a neighboring span despite a shared shaping range.
        assert_eq!(
            checked_cluster_ranges(core::slice::from_ref(&(0..2)), 1, &(0..3), &(1..2)),
            Err(ClusterReject::OutOfRun)
        );
    }

    #[test]
    fn ligature_clusters_may_be_wider_than_one_char() {
        // One ligature glyph may cover multiple source characters.
        assert_eq!(
            checked_cluster_ranges(&[0..2, 2..3], 2, &(0..3), &(0..3)),
            Ok(vec![span(0, 2), span(2, 3)])
        );
    }

    #[test]
    fn a_window_locked_onto_the_wrong_occurrence_is_rejected() {
        // Repeated glyph IDs must not select source offsets from an earlier differently styled
        // span.
        assert_eq!(
            checked_cluster_ranges(core::slice::from_ref(&(0..1)), 1, &(1..2), &(0..2)),
            Err(ClusterReject::OutOfRun)
        );
    }

    #[test]
    fn a_run_reaching_outside_its_own_span_is_rejected() {
        // Reject source ranges assigned to the wrong node.
        assert_eq!(
            checked_cluster_ranges(core::slice::from_ref(&(3..6)), 1, &(3..6), &(6..15)),
            Err(ClusterReject::OutOfRun)
        );
    }

    #[test]
    fn upstream_unknown_and_arity_drift_are_both_rejected() {
        assert_eq!(
            checked_cluster_ranges(&[], 2, &(0..6), &(0..6)),
            Err(ClusterReject::Unaligned)
        );
        assert_eq!(
            checked_cluster_ranges(core::slice::from_ref(&(0..3)), 2, &(0..6), &(0..6)),
            Err(ClusterReject::Arity)
        );
    }

    #[test]
    fn every_reject_reason_is_distinct_so_the_bill_is_readable() {
        let reasons = [
            ClusterReject::Unaligned.reason(),
            ClusterReject::Arity.reason(),
            ClusterReject::OutOfRun.reason(),
            ClusterReject::Overflow.reason(),
            ClusterReject::Unattributed.reason(),
            ClusterReject::NotVerbatim.reason(),
        ];
        for (i, a) in reasons.iter().enumerate() {
            assert!(
                reasons[i + 1..].iter().all(|b| a != b),
                "duplicate reason: {a}"
            );
        }
    }

    #[test]
    fn preserved_tabs_project_generated_spaces_back_to_the_authored_tab() {
        let projection = text_source_projection(
            "a\tb",
            "a    b",
            WhiteSpaceCollapse::Preserve,
            TextTransform::None,
            4,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            project_cluster_ranges(&[span(1, 5), span(5, 6)], &projection),
            Ok(vec![span(1, 2), span(2, 3)])
        );
    }

    #[test]
    fn collapsed_spaces_project_back_to_the_full_authored_runs() {
        let projection = text_source_projection(
            "7 experiments  ·  23",
            "7 experiments · 23",
            WhiteSpaceCollapse::Collapse,
            TextTransform::None,
            8,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            project_cluster_ranges(&[span(13, 14), span(14, 16), span(16, 17)], &projection),
            Ok(vec![span(13, 15), span(15, 17), span(17, 19)])
        );
    }

    #[test]
    fn collapsed_leading_and_trailing_space_keeps_visible_offsets() {
        let projection = text_source_projection(
            "  alpha  ",
            "alpha",
            WhiteSpaceCollapse::Collapse,
            TextTransform::None,
            8,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            project_cluster_ranges(&[span(0, 1), span(4, 5)], &projection),
            Ok(vec![span(2, 3), span(6, 7)])
        );
    }

    #[test]
    fn an_ellipsized_verbatim_prefix_keeps_authored_offsets() {
        let projection = text_source_projection(
            "abcdefgh",
            "abc",
            WhiteSpaceCollapse::Collapse,
            TextTransform::None,
            8,
        )
        .unwrap();
        assert!(projection.is_none());
        assert_eq!(
            checked_cluster_ranges(&[0..1, 2..3], 2, &(0..3), &(0..3)),
            Ok(vec![span(0, 1), span(2, 3)])
        );
    }

    #[test]
    fn paper_grain_shares_the_remaining_display_instance_budget() {
        let area = 4_000_000.0;
        let (specks, fibers) = paper_grain_counts(area, 1.0, usize::MAX);
        assert_eq!(specks, PAPER_GRAIN_MAX_SPECKS);
        assert_eq!(fibers, PAPER_GRAIN_MAX_FIBERS);

        let (specks, fibers) = paper_grain_counts(area, 1.0, 1_000);
        assert_eq!(specks + fibers, 1_000);
        assert!(specks > 0 && fibers > 0);

        assert_eq!(paper_grain_counts(area, 1.0, 0), (0, 0));
        let (capped_specks, capped_fibers) = paper_grain_counts(area, 1.0, 99_000);
        assert_eq!(
            capped_specks + capped_fibers,
            PAPER_GRAIN_MAX_SPECKS + PAPER_GRAIN_MAX_FIBERS
        );

        let remaining_after_reserved =
            valle_draw::program::recording::MAX_BATCH_INSTANCES_PER_RECORDING
                .saturating_sub(99_000);
        let (specks, fibers) = paper_grain_counts(area, 1.0, remaining_after_reserved);
        assert_eq!(specks + fibers, remaining_after_reserved);
    }
}
