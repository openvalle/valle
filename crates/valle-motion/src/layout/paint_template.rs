//! Preparation templates for typed background colors and admitted rectangular opacity folds.
//! Admission is conservative and load-time. No previous frame is needed to evaluate a slot.
//! Other dynamic styles, resource producers, and instance columns keep their existing lowering
//! until they have explicit bindings and range contracts.

use super::{LayoutCache, LayoutError, PreparedScene, StyleCache, Viewport, reuse::GeometryKey};
use crate::{EvalInputs, ExprId, MotionContext, MotionValue, ResolvedProps, StyleValue};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use valle_draw::{
    Rect,
    program::{
        AuthorColor, DrawProgram, LinearColor, Node, Paint, PaintFrame, PaintId, PaintParameter,
        PaintTemplate, ProgramResourceCatalog,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum PaintPreparationError {
    #[error("{0}")]
    Layout(#[from] LayoutError),
    #[error("{0}")]
    Emit(#[from] crate::EmitError),
    #[error("unsupported paint: {0:?}")]
    Unsupported(Vec<(String, &'static str)>),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PaintPreparationStats {
    pub evaluated_expressions: usize,
    pub parameter_slots: usize,
    pub reused_template: bool,
    pub emitted_program: bool,
    pub resident_variants: usize,
    pub checked_variants: usize,
}

pub struct PreparedPaintFrame {
    pub program: Arc<DrawProgram>,
    pub paint_frame: Option<PaintFrame>,
    pub layout_boxes: Arc<BTreeMap<String, [f32; 4]>>,
    pub stats: PaintPreparationStats,
}

#[derive(Clone, PartialEq)]
struct TemplateKey {
    geometry: GeometryKey,
    clip: Option<[u64; 4]>,
    density: (u64, usize),
    compact: bool,
    clock: valle_timeline::ClipClock,
}

struct Snapshot {
    values: Vec<MotionValue>,
    program: Arc<DrawProgram>,
    boxes: Arc<BTreeMap<String, [f32; 4]>>,
    bindings: Bindings,
    paint_frame: Option<PaintFrame>,
    sample_clocks: super::temporal::SampleClockChecks,
}

// Both caps affect residency only. Oversized or evicted variants still produce the same
// parameter template, but will be rebuilt on the next request. The byte cap accounts encoded
// template storage, not allocator RSS or the host's surface/flight budget.
const MAX_VARIANTS: usize = 16;
const MAX_TEMPLATE_BYTES: usize = 8 * 1024 * 1024;

struct Bindings {
    template: Arc<PaintTemplate>,
    omitted: Vec<bool>,
    // Static lookup is built once; hot guards never scan unrelated nodes or paint arenas.
    static_paints: BTreeSet<PaintKey>,
    unit_opacity: Vec<bool>,
    groups: Vec<PaintGroup>,
}

struct PaintGroup {
    paint: PaintId,
    slots: Vec<usize>,
    // A paint read by a static consumer cannot change, even when dynamic consumers share it.
    pinned: Option<(PaintKey, [u32; 4])>,
}

fn color_bits(c: LinearColor) -> [u32; 4] {
    [c.red, c.green, c.blue, c.alpha].map(f32::to_bits)
}

fn color_key(c: LinearColor) -> [u32; 4] {
    // Paint interning considers positive and negative zero equal; parameter bytes do not.
    [c.red, c.green, c.blue, c.alpha].map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PaintKey {
    color: [u32; 4],
    opacity: u32,
}

impl PaintKey {
    fn new(color: LinearColor, opacity: f32) -> Self {
        Self {
            color: color_key(color),
            opacity: opacity.to_bits(),
        }
    }
}

#[derive(Clone)]
enum InputBinding {
    Root(usize),
    Static(MotionValue),
}
impl InputBinding {
    fn value<'a>(&'a self, values: &'a [MotionValue]) -> &'a MotionValue {
        match self {
            Self::Root(slot) => &values[*slot],
            Self::Static(value) => value,
        }
    }
}

struct PaintSource {
    color: InputBinding,
    opacity: InputBinding,
}

struct ColorInputs {
    colors: Vec<LinearColor>,
    originals: Vec<LinearColor>,
    keys: Vec<PaintKey>,
    omitted: Vec<bool>,
    unit_opacity: Vec<bool>,
}

impl ColorInputs {
    fn new(values: &[MotionValue], sources: &[PaintSource]) -> Option<Self> {
        let mut inputs = Self {
            colors: Vec::new(),
            originals: Vec::new(),
            keys: Vec::new(),
            omitted: Vec::new(),
            unit_opacity: Vec::new(),
        };
        for source in sources {
            let color = match source.color.value(values) {
                MotionValue::Color(color) => *color,
                MotionValue::Str(color) => AuthorColor::parse(color)?,
                _ => return None,
            };
            let MotionValue::Number(opacity) = source.opacity.value(values) else {
                return None;
            };
            if !color.is_finite() || !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                return None;
            }
            // The style bridge and canonical opacity rewrite both operate at f32 precision.
            let opacity = *opacity as f32;
            let working = AuthorColor::to_working(color);
            let scaled = if opacity == 1.0 {
                working
            } else {
                LinearColor::new(
                    working.red * opacity,
                    working.green * opacity,
                    working.blue * opacity,
                    working.alpha * opacity,
                )
            };
            if !working.is_finite() || !scaled.is_finite() {
                return None;
            }
            inputs.colors.push(scaled);
            inputs.originals.push(working);
            inputs.keys.push(PaintKey::new(working, opacity));
            inputs
                .omitted
                .push(color.to_srgb8().a == 0 || opacity == 0.0);
            inputs.unit_opacity.push(opacity == 1.0);
        }
        Some(inputs)
    }
}

impl Bindings {
    fn new(
        report: &crate::EmitReport,
        sources: &BTreeMap<String, usize>,
        inputs: &ColorInputs,
    ) -> Option<Self> {
        let mut paints = BTreeMap::<PaintId, Vec<(valle_draw::program::NodeId, usize)>>::new();
        let mut consumers = BTreeSet::new();
        for (key, slot) in sources {
            let consumer = *report.background_paints.get(key)?;
            if inputs.omitted[*slot] {
                if consumer.is_some() {
                    return None;
                }
                continue;
            }
            let (node, paint) = consumer?;
            let Paint::Solid(color) = report.program.paints()[paint.raw() as usize] else {
                return None;
            };
            let origin = report.paint_origins.get(paint.raw() as usize)?;
            let Paint::Solid(original) = origin.source else {
                return None;
            };
            if PaintKey::new(original, origin.opacity) != inputs.keys[*slot]
                || color_bits(original) != color_bits(inputs.originals[*slot])
                || color_bits(color) != color_bits(inputs.colors[*slot])
            {
                // Rejects unbound group opacity, cascade overrides, and signed-zero ambiguity in
                // shared paints. Never infer declaration provenance from numeric equality.
                return None;
            }
            paints.entry(paint).or_default().push((node, *slot));
            consumers.insert((node, paint));
        }
        let mut pinned = BTreeSet::new();
        for (index, node) in report.program.nodes().iter().enumerate() {
            let reads: Vec<_> = match node {
                Node::Path(path) => path
                    .fill
                    .into_iter()
                    .chain(path.stroke.iter().map(|s| s.paint))
                    .collect(),
                Node::GlyphRun(run) => std::iter::once(run.paint)
                    .chain(run.stroke.iter().map(|s| s.paint))
                    .collect(),
                _ => Vec::new(),
            };
            for paint in reads {
                if paints.contains_key(&paint)
                    && !consumers
                        .contains(&(valle_draw::program::NodeId::from_raw(index as u32), paint))
                {
                    pinned.insert(paint);
                }
            }
        }
        let static_paints = report
            .paint_origins
            .iter()
            .enumerate()
            .filter_map(|(index, origin)| {
                let id = PaintId::from_raw(index as u32);
                if paints.contains_key(&id) && !pinned.contains(&id) {
                    return None;
                }
                if let Paint::Solid(c) = origin.source {
                    Some(PaintKey::new(c, origin.opacity))
                } else {
                    None
                }
            })
            .collect();
        let groups: Vec<_> = paints
            .into_iter()
            .map(|(paint, mut consumers)| {
                // Canonical node order follows paint use order for the admitted recording class.
                consumers.sort_unstable();
                let slots = consumers.into_iter().map(|(_, slot)| slot).collect();
                let pinned = pinned.contains(&paint).then(|| {
                    let Paint::Solid(color) = report.program.paints()[paint.raw() as usize] else {
                        unreachable!()
                    };
                    let origin = &report.paint_origins[paint.raw() as usize];
                    let Paint::Solid(original) = origin.source else {
                        unreachable!()
                    };
                    (PaintKey::new(original, origin.opacity), color_bits(color))
                });
                PaintGroup {
                    paint,
                    slots,
                    pinned,
                }
            })
            .collect();
        let bindings = groups
            .iter()
            .filter(|g| g.pinned.is_none())
            .map(|g| PaintParameter {
                paint: g.paint,
                slot: g.slots[0],
            })
            .collect();
        Some(Self {
            template: Arc::new(
                PaintTemplate::new(report.program.clone(), bindings, inputs.colors.len()).ok()?,
            ),
            omitted: inputs.omitted.clone(),
            unit_opacity: inputs.unit_opacity.clone(),
            static_paints,
            groups,
        })
    }

    fn accepts(&self, inputs: &ColorInputs) -> bool {
        if self.omitted != inputs.omitted || self.unit_opacity != inputs.unit_opacity {
            return false;
        }
        let mut distinct = BTreeMap::new();
        for group in &self.groups {
            let color = inputs.colors[group.slots[0]];
            let bits = color_bits(color);
            let key = inputs.keys[group.slots[0]];
            if group
                .slots
                .iter()
                .any(|slot| color_bits(inputs.colors[*slot]) != bits || inputs.keys[*slot] != key)
            {
                return false;
            }
            if let Some(pinned) = group.pinned {
                if pinned != (key, bits) {
                    return false;
                }
                continue;
            }
            if self.static_paints.contains(&key) || distinct.insert(key, group.paint).is_some() {
                return false;
            }
        }
        true
    }

    fn bind(&self, inputs: &ColorInputs) -> Option<PaintFrame> {
        self.accepts(inputs)
            .then(|| self.template.bind_frame(inputs.colors.clone()).ok())
            .flatten()
    }
}

/// Worker-local variants for one immutable scene/font registry and one geometry/host key.
/// Guards inspect only current values. LRU residency changes cost, never canonical output.
pub struct PaintProgramCache {
    prepared: PreparedScene,
    layout: LayoutCache,
    plan: crate::eval::EvalPlan,
    order: Vec<ExprId>,
    roots: Vec<ExprId>,
    sources: BTreeMap<String, usize>,
    inputs: Vec<PaintSource>,
    nodes: BTreeSet<String>,
    key: Option<TemplateKey>,
    variants: VecDeque<Snapshot>,
    template_bytes: usize,
}

impl PaintProgramCache {
    pub fn new(prepared: &PreparedScene, fonts: &crate::Fonts) -> Option<Self> {
        if !prepared.temporal_samples_invariant() {
            return None;
        }
        let layout = LayoutCache::new(prepared, fonts)?;
        let artifact = prepared.artifact();
        let plan = crate::eval::EvalPlan::new(&artifact.exprs);
        // A sampled background has multiple consumers/weights, not one current-frame slot.
        // Keep the proven-static sample subtrees literal; props/viewport/host changes invalidate
        // the whole key. Unrelated animated branches can still bind ordinary paint parameters.
        let mut sampled = BTreeSet::new();
        let mut pending: Vec<_> = prepared
            .dependencies()
            .temporal_nodes()
            .iter()
            .map(|n| n.node)
            .collect();
        while let Some(id) = pending.pop() {
            if !sampled.insert(id) {
                continue;
            }
            let node = &artifact.nodes[id.0 as usize];
            pending.extend(
                &artifact.node_children[node.children.start as usize..node.children.end as usize],
            );
        }
        let mut expressions = BTreeMap::new();
        for (index, node) in artifact.nodes.iter().enumerate() {
            if sampled.contains(&crate::NodeId(index as u32)) {
                continue;
            }
            let binding = |property: &str| {
                node.styles
                    .iter()
                    .rev()
                    .find(|s| s.property == property)
                    .map(|s| s.value.clone())
            };
            let color = binding("background-color");
            let opacity = binding("opacity");
            let dynamic_opacity =
                matches!(&opacity, Some(StyleValue::Expr { expr }) if plan.sample_dependent(*expr));
            for style in &node.styles {
                if !matches!(style.property.as_str(), "background-color" | "opacity")
                    && let StyleValue::Expr { expr } = style.value
                    && plan.sample_dependent(expr)
                {
                    return None;
                }
            }
            if dynamic_opacity
                && (!matches!(node.kind, crate::NodeKind::Box)
                    || node.children.start != node.children.end
                    || node.styles.iter().any(|s| {
                        !matches!(
                            s.property.as_str(),
                            "opacity"
                                | "background-color"
                                | "width"
                                | "height"
                                | "min-width"
                                | "min-height"
                                | "max-width"
                                | "max-height"
                                | "position"
                                | "left"
                                | "right"
                                | "top"
                                | "bottom"
                                | "margin"
                                | "margin-left"
                                | "margin-right"
                                | "margin-top"
                                | "margin-bottom"
                                | "translate"
                                | "rotate"
                                | "scale"
                                | "transform"
                                | "transform-origin"
                                | "z-index"
                        )
                    }))
            {
                return None;
            }
            if dynamic_opacity || matches!(color, Some(StyleValue::Expr { .. })) {
                // Cascade origins and inherited/dynamic groups need their own explicit bindings.
                if !node.class_names.is_empty()
                    || !node.class_conditions.is_empty()
                    || node
                        .styles
                        .iter()
                        .any(|s| matches!(s.property.as_str(), "background" | "all"))
                {
                    return None;
                }
                expressions.insert(
                    node.key.clone(),
                    (
                        color.unwrap_or(StyleValue::Static {
                            value: MotionValue::Color(AuthorColor::from_srgb8(
                                valle_draw::Rgba::new(0, 0, 0, 0),
                            )),
                        }),
                        opacity.unwrap_or(StyleValue::Static {
                            value: MotionValue::Number(1.0),
                        }),
                    ),
                );
            }
        }
        let roots: Vec<_> = expressions
            .values()
            .flat_map(|(color, opacity)| [color, opacity])
            .filter_map(|v| match v {
                StyleValue::Expr { expr } => Some(*expr),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let input = |value| match value {
            StyleValue::Expr { expr } => InputBinding::Root(roots.binary_search(&expr).unwrap()),
            StyleValue::Static { value } => InputBinding::Static(value),
        };
        let mut inputs = Vec::new();
        let sources: BTreeMap<_, _> = expressions
            .into_iter()
            .map(|(key, (color, opacity))| {
                let slot = inputs.len();
                inputs.push(PaintSource {
                    color: input(color),
                    opacity: input(opacity),
                });
                (key, slot)
            })
            .collect();
        let nodes = sources.keys().cloned().collect();
        let order = plan.schedule(roots.iter().copied());
        Some(Self {
            prepared: prepared.clone(),
            layout,
            plan,
            order,
            roots,
            sources,
            inputs,
            nodes,
            key: None,
            variants: VecDeque::new(),
            template_bytes: 0,
        })
    }

    pub fn is_for(&self, prepared: &PreparedScene) -> bool {
        self.prepared.same_instance(prepared)
    }

    pub fn clear(&mut self) {
        self.key = None;
        self.variants.clear();
        self.template_bytes = 0;
        self.layout.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        ctx: &MotionContext,
        props: &ResolvedProps,
        viewport: Viewport,
        styles: Option<&StyleCache>,
        faces: Option<&crate::FaceCache>,
        catalog: &dyn ProgramResourceCatalog,
        clip: Option<Rect>,
        density: crate::emit::RasterDensity,
    ) -> Result<PreparedPaintFrame, PaintPreparationError> {
        let key = TemplateKey {
            geometry: GeometryKey::new(viewport, props, ctx),
            clip: clip.map(|r| [r.x, r.y, r.width, r.height].map(f64::to_bits)),
            density: (density.scale.to_bits(), density.layer_limit),
            compact: valle_draw::program::instance_compaction_enabled(),
            clock: ctx.clock,
        };
        let mut stats = PaintPreparationStats {
            parameter_slots: self.inputs.len(),
            ..Default::default()
        };
        if self.key.as_ref() != Some(&key) {
            self.variants.clear();
            self.template_bytes = 0;
            self.key = Some(key);
        }
        if !self.variants.is_empty() {
            let evaluated = crate::eval::eval_roots_planned(
                self.prepared.artifact(),
                &self.plan,
                EvalInputs {
                    ctx,
                    props,
                    unit: None,
                    viewport: viewport.size.width.zip(viewport.size.height).map(|(w, h)| {
                        (
                            f64::from(w) / f64::from(viewport.device_pixel_ratio),
                            f64::from(h) / f64::from(viewport.device_pixel_ratio),
                        )
                    }),
                },
                &self.order,
            )
            .map_err(LayoutError::Eval)?;
            let values: Vec<_> = self.roots.iter().map(|id| evaluated[id].clone()).collect();
            stats.evaluated_expressions = self.order.len();
            if let Some(inputs) = ColorInputs::new(&values, &self.inputs) {
                let found = self.variants.iter().rposition(|snapshot| {
                    stats.checked_variants += 1;
                    snapshot.bindings.accepts(&inputs)
                });
                if let Some(index) = found {
                    self.variants[index].sample_clocks.validate(ctx)?;
                    let snapshot = self.variants.remove(index).expect("resident variant");
                    let (program, paint_frame) = if snapshot
                        .values
                        .iter()
                        .zip(&values)
                        .all(|(a, b)| a.bitwise_eq(b))
                    {
                        (snapshot.program.clone(), snapshot.paint_frame.clone())
                    } else {
                        let frame = snapshot
                            .bindings
                            .template
                            .bind_frame(inputs.colors)
                            .expect("accepted color inputs");
                        (frame.program().clone(), Some(frame))
                    };
                    let boxes = snapshot.boxes.clone();
                    self.variants.push_back(snapshot);
                    stats.reused_template = true;
                    stats.resident_variants = self.variants.len();
                    return Ok(PreparedPaintFrame {
                        program,
                        paint_frame,
                        layout_boxes: boxes,
                        stats,
                    });
                }
            }
        }
        // The full activation pass determines which expressions are live on a cold frame. Do
        // not eagerly evaluate dormant color expressions just to discover a possible template.
        let tree = self.layout.build_tree(ctx, props, viewport, styles)?;
        stats.evaluated_expressions += self
            .prepared
            .frame_evaluation_stats(ctx)
            .evaluated_expressions;
        let values: Vec<_> = self
            .roots
            .iter()
            .map(|id| tree.values[id.0 as usize].clone())
            .collect();
        let boxes = Arc::new(super::layout_boxes(self.prepared.artifact(), &tree)?);
        let report = crate::emit::emit_with_paint_parameters(
            &tree,
            &crate::default_font_naming,
            faces,
            catalog,
            clip,
            density,
            &self.nodes,
        )?;
        if !report.unsupported.is_empty() {
            return Err(PaintPreparationError::Unsupported(report.unsupported));
        }
        // Resource extent/content and implicit shader/video time need explicit identity slots.
        // Until then even an equal color buffer must not reuse a resource-producing program.
        let requirements = report.program.requirements();
        let cacheable = requirements.external_textures.is_empty()
            && requirements.frames.is_empty()
            && requirements.runtime_shaders.is_empty()
            && requirements.scene3d.is_empty();
        let bindings = cacheable
            .then(|| {
                Bindings::new(
                    &report,
                    &self.sources,
                    &ColorInputs::new(&values, &self.inputs)?,
                )
            })
            .flatten();
        let paint_frame = bindings
            .as_ref()
            .and_then(|b| b.bind(&ColorInputs::new(&values, &self.inputs)?));
        let program = paint_frame
            .as_ref()
            .map(|frame| frame.program().clone())
            .unwrap_or_else(|| Arc::new(report.program));
        if let Some(bindings) = bindings {
            // Packing happens only on construction and is shared with later transport/digests.
            let bytes = bindings.template.packed().map_or(usize::MAX, <[u8]>::len);
            if bytes <= MAX_TEMPLATE_BYTES {
                while self.variants.len() >= MAX_VARIANTS
                    || self.template_bytes + bytes > MAX_TEMPLATE_BYTES
                {
                    let old = self
                        .variants
                        .pop_front()
                        .expect("nonempty cache over quota");
                    self.template_bytes -= old
                        .bindings
                        .template
                        .packed()
                        .expect("resident encoding")
                        .len();
                }
                self.template_bytes += bytes;
                self.variants.push_back(Snapshot {
                    sample_clocks: super::temporal::SampleClockChecks::new(&self.prepared, &tree),
                    values,
                    program: program.clone(),
                    boxes: boxes.clone(),
                    bindings,
                    paint_frame: paint_frame.clone(),
                });
            }
        }
        stats.resident_variants = self.variants.len();
        stats.emitted_program = true;
        Ok(PreparedPaintFrame {
            program,
            paint_frame,
            layout_boxes: boxes,
            stats,
        })
    }
}
