//! First preparation template: fixed layout/paint structure with typed background-color slots.
//! Admission is conservative and load-time. No previous frame is needed to evaluate a slot.
//! Other dynamic styles, resource producers, and instance columns keep their existing lowering
//! until they have explicit bindings and range contracts.

use super::{LayoutCache, LayoutError, PreparedScene, StyleCache, Viewport, reuse::GeometryKey};
use crate::{EvalInputs, ExprId, MotionContext, MotionValue, ResolvedProps, StyleValue};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    key: TemplateKey,
    values: Vec<MotionValue>,
    program: Arc<DrawProgram>,
    boxes: Arc<BTreeMap<String, [f32; 4]>>,
    bindings: Option<Bindings>,
    paint_frame: Option<PaintFrame>,
}

struct Bindings {
    template: Arc<PaintTemplate>,
    // Exact paint interning is part of today's canonical program. Do not reuse a topology if
    // changing a value would merge two previously distinct paints or split a shared paint.
    static_colors: BTreeSet<[u32; 4]>,
    slots: Vec<(PaintId, usize)>,
}

fn color_key(c: LinearColor) -> [u32; 4] {
    // Paint equality considers positive and negative zero equal.
    [c.red, c.green, c.blue, c.alpha].map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}

impl Bindings {
    fn new(
        report: &crate::EmitReport,
        sources: &BTreeMap<String, usize>,
        values: &[MotionValue],
    ) -> Option<Self> {
        let colors = colors(values)?;
        let mut paints = BTreeMap::new();
        let mut consumers = BTreeSet::new();
        for (key, slot) in sources {
            let (node, paint) = *report.background_paints.get(key)?;
            if report.program.paints()[paint.raw() as usize] != Paint::Solid(colors[*slot]) {
                // Includes opacity folding, overridden declarations, and non-typed paints.
                return None;
            }
            if let Some(previous) = paints.insert(paint, *slot)
                && previous != *slot
            {
                return None;
            }
            consumers.insert(node);
        }
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
            if reads.iter().any(|p| paints.contains_key(p))
                && !consumers.contains(&valle_draw::program::NodeId::from_raw(index as u32))
            {
                return None;
            }
        }
        let static_colors = report
            .program
            .paints()
            .iter()
            .enumerate()
            .filter_map(|(index, paint)| {
                if paints.contains_key(&PaintId::from_raw(index as u32)) {
                    return None;
                }
                if let Paint::Solid(c) = paint {
                    Some(color_key(*c))
                } else {
                    None
                }
            })
            .collect();
        let slots: Vec<_> = paints.into_iter().collect();
        let bindings = slots
            .iter()
            .map(|(paint, slot)| PaintParameter {
                paint: *paint,
                slot: *slot,
            })
            .collect();
        Some(Self {
            template: Arc::new(
                PaintTemplate::new(report.program.clone(), bindings, values.len()).ok()?,
            ),
            static_colors,
            slots,
        })
    }

    fn bind(&self, values: &[MotionValue]) -> Option<PaintFrame> {
        let colors = colors(values)?;
        let mut distinct = BTreeMap::new();
        for (paint, slot) in &self.slots {
            let key = color_key(colors[*slot]);
            if self.static_colors.contains(&key) {
                return None;
            }
            if let Some(previous) = distinct.insert(key, *paint)
                && previous != *paint
            {
                return None;
            }
        }
        self.template.bind_frame(colors).ok()
    }
}

fn colors(values: &[MotionValue]) -> Option<Vec<LinearColor>> {
    values
        .iter()
        .map(|value| {
            let MotionValue::Color(color) = value else {
                return None;
            };
            // The existing emitter omits backgrounds whose CSS alpha probe rounds to zero. That
            // changes structure, so select the full lowering at this boundary; retain HDR RGB.
            (color.is_finite() && color.to_srgb8().a != 0).then(|| AuthorColor::to_working(*color))
        })
        .collect()
}

/// Worker-local, bounded to one immutable scene/font registry and one resident variant. Eviction
/// only changes preparation cost. A cold evaluation always selects the same canonical program.
pub struct PaintProgramCache {
    prepared: PreparedScene,
    layout: LayoutCache,
    plan: crate::eval::EvalPlan,
    order: Vec<ExprId>,
    roots: Vec<ExprId>,
    sources: BTreeMap<String, usize>,
    nodes: BTreeSet<String>,
    snapshot: Option<Snapshot>,
}

impl PaintProgramCache {
    pub fn new(prepared: &PreparedScene, fonts: &crate::Fonts) -> Option<Self> {
        let layout = LayoutCache::new(prepared, fonts)?;
        let artifact = prepared.artifact();
        let plan = crate::eval::EvalPlan::new(&artifact.exprs);
        let mut expressions = BTreeMap::new();
        for node in &artifact.nodes {
            let mut background = None;
            for style in &node.styles {
                if style.property == "background-color" {
                    background = match style.value {
                        StyleValue::Expr { expr } => Some(expr),
                        _ => None,
                    };
                } else if let StyleValue::Expr { expr } = style.value
                    && plan.sample_dependent(expr)
                {
                    return None;
                }
            }
            if let Some(expr) = background {
                // Until cascade origins are recorded, do not infer that a matching color means
                // this declaration won against a shorthand or an important utility.
                if !node.class_names.is_empty()
                    || !node.class_conditions.is_empty()
                    || node
                        .styles
                        .iter()
                        .any(|s| matches!(s.property.as_str(), "background" | "all"))
                {
                    return None;
                }
                expressions.insert(node.key.clone(), expr);
            }
        }
        let roots: Vec<_> = expressions
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let sources: BTreeMap<_, _> = expressions
            .into_iter()
            .map(|(key, expr)| (key, roots.binary_search(&expr).unwrap()))
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
            nodes,
            snapshot: None,
        })
    }

    pub fn is_for(&self, prepared: &PreparedScene) -> bool {
        self.prepared.same_instance(prepared)
    }

    pub fn clear(&mut self) {
        self.snapshot = None;
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
            parameter_slots: self.roots.len(),
            ..Default::default()
        };
        if let Some(snapshot) = &self.snapshot
            && snapshot.key == key
            && (snapshot.bindings.is_some() || self.roots.is_empty())
        {
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
            let program = if snapshot.values.len() == values.len()
                && snapshot
                    .values
                    .iter()
                    .zip(&values)
                    .all(|(a, b)| a.bitwise_eq(b))
            {
                Some((snapshot.program.clone(), snapshot.paint_frame.clone()))
            } else {
                snapshot
                    .bindings
                    .as_ref()
                    .and_then(|b| b.bind(&values))
                    .map(|frame| (frame.program().clone(), Some(frame)))
            };
            if let Some((program, paint_frame)) = program {
                stats.reused_template = true;
                return Ok(PreparedPaintFrame {
                    program,
                    paint_frame,
                    layout_boxes: snapshot.boxes.clone(),
                    stats,
                });
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
            .then(|| Bindings::new(&report, &self.sources, &values))
            .flatten();
        let paint_frame = bindings.as_ref().and_then(|b| b.bind(&values));
        let program = paint_frame
            .as_ref()
            .map(|frame| frame.program().clone())
            .unwrap_or_else(|| Arc::new(report.program));
        self.snapshot = cacheable.then(|| Snapshot {
            key,
            values,
            program: program.clone(),
            boxes: boxes.clone(),
            bindings,
            paint_frame: paint_frame.clone(),
        });
        stats.emitted_program = true;
        Ok(PreparedPaintFrame {
            program,
            paint_frame,
            layout_boxes: boxes,
            stats,
        })
    }
}
