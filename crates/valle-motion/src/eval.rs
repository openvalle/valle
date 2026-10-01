//! Per-frame evaluator for the Motion JSX Scene contract.
//!
//! The evaluator consumes only the closed [`MotionContext`], resolved props, and the expression
//! arena carried by [`SceneArtifact`]. It has no clock, JavaScript runtime, layout engine, or
//! renderer dependency. Expressions reference earlier entries only, so a single forward pass is
//! deterministic and evaluates shared subexpressions exactly once.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use valle_draw::{Point, Rect, Vec2, program::GradientInterpolation};

use crate::context::MotionContext;
use crate::geometry::{GeometryError, PathData};
pub use crate::value::css_token;
use crate::value::{Angle, AngleUnit, Length, Length2, MotionEasing, MotionValue};

use super::{
    CompareOp, ContextInput, ControlType, ControlsSchema, Expr, ExprId, Extrapolation,
    GeometryField, InstanceGroup, InterpolateStop, MAX_TEMPLATE_OUTPUT_BYTES, RangeShape,
    SceneArtifact, TemplatePart,
};

/// Caller-supplied prop values after schema/default validation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedProps(BTreeMap<String, MotionValue>);

impl ResolvedProps {
    pub fn get(&self, name: &str) -> Option<&MotionValue> {
        self.0.get(name)
    }

    pub fn as_map(&self) -> &BTreeMap<String, MotionValue> {
        &self.0
    }
}

/// Inputs visible to one expression-arena evaluation.
#[derive(Debug, Clone, Copy)]
pub struct EvalInputs<'a> {
    pub ctx: &'a MotionContext,
    pub props: &'a ResolvedProps,
    /// Current text unit; unit fields are undefined outside per-unit evaluation.
    pub unit: Option<UnitContext>,
    /// Optional viewport dimensions from the render request, separate from the time context.
    /// Reading unavailable viewport inputs is an error rather than a zero-valued fallback.
    pub viewport: Option<(f64, f64)>,
}

/// Text-unit identity with byte offsets relative to its own node text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitContext {
    pub index: u32,
    pub count: u32,
    pub start: u32,
    pub end: u32,
}

/// Fail-closed errors from prop resolution or expression evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    UnknownProp {
        name: String,
    },
    MissingProp {
        name: String,
    },
    BadGeometry {
        at: usize,
        reason: GeometryError,
    },
    InvalidProp {
        name: String,
    },
    BadExpr {
        at: usize,
        referenced: usize,
    },
    TypeMismatch {
        at: usize,
        op: &'static str,
    },
    DivisionByZero {
        at: usize,
    },
    BadInterpolate {
        at: usize,
    },
    TemplateTooLong {
        at: usize,
    },
    BuiltinDomain {
        at: usize,
        op: &'static str,
    },
    NonFinite {
        at: usize,
        op: &'static str,
    },
    /// Unit context accessed outside per-unit evaluation; reject instead of substituting zero.
    UnitOutsidePerUnit {
        at: usize,
    },
    InstanceOutsideGroup {
        at: usize,
    },
    /// Layout bounds accessed before the post-layout pass; reject instead of substituting a zero
    /// rectangle.
    BoundsOutsidePostLayout {
        at: usize,
    },
    Project3DOutsidePostLayout {
        at: usize,
    },
    /// Viewport input requested without host-supplied dimensions.
    ViewportUnavailable {
        at: usize,
    },
    /// Bounds target key is absent from the scene.
    MissingBoundsTarget {
        key: String,
    },
    MissingProject3DTarget {
        scene_key: String,
        anchor_key: String,
    },
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EvalError::UnknownProp { name } => write!(f, "unknown prop `{name}`"),
            EvalError::MissingProp { name } => write!(f, "required prop `{name}` is missing"),
            EvalError::InvalidProp { name } => {
                write!(f, "prop `{name}` does not match its control schema")
            }
            EvalError::BadGeometry { at, reason } => {
                write!(f, "expression {at}: invalid geometry: {reason}")
            }
            EvalError::BadExpr { at, referenced } => {
                write!(
                    f,
                    "expression {at} references unavailable expression {referenced}"
                )
            }
            EvalError::TypeMismatch { at, op } => {
                write!(f, "expression {at}: type mismatch in {op}")
            }
            EvalError::DivisionByZero { at } => write!(f, "expression {at}: division by zero"),
            EvalError::BadInterpolate { at } => {
                write!(f, "expression {at}: invalid interpolate")
            }
            EvalError::TemplateTooLong { at } => {
                write!(
                    f,
                    "expression {at}: template output exceeds its byte budget"
                )
            }
            EvalError::BuiltinDomain { at, op } => {
                write!(f, "expression {at}: invalid input for {op}")
            }
            EvalError::NonFinite { at, op } => {
                write!(f, "expression {at}: {op} produced a non-finite value")
            }
            EvalError::UnitOutsidePerUnit { at } => write!(
                f,
                "expression {at}: ctx.unit.* is only defined inside a Text per-unit style"
            ),
            EvalError::InstanceOutsideGroup { at } => write!(
                f,
                "expression {at}: instance fields are only defined during instance evaluation"
            ),
            EvalError::ViewportUnavailable { at } => write!(
                f,
                "expression {at}: ctx.viewport.* needs a render viewport, but none was supplied \
                 to the evaluator"
            ),
            EvalError::BoundsOutsidePostLayout { at } => write!(
                f,
                "expression {at}: bounds(...) is only defined after layout"
            ),
            EvalError::Project3DOutsidePostLayout { at } => write!(
                f,
                "expression {at}: project3d(...) is only defined after Scene3D projection"
            ),
            EvalError::MissingBoundsTarget { key } => {
                write!(f, "bounds target `{key}` is not a node in this scene")
            }
            EvalError::MissingProject3DTarget {
                scene_key,
                anchor_key,
            } => write!(
                f,
                "project3d target `{scene_key}` / `{anchor_key}` is not projected in this frame"
            ),
        }
    }
}

impl std::error::Error for EvalError {}

/// Merge caller overrides with control defaults and validate the resulting typed values.
pub fn resolve_props(
    controls: &ControlsSchema,
    overrides: &BTreeMap<String, MotionValue>,
) -> Result<ResolvedProps, EvalError> {
    for name in overrides.keys() {
        if !controls.props.contains_key(name) {
            return Err(EvalError::UnknownProp { name: name.clone() });
        }
    }

    let mut values = BTreeMap::new();
    for (name, schema) in &controls.props {
        let value = overrides.get(name).or(schema.default.as_ref());
        let Some(value) = value else {
            if schema.required {
                return Err(EvalError::MissingProp { name: name.clone() });
            }
            continue;
        };
        let inside_bounds = match (&schema.control, value) {
            (ControlType::Number { min, max, .. }, MotionValue::Number(value)) => {
                min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max)
            }
            _ => true,
        };
        if !value.is_finite() || !schema.control.accepts(value) || !inside_bounds {
            return Err(EvalError::InvalidProp { name: name.clone() });
        }
        values.insert(name.clone(), value.clone());
    }
    Ok(ResolvedProps(values))
}

/// Immutable dependency analysis for one expression arena. Build this once when admitting a
/// scene, then reuse it for every frame and every sampling time. The arena is topologically
/// ordered, so each scheduled dependency is evaluated before its consumer.
#[derive(Debug, Clone)]
pub struct EvalPlan {
    children: Vec<Vec<ExprId>>,
    sample_dependent: Vec<bool>,
    unit_dependent: Vec<bool>,
    bounds_dependent: Vec<bool>,
    projection_dependent: Vec<bool>,
    unit_order: Vec<usize>,
    bounds_order: Vec<usize>,
    post_layout_order: Vec<usize>,
}

/// Prepared evaluation order for one instance template. Row-independent values are computed
/// once per sampled time; only the listed row-dependent slots run for each selected instance.
#[derive(Debug, Clone)]
pub(crate) struct InstanceEvalPlan {
    row_dependent: Vec<bool>,
    row_order: Vec<usize>,
    sample_dependent: Vec<bool>,
    live: Vec<bool>,
    shared_count: usize,
}

impl InstanceEvalPlan {
    pub(crate) fn new(group: &InstanceGroup) -> Self {
        Self::build(group, false)
    }

    /// The renderer consumes circle center/radius and solid fill, so expression evaluation need
    /// not construct a PathData per row before the emitter creates the exact authored arc.
    pub(crate) fn for_render(group: &InstanceGroup) -> Self {
        Self::build(group, true)
    }

    fn build(group: &InstanceGroup, consumed_only: bool) -> Self {
        let eval_plan = EvalPlan::new(&group.exprs);
        let live_from_roots = || {
            fn collect(node: &crate::InstanceTemplateNode, roots: &mut Vec<ExprId>) {
                roots.extend(
                    node.node
                        .expr_refs_outside_per_unit()
                        .into_iter()
                        .map(|(_, id)| id),
                );
                for child in &node.children {
                    collect(child, roots);
                }
            }
            let mut roots: Vec<_> = group
                .template
                .expr_refs_outside_per_unit()
                .into_iter()
                .map(|(_, id)| id)
                .collect();
            for child in &group.template_children {
                collect(child, &mut roots);
            }
            let mut live = vec![false; group.exprs.len()];
            for id in eval_plan.schedule(roots) {
                live[id.0 as usize] = true;
            }
            live
        };
        let live =
            if consumed_only {
                if let Some((center, radius)) = group.circle_template_parameters() {
                    let mut roots = vec![center, radius];
                    if let crate::NodeKind::Path {
                        fill:
                            Some(crate::PaintValue::Solid {
                                color: crate::ColorValue::Expr { expr },
                            }),
                        ..
                    } = &group.template.kind
                    {
                        roots.push(*expr);
                    }
                    if let Some(visibility) = group.template.visibility {
                        roots.push(visibility);
                    }
                    roots.extend(group.template.styles.iter().filter_map(
                        |style| match style.value {
                            crate::StyleValue::Expr { expr } => Some(expr),
                            crate::StyleValue::Static { .. } => None,
                        },
                    ));
                    let mut live = vec![false; group.exprs.len()];
                    for id in eval_plan.schedule(roots) {
                        live[id.0 as usize] = true;
                    }
                    live
                } else {
                    live_from_roots()
                }
            } else {
                // The public eval_instances API returns every expression, including
                // values not consumed by a node. Only rendering uses a sparse slice.
                vec![true; group.exprs.len()]
            };
        let mut row_dependent = Vec::with_capacity(group.exprs.len());
        let mut row_order = Vec::new();
        let mut shared_count = 0;
        for (at, expr) in group.exprs.iter().enumerate() {
            let dependent = matches!(expr, Expr::InstanceField { .. } | Expr::InstanceIndex)
                || expr
                    .children()
                    .iter()
                    .any(|id| row_dependent[id.0 as usize]);
            row_dependent.push(dependent);
            if dependent && live[at] {
                row_order.push(at);
            } else if live[at] {
                shared_count += 1;
            }
        }
        Self {
            row_dependent,
            row_order,
            sample_dependent: eval_plan.sample_dependent,
            live,
            shared_count,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.row_dependent.len()
    }

    pub(crate) fn work_for_rows(&self, rows: usize) -> usize {
        if rows == 0 {
            0
        } else {
            self.shared_count + self.row_order.len() * rows
        }
    }
}

/// Static expression values shared only by sampling rounds in one render request. The caller
/// owns this table and must not reuse it with different props, duration, frame rate, or viewport.
pub(crate) struct SampleValueCache {
    slots: Vec<Option<MotionValue>>,
    computed: usize,
    reused: usize,
}

impl SampleValueCache {
    pub(crate) fn new(plan: &EvalPlan) -> Self {
        Self::for_len(plan.children.len())
    }

    pub(crate) fn for_len(len: usize) -> Self {
        Self {
            slots: vec![None; len],
            computed: 0,
            reused: 0,
        }
    }

    fn get(&mut self, at: usize) -> Option<MotionValue> {
        let value = self.slots.get(at)?.as_ref()?.clone();
        self.reused += 1;
        Some(value)
    }

    fn insert(&mut self, at: usize, value: &MotionValue) {
        self.slots[at] = Some(value.clone());
        self.computed += 1;
    }

    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.computed, self.reused)
    }
}

impl EvalPlan {
    pub fn new(exprs: &[Expr]) -> Self {
        let mut plan = Self {
            children: Vec::with_capacity(exprs.len()),
            sample_dependent: vec![false; exprs.len()],
            unit_dependent: vec![false; exprs.len()],
            bounds_dependent: vec![false; exprs.len()],
            projection_dependent: vec![false; exprs.len()],
            unit_order: Vec::new(),
            bounds_order: Vec::new(),
            post_layout_order: Vec::new(),
        };
        for (at, expr) in exprs.iter().enumerate() {
            let children = expr.children();
            let inherited = |flags: &[bool]| {
                children
                    .iter()
                    .any(|id| flags.get(id.0 as usize).copied().unwrap_or(false))
            };
            plan.sample_dependent[at] = matches!(
                expr,
                Expr::Context {
                    input: ContextInput::LocalFrame
                        | ContextInput::LocalProgress
                        | ContextInput::CompositionSeconds
                        | ContextInput::HostSeconds
                        | ContextInput::HostProgress
                        | ContextInput::UnitIndex
                        | ContextInput::UnitCount
                        | ContextInput::UnitStart
                        | ContextInput::UnitEnd
                } | Expr::InstanceField { .. }
                    | Expr::InstanceIndex
                    | Expr::InstanceCount
                    | Expr::NodeBounds { .. }
                    | Expr::Project3D { .. }
                    | Expr::RangeSelector { .. }
            ) || inherited(&plan.sample_dependent);
            plan.unit_dependent[at] = (matches!(expr, Expr::Context { input } if input.is_unit())
                || matches!(expr, Expr::RangeSelector { .. }))
                || inherited(&plan.unit_dependent);
            plan.bounds_dependent[at] =
                matches!(expr, Expr::NodeBounds { .. }) || inherited(&plan.bounds_dependent);
            plan.projection_dependent[at] =
                matches!(expr, Expr::Project3D { .. }) || inherited(&plan.projection_dependent);
            if plan.unit_dependent[at] {
                plan.unit_order.push(at);
            }
            if plan.bounds_dependent[at] && !plan.projection_dependent[at] {
                plan.bounds_order.push(at);
            }
            if plan.bounds_dependent[at] || plan.projection_dependent[at] {
                plan.post_layout_order.push(at);
            }
            plan.children.push(children);
        }
        plan
    }

    /// Close a set of requested expressions over their ancestors without visiting unrelated
    /// arena entries. The returned order follows the arena's topological order.
    pub fn schedule(&self, roots: impl IntoIterator<Item = ExprId>) -> Vec<ExprId> {
        let mut live = BTreeSet::new();
        let mut todo: Vec<_> = roots.into_iter().collect();
        while let Some(id) = todo.pop() {
            if live.insert(id) {
                if let Some(children) = self.children.get(id.0 as usize) {
                    todo.extend(children);
                }
            }
        }
        live.into_iter().collect()
    }

    /// Close a frame's activation roots using arena-indexed flags. The ordinary sparse API keeps
    /// its ordered-set scheduler; this path avoids tree insertion for large changing live sets.
    pub(crate) fn schedule_dense(&self, roots: impl IntoIterator<Item = ExprId>) -> Vec<ExprId> {
        let mut live = vec![false; self.children.len()];
        let mut invalid = Vec::new();
        let mut todo: Vec<_> = roots.into_iter().collect();
        while let Some(id) = todo.pop() {
            let at = id.0 as usize;
            let Some(marked) = live.get_mut(at) else {
                invalid.push(id);
                continue;
            };
            if !*marked {
                *marked = true;
                todo.extend(&self.children[at]);
            }
        }
        let mut order: Vec<_> = live
            .into_iter()
            .enumerate()
            .filter_map(|(at, live)| live.then_some(ExprId(at as u32)))
            .collect();
        // Admission rejects invalid references. Keep a malformed root in the returned order so
        // the evaluator still raises BadExpr instead of silently dropping it.
        order.extend(invalid);
        order
    }

    fn is_post_layout(&self, at: usize) -> bool {
        self.bounds_dependent[at] || self.projection_dependent[at]
    }

    pub(crate) fn bounds_dependent(&self, id: ExprId) -> bool {
        self.bounds_dependent
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn unit_dependent(&self, id: ExprId) -> bool {
        self.unit_dependent
            .get(id.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn post_layout_dependent(&self, id: ExprId) -> bool {
        self.bounds_dependent(id)
            || self
                .projection_dependent
                .get(id.0 as usize)
                .copied()
                .unwrap_or_default()
    }

    /// A value may change when the time, text unit, instance, or layout sample changes.
    pub(crate) fn sample_dependent(&self, id: ExprId) -> bool {
        self.sample_dependent
            .get(id.0 as usize)
            .copied()
            .unwrap_or(true)
    }

    pub(crate) fn base_eval_count(&self) -> usize {
        (0..self.children.len())
            .filter(|&at| !self.is_post_layout(at) && !self.unit_dependent[at])
            .count()
    }
}

/// Evaluate the complete expression arena in one forward pass.
pub fn eval_all(
    artifact: &SceneArtifact,
    inputs: EvalInputs<'_>,
) -> Result<Vec<MotionValue>, EvalError> {
    eval_all_planned(artifact, &EvalPlan::new(&artifact.exprs), inputs)
}

/// Visit prepared rows without retaining one expression table per instance. Shared expressions
/// run once; each dependent slot is overwritten in topological order before the row is visited.
/// This keeps the expression-value working set proportional to the template instead of the
/// instance count; the caller still stores the resulting batch instances.
pub(crate) fn try_for_each_instance<E>(
    group: &InstanceGroup,
    inputs: EvalInputs<'_>,
    visit: impl FnMut(usize, &[MotionValue]) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<EvalError>,
{
    try_for_each_instance_rows(group, inputs, 0..group.rows(), visit)
}

pub(crate) fn try_for_each_instance_rows<E>(
    group: &InstanceGroup,
    inputs: EvalInputs<'_>,
    rows: Range<usize>,
    visit: impl FnMut(usize, &[MotionValue]) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<EvalError>,
{
    let plan = InstanceEvalPlan::new(group);
    try_for_each_instance_rows_planned(group, &plan, inputs, rows, None, visit)
}

pub(crate) fn try_for_each_instance_rows_planned<E>(
    group: &InstanceGroup,
    plan: &InstanceEvalPlan,
    inputs: EvalInputs<'_>,
    rows: Range<usize>,
    mut shared: Option<&mut SampleValueCache>,
    mut visit: impl FnMut(usize, &[MotionValue]) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<EvalError>,
{
    debug_assert!(rows.end <= group.rows());
    debug_assert_eq!(plan.len(), group.exprs.len());
    let mut values = Vec::with_capacity(group.exprs.len());
    for (at, expr) in group.exprs.iter().enumerate() {
        if !plan.live[at] || plan.row_dependent[at] {
            values.push(MotionValue::Number(0.0));
        } else {
            if !plan.sample_dependent[at]
                && let Some(value) = shared.as_deref_mut().and_then(|cache| cache.get(at))
            {
                values.push(value);
                continue;
            }
            let value = eval_one_with(
                at,
                expr,
                |id| values.get(id.0 as usize),
                inputs,
                None,
                None,
                Some((group, 0)),
            )
            .map_err(E::from)?;
            let value = finite(at, "instance shared evaluation", value).map_err(E::from)?;
            if !plan.sample_dependent[at]
                && let Some(cache) = shared.as_deref_mut()
            {
                cache.insert(at, &value);
            }
            values.push(value);
        }
    }
    for row in rows {
        for &at in &plan.row_order {
            let expr = &group.exprs[at];
            let value = eval_one_with(
                at,
                expr,
                |id| values.get(id.0 as usize),
                inputs,
                None,
                None,
                Some((group, row)),
            )
            .map_err(E::from)?;
            values[at] = finite(at, "instance evaluation", value).map_err(E::from)?;
        }
        visit(row, &values)?;
    }
    Ok(())
}

/// Evaluate one template arena over every prepared instance row. Expressions independent of
/// instance fields run once for the frame; dependent expressions run in table order. Each call
/// owns its value tables, so random-access and repeated frames cannot inherit previous state.
pub fn eval_instances(
    group: &InstanceGroup,
    inputs: EvalInputs<'_>,
) -> Result<Vec<Vec<MotionValue>>, EvalError> {
    let mut rows = Vec::with_capacity(group.rows());
    try_for_each_instance(group, inputs, |_, values| {
        rows.push(values.to_vec());
        Ok(())
    })?;
    Ok(rows)
}

pub(crate) fn eval_all_planned(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    inputs: EvalInputs<'_>,
) -> Result<Vec<MotionValue>, EvalError> {
    eval_all_planned_cached(artifact, plan, inputs, None)
}

pub(crate) fn eval_all_planned_cached(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    inputs: EvalInputs<'_>,
    mut shared: Option<&mut SampleValueCache>,
) -> Result<Vec<MotionValue>, EvalError> {
    let mut values = Vec::with_capacity(artifact.exprs.len());
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if plan.is_post_layout(at) {
            // Skip post-layout expressions in the base pass; admission prevents layout consumers
            // from reading their placeholders.
            values.push(MotionValue::Number(0.0));
            continue;
        }
        if plan.unit_dependent[at] && inputs.unit.is_none() {
            // Skip unit-dependent expressions until a unit exists; admission prevents other
            // consumers from reading their placeholders.
            values.push(MotionValue::Number(0.0));
            continue;
        }
        if !plan.sample_dependent[at]
            && let Some(value) = shared.as_deref_mut().and_then(|cache| cache.get(at))
        {
            values.push(value);
            continue;
        }
        let value = eval_one(at, expr, &values, inputs, None, None)?;
        let value = finite(at, "evaluation", value)?;
        if !plan.sample_dependent[at]
            && let Some(cache) = shared.as_deref_mut()
        {
            cache.insert(at, &value);
        }
        values.push(value);
    }
    Ok(values)
}

/// Evaluate only the listed expression ids plus their ancestors. Missing slots stay `None`.
pub fn eval_slice(
    artifact: &SceneArtifact,
    inputs: EvalInputs<'_>,
    live: &BTreeSet<ExprId>,
) -> Result<Vec<Option<MotionValue>>, EvalError> {
    let plan = EvalPlan::new(&artifact.exprs);
    let order = plan.schedule(live.iter().copied());
    let sparse = eval_roots_planned(artifact, &plan, inputs, &order)?;
    let mut values = vec![None; artifact.exprs.len()];
    for (id, value) in sparse {
        values[id.0 as usize] = Some(value);
    }
    Ok(values)
}

/// Evaluate a precomputed dependency slice into a sparse value table. Each call owns its values,
/// so separate subframe samples cannot observe stale values from another evaluation round.
pub(crate) fn eval_roots_planned(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    inputs: EvalInputs<'_>,
    order: &[ExprId],
) -> Result<BTreeMap<ExprId, MotionValue>, EvalError> {
    let mut values = BTreeMap::new();
    eval_roots_planned_into(artifact, plan, inputs, order, &mut values, None)?;
    Ok(values)
}

/// Evaluate a closed, topologically ordered slice into the arena's dense slots. The activation
/// path needs a full value table for layout even when most expressions remain placeholders;
/// using the arena indices avoids a tree lookup and insertion for every selected expression.
pub(crate) fn eval_roots_planned_dense_cached(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    inputs: EvalInputs<'_>,
    order: &[ExprId],
    mut shared: Option<&mut SampleValueCache>,
) -> Result<Vec<MotionValue>, EvalError> {
    let mut values = vec![MotionValue::Number(0.0); artifact.exprs.len()];
    for id in order {
        let at = id.0 as usize;
        let expr = artifact
            .exprs
            .get(at)
            .ok_or(EvalError::BadExpr { at, referenced: at })?;
        if plan.is_post_layout(at) || (plan.unit_dependent[at] && inputs.unit.is_none()) {
            continue;
        }
        if !plan.sample_dependent[at]
            && let Some(value) = shared.as_deref_mut().and_then(|cache| cache.get(at))
        {
            values[at] = value;
            continue;
        }
        let value = finite(
            at,
            "evaluation",
            eval_one(at, expr, &values, inputs, None, None)?,
        )?;
        if !plan.sample_dependent[at]
            && let Some(cache) = shared.as_deref_mut()
        {
            cache.insert(at, &value);
        }
        values[at] = value;
    }
    Ok(values)
}

/// Fill one sample's sparse table incrementally. `shared` belongs to a single request whose
/// props, duration, frame rate, and viewport stay fixed while sample time changes. Static values
/// are reused across times; sample-dependent values live only in `values` for the current time.
pub(crate) fn eval_roots_planned_into(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    inputs: EvalInputs<'_>,
    order: &[ExprId],
    values: &mut BTreeMap<ExprId, MotionValue>,
    mut shared: Option<&mut BTreeMap<ExprId, MotionValue>>,
) -> Result<usize, EvalError> {
    let mut evaluated = 0;
    for id in order {
        if values.contains_key(id) {
            continue;
        }
        let at = id.0 as usize;
        let expr = artifact
            .exprs
            .get(at)
            .ok_or(EvalError::BadExpr { at, referenced: at })?;
        if !plan.sample_dependent(*id)
            && let Some(cached) = shared.as_ref().and_then(|shared| shared.get(id))
        {
            values.insert(*id, cached.clone());
            continue;
        }
        let value = if plan.is_post_layout(at) || (plan.unit_dependent[at] && inputs.unit.is_none())
        {
            MotionValue::Number(0.0)
        } else {
            finite(
                at,
                "evaluation",
                eval_one_with(
                    at,
                    expr,
                    |child| values.get(&child),
                    inputs,
                    None,
                    None,
                    None,
                )?,
            )?
        };
        if !plan.sample_dependent(*id)
            && let Some(shared) = shared.as_deref_mut()
        {
            shared.insert(*id, value.clone());
        }
        values.insert(*id, value);
        evaluated += 1;
    }
    Ok(evaluated)
}

/// Recompute only unit-dependent expressions for one text unit, reusing base values. Supply
/// post-layout boxes because unit expressions may also depend on bounds.
pub fn eval_units(
    artifact: &SceneArtifact,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    unit: UnitContext,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
) -> Result<Vec<MotionValue>, EvalError> {
    eval_units_planned(
        artifact,
        &EvalPlan::new(&artifact.exprs),
        base,
        inputs,
        unit,
        boxes,
        projected,
    )
}

pub(crate) fn eval_units_planned(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    unit: UnitContext,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
) -> Result<Vec<MotionValue>, EvalError> {
    let inputs = EvalInputs {
        unit: Some(unit),
        ..inputs
    };
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for &at in &plan.unit_order {
        let expr = &artifact.exprs[at];
        let value = eval_one(at, expr, &values, inputs, Some(boxes), Some(projected))?;
        values[at] = finite(at, "evaluation", value)?;
    }
    Ok(values)
}

/// After layout, recompute bounds-dependent expressions using Scene-keyed boxes and reuse other
/// base values. Bounds access before this pass is invalid.
pub fn eval_layout_bounds(
    artifact: &SceneArtifact,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
) -> Result<Vec<MotionValue>, EvalError> {
    eval_layout_bounds_planned(
        artifact,
        &EvalPlan::new(&artifact.exprs),
        base,
        inputs,
        boxes,
    )
}

pub(crate) fn eval_layout_bounds_planned(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
) -> Result<Vec<MotionValue>, EvalError> {
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for &at in &plan.bounds_order {
        let expr = &artifact.exprs[at];
        let value = eval_one(at, expr, &values, inputs, Some(boxes), None)?;
        values[at] = finite(at, "evaluation", value)?;
    }
    Ok(values)
}

/// Complete the final post-layout pass after Scene3D anchor projection is available.
pub fn eval_post_layout(
    artifact: &SceneArtifact,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
) -> Result<Vec<MotionValue>, EvalError> {
    eval_post_layout_planned(
        artifact,
        &EvalPlan::new(&artifact.exprs),
        base,
        inputs,
        boxes,
        projected,
    )
}

pub(crate) fn eval_post_layout_planned(
    artifact: &SceneArtifact,
    plan: &EvalPlan,
    base: &[MotionValue],
    inputs: EvalInputs<'_>,
    boxes: &BTreeMap<String, valle_draw::Rect>,
    projected: &BTreeMap<(String, String), valle_draw::Point>,
) -> Result<Vec<MotionValue>, EvalError> {
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for &at in &plan.post_layout_order {
        let expr = &artifact.exprs[at];
        let value = eval_one(at, expr, &values, inputs, Some(boxes), Some(projected))?;
        values[at] = finite(at, "evaluation", value)?;
    }
    Ok(values)
}

/// Detect runtime inputs anywhere in an expression subtree before attempting constant folding. Use
/// the same exhaustive input classification as folding, including operations such as spring that
/// read frame rate directly. Skip unfoldable subtrees to avoid repeated whole-arena work.
pub fn subtree_reads_runtime_inputs(exprs: &[Expr], id: crate::ExprId) -> bool {
    subtree_reads_runtime_inputs_impl(exprs, id)
}

/// Single-node runtime-dependency classification for incremental arena flags; combine it with
/// already-known child flags.
pub fn expr_reads_runtime_inputs(expr: &Expr) -> bool {
    reads_runtime_inputs(expr)
}

fn subtree_reads_runtime_inputs_impl(exprs: &[Expr], id: crate::ExprId) -> bool {
    // Traverse the topologically ordered DAG with an explicit stack to avoid recursion limits.
    let mut stack = vec![id];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(at) = stack.pop() {
        if !seen.insert(at.0) {
            continue;
        }
        let Some(expr) = exprs.get(at.0 as usize) else {
            continue;
        };
        if reads_runtime_inputs(expr) {
            return true;
        }
        stack.extend(expr.children());
    }
    false
}

fn reads_runtime_inputs(expr: &Expr) -> bool {
    match expr {
        // Runtime input leaves.
        Expr::InstanceField { .. }
        | Expr::InstanceIndex
        | Expr::InstanceCount
        | Expr::Context { .. }
        | Expr::Prop { .. }
        | Expr::NodeBounds { .. }
        | Expr::Project3D { .. } => true,
        // Spring reads frame rate directly from EvalInputs.
        Expr::Spring { .. } => true,
        // Pure operations depend only on child values.
        Expr::Const { .. }
        | Expr::MakePoint { .. }
        | Expr::MakeRect { .. }
        | Expr::PathLine { .. }
        | Expr::PathTemplate { .. }
        | Expr::PathCubic { .. }
        | Expr::PathArc { .. }
        | Expr::PathArea { .. }
        | Expr::PathSector { .. }
        | Expr::PathAreaBand { .. }
        | Expr::PathOffset { .. }
        | Expr::PathResample { .. }
        | Expr::PathReverse { .. }
        | Expr::PathRoundCorners { .. }
        | Expr::PathZigzag { .. }
        | Expr::PathNoiseDisplace { .. }
        | Expr::PathPuckerBloat { .. }
        | Expr::PathTwist { .. }
        | Expr::PathSimplify { .. }
        | Expr::PathStrokeToPath { .. }
        | Expr::PathMorph { .. }
        | Expr::PathCompatibleMorph { .. }
        | Expr::PathCompatibleMorphSequence { .. }
        | Expr::PathPointAt { .. }
        | Expr::PathTangentAt { .. }
        | Expr::PathAngleAt { .. }
        | Expr::PathLength { .. }
        | Expr::PathTrajectory { .. }
        | Expr::GeometryField { .. }
        | Expr::ToLength2 { .. }
        | Expr::Add { .. }
        | Expr::Sub { .. }
        | Expr::Mul { .. }
        | Expr::Div { .. }
        | Expr::Neg { .. }
        | Expr::Compare { .. }
        | Expr::Select { .. }
        | Expr::Template { .. }
        | Expr::MathUnary { .. }
        | Expr::MathBinary { .. }
        | Expr::Noise1D { .. }
        | Expr::Noise2D { .. }
        | Expr::FormatNumber { .. }
        | Expr::Interpolate { .. }
        | Expr::SimulationSample { .. } => false,
        Expr::AudioSample { .. } => false,
        Expr::RangeSelector { .. } => true,
    }
}

/// Dummy folding context required by the evaluator signature; runtime-dependent branches are never
/// evaluated during folding.
fn folding_context() -> MotionContext {
    MotionContext {
        host: crate::MotionHostContext {
            sample: valle_timeline::internal::SampleTime::ZERO,
            duration: valle_timeline::RationalTime::ZERO,
            progress: valle_timeline::RationalTime::ZERO,
        },
        local_frame: 0,
        sample: valle_timeline::internal::SampleTime::ZERO,
        progress: 0.0,
        duration_frames: 0,
        fps: valle_timeline::FrameRate::new(1, 1).expect("valid rate"),
    }
}

pub fn fold_constants(
    exprs: &[Expr],
    controls: &crate::ControlsSchema,
) -> Vec<Option<MotionValue>> {
    let mut folded: Vec<Option<MotionValue>> = Vec::with_capacity(exprs.len());
    // Maintain a dense evaluated prefix, using folded flags to distinguish usable values from
    // placeholders.
    let mut dense: Vec<MotionValue> = Vec::with_capacity(exprs.len());
    let empty_ctx = folding_context();
    let empty_props = crate::ResolvedProps::default();
    // Ask artifact admission before folding so replacing an expression with a constant cannot
    // bypass its type or value restrictions.
    //
    let admitted = crate::expr::admitted_flags(exprs, controls);
    for (at, expr) in exprs.iter().enumerate() {
        let runtime_only = reads_runtime_inputs(expr);
        let children_ready = expr
            .children()
            .into_iter()
            .all(|id| folded.get(id.0 as usize).is_some_and(Option::is_some));
        let value = if runtime_only || !children_ready || !admitted[at] {
            None
        } else {
            eval_one(
                at,
                expr,
                &dense,
                EvalInputs {
                    ctx: &empty_ctx,
                    props: &empty_props,
                    unit: None,
                    // Viewport dimensions are unavailable during compile-time folding.
                    viewport: None,
                },
                None,
                None,
            )
            .ok()
        };
        dense.push(value.clone().unwrap_or(MotionValue::Number(0.0)));
        folded.push(value);
    }
    folded
}

fn eval_one(
    at: usize,
    expr: &Expr,
    done: &[MotionValue],
    inputs: EvalInputs<'_>,
    bounds: Option<&BTreeMap<String, valle_draw::Rect>>,
    projected: Option<&BTreeMap<(String, String), valle_draw::Point>>,
) -> Result<MotionValue, EvalError> {
    eval_one_with(
        at,
        expr,
        |id| done.get(id.0 as usize),
        inputs,
        bounds,
        projected,
        None,
    )
}

fn eval_one_with<'a>(
    at: usize,
    expr: &Expr,
    lookup: impl Fn(ExprId) -> Option<&'a MotionValue>,
    inputs: EvalInputs<'_>,
    bounds: Option<&BTreeMap<String, valle_draw::Rect>>,
    projected: Option<&BTreeMap<(String, String), valle_draw::Point>>,
    instance: Option<(&InstanceGroup, usize)>,
) -> Result<MotionValue, EvalError> {
    let child = |id: ExprId| {
        lookup(id).ok_or(EvalError::BadExpr {
            at,
            referenced: id.0 as usize,
        })
    };
    match expr {
        Expr::Const { value } => Ok(value.clone()),
        Expr::InstanceField { column, .. } => instance
            .and_then(|(group, row)| group.columns.get(*column as usize)?.values.get(row))
            .ok_or(EvalError::InstanceOutsideGroup { at }),
        Expr::InstanceIndex => instance
            .map(|(_, row)| MotionValue::Number(row as f64))
            .ok_or(EvalError::InstanceOutsideGroup { at }),
        Expr::InstanceCount => instance
            .map(|(group, _)| MotionValue::Number(group.rows() as f64))
            .ok_or(EvalError::InstanceOutsideGroup { at }),
        Expr::Context { input } => context_value(*input, inputs.ctx, inputs.unit, inputs.viewport)
            .ok_or_else(|| match input {
                ContextInput::ViewportWidth | ContextInput::ViewportHeight => {
                    EvalError::ViewportUnavailable { at }
                }
                _ => EvalError::UnitOutsidePerUnit { at },
            }),
        Expr::RangeSelector {
            start,
            end,
            offset,
            softness,
            shape,
        } => {
            let unit = inputs.unit.ok_or(EvalError::UnitOutsidePerUnit { at })?;
            let start = as_number(at, "rangeSelector", child(*start)?)?;
            let end = as_number(at, "rangeSelector", child(*end)?)?;
            let offset = as_number(at, "rangeSelector", child(*offset)?)?;
            let softness = as_number(at, "rangeSelector", child(*softness)?)?;
            let value = range_selector(unit, start, end, offset, softness, *shape).ok_or(
                EvalError::BuiltinDomain {
                    at,
                    op: "rangeSelector",
                },
            )?;
            Ok(MotionValue::Number(value))
        }
        Expr::Prop { name } => inputs
            .props
            .get(name)
            .cloned()
            .ok_or_else(|| EvalError::MissingProp { name: name.clone() }),
        Expr::NodeBounds { key } => {
            let bounds = bounds.ok_or(EvalError::BoundsOutsidePostLayout { at })?;
            bounds
                .get(key)
                .copied()
                .map(MotionValue::Rect)
                .ok_or_else(|| EvalError::MissingBoundsTarget { key: key.clone() })
        }
        Expr::Project3D {
            scene_key,
            anchor_key,
        } => {
            let projected = projected.ok_or(EvalError::Project3DOutsidePostLayout { at })?;
            projected
                .get(&(scene_key.clone(), anchor_key.clone()))
                .copied()
                .map(MotionValue::Point)
                .ok_or_else(|| EvalError::MissingProject3DTarget {
                    scene_key: scene_key.clone(),
                    anchor_key: anchor_key.clone(),
                })
        }
        Expr::MakePoint { x, y } => Ok(MotionValue::Point(Point::new(
            as_number(at, "point", child(*x)?)?,
            as_number(at, "point", child(*y)?)?,
        ))),
        Expr::MakeRect {
            x,
            y,
            width,
            height,
        } => Ok(MotionValue::Rect(Rect::new(
            as_number(at, "rect", child(*x)?)?,
            as_number(at, "rect", child(*y)?)?,
            as_number(at, "rect", child(*width)?)?,
            as_number(at, "rect", child(*height)?)?,
        ))),
        Expr::PathLine { points } => PathData::line(
            points
                .iter()
                .map(|point| as_point(at, "line", child(*point)?).copied())
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathTemplate { verbs, points } => PathData::new(
            verbs.clone(),
            points
                .iter()
                .map(|point| as_point(at, "pathTemplate", child(*point)?).copied())
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathCubic {
            from,
            control_1,
            control_2,
            to,
        } => PathData::cubic(
            *as_point(at, "cubic", child(*from)?)?,
            *as_point(at, "cubic", child(*control_1)?)?,
            *as_point(at, "cubic", child(*control_2)?)?,
            *as_point(at, "cubic", child(*to)?)?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathArc {
            center,
            radius,
            start_angle,
            end_angle,
        } => PathData::arc(
            *as_point(at, "arc", child(*center)?)?,
            as_number(at, "arc", child(*radius)?)?,
            as_number(at, "arc", child(*start_angle)?)?,
            as_number(at, "arc", child(*end_angle)?)?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathArea { path, baseline } => as_path(at, "area", child(*path)?)?
            .area(as_number(at, "area", child(*baseline)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathSector {
            center,
            inner,
            outer,
            start,
            end,
            corner_radius,
        } => PathData::sector(
            *as_point(at, "sector", child(*center)?)?,
            as_number(at, "sector", child(*inner)?)?,
            as_number(at, "sector", child(*outer)?)?,
            as_number(at, "sector", child(*start)?)?,
            as_number(at, "sector", child(*end)?)?,
            as_number(at, "sector", child(*corner_radius)?)?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathAreaBand { upper, lower } => PathData::area_band(
            as_path(at, "areaBand", child(*upper)?)?,
            as_path(at, "areaBand", child(*lower)?)?,
        )
        .map(MotionValue::PathData)
        .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathOffset { path, distance } => as_path(at, "offsetPath", child(*path)?)?
            .offset_path(as_number(at, "offsetPath", child(*distance)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathResample { path, count } => as_path(at, "resamplePath", child(*path)?)?
            .resample(usize::from(*count))
            .and_then(|value| {
                (value.points.len() <= crate::geometry::MAX_FRAME_GEOMETRY_POINTS)
                    .then_some(value)
                    .ok_or(crate::geometry::GeometryError::TooComplex)
            })
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathReverse { path } => as_path(at, "reversePath", child(*path)?)?
            .reverse()
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathRoundCorners { path, radius } => as_path(at, "roundCorners", child(*path)?)?
            .round_corners(as_number(at, "roundCorners", child(*radius)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathZigzag { path, size, ridges } => as_path(at, "zigzag", child(*path)?)?
            .zigzag(
                as_number(at, "zigzag", child(*size)?)?,
                usize::from(*ridges),
            )
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathNoiseDisplace {
            path,
            seed,
            amount,
            frequency,
            phase,
        } => as_path(at, "noiseDisplace", child(*path)?)?
            .noise_displace(
                *seed,
                as_number(at, "noiseDisplace", child(*amount)?)?,
                as_number(at, "noiseDisplace", child(*frequency)?)?,
                as_number(at, "noiseDisplace", child(*phase)?)?,
            )
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathPuckerBloat { path, amount } => as_path(at, "puckerBloat", child(*path)?)?
            .pucker_bloat(as_number(at, "puckerBloat", child(*amount)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathTwist { path, angle } => as_path(at, "twist", child(*path)?)?
            .twist(as_number(at, "twist", child(*angle)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathSimplify { path, tolerance } => as_path(at, "simplify", child(*path)?)?
            .simplify(as_number(at, "simplify", child(*tolerance)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathStrokeToPath { path, width } => as_path(at, "strokeToPath", child(*path)?)?
            .stroke_to_path(as_number(at, "strokeToPath", child(*width)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathMorph { from, to, progress } => {
            let from = as_path(at, "morphPath", child(*from)?)?;
            let to = as_path(at, "morphPath", child(*to)?)?;
            from.morph(to, as_number(at, "morphPath", child(*progress)?)?)
                .map(MotionValue::PathData)
                .map_err(|reason| EvalError::BadGeometry { at, reason })
        }
        Expr::PathCompatibleMorph { prepared, progress } => {
            let t = as_number(at, "morph", child(*progress)?)?.clamp(0.0, 1.0);
            prepared
                .outline_path(t)
                .map(MotionValue::PathData)
                .ok_or(EvalError::BadGeometry {
                    at,
                    reason: GeometryError::InvalidCompatibleMorph,
                })
        }
        Expr::PathCompatibleMorphSequence {
            segments,
            stops,
            progress,
        } => {
            let input = as_number(at, "morphSequence", child(*progress)?)?;
            let index = stops.partition_point(|stop| *stop <= input);
            let segment_index = index.saturating_sub(1).min(segments.len() - 1);
            let t = ((input - stops[segment_index])
                / (stops[segment_index + 1] - stops[segment_index]))
                .clamp(0.0, 1.0);
            segments[segment_index]
                .outline_path(t)
                .map(MotionValue::PathData)
                .ok_or(EvalError::BadGeometry {
                    at,
                    reason: GeometryError::InvalidCompatibleMorph,
                })
        }
        Expr::PathPointAt { path, progress } => as_path(at, "pointAt", child(*path)?)?
            .point_at(as_number(at, "pointAt", child(*progress)?)?)
            .map(MotionValue::Point)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathTangentAt { path, progress } => as_path(at, "tangentAt", child(*path)?)?
            .tangent_at(as_number(at, "tangentAt", child(*progress)?)?)
            .map(MotionValue::Vec2)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathAngleAt { path, progress } => as_path(at, "motionPath", child(*path)?)?
            .angle_at(as_number(at, "motionPath", child(*progress)?)?)
            .map(|value| MotionValue::Angle(crate::value::Angle::deg(value)))
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathLength { path } => as_path(at, "pathLength", child(*path)?)?
            .path_length()
            .map(MotionValue::Number)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathTrajectory { frames, frame } => {
            if frames.is_empty() {
                return Err(EvalError::BadGeometry {
                    at,
                    reason: GeometryError::BadTrajectory,
                });
            }
            let frame = as_number(at, "pathTrajectory", child(*frame)?)?
                .floor()
                .max(0.0) as usize;
            Ok(MotionValue::PathData(
                frames[frame.min(frames.len() - 1)].clone(),
            ))
        }
        Expr::GeometryField { input, field } => {
            let value = child(*input)?;
            let number = match (value, field) {
                (MotionValue::Point(value), GeometryField::X) => value.x,
                (MotionValue::Point(value), GeometryField::Y) => value.y,
                (MotionValue::Vec2(value), GeometryField::X) => value.x,
                (MotionValue::Vec2(value), GeometryField::Y) => value.y,
                (MotionValue::Rect(value), GeometryField::X) => value.x,
                (MotionValue::Rect(value), GeometryField::Y) => value.y,
                (MotionValue::Rect(value), GeometryField::Width) => value.width,
                (MotionValue::Rect(value), GeometryField::Height) => value.height,
                _ => {
                    return Err(EvalError::TypeMismatch {
                        at,
                        op: "geometry field",
                    });
                }
            };
            Ok(MotionValue::Number(number))
        }
        Expr::ToLength2 { input } => match child(*input)? {
            MotionValue::Length2(value) => Ok(MotionValue::Length2(*value)),
            MotionValue::Point(value) => Ok(MotionValue::Length2(Length2::px(value.x, value.y))),
            MotionValue::Vec2(value) => Ok(MotionValue::Length2(Length2::px(value.x, value.y))),
            _ => Err(EvalError::TypeMismatch {
                at,
                op: "length-pair conversion",
            }),
        },
        Expr::Add { lhs, rhs } => number_arith(at, "add", child(*lhs)?, child(*rhs)?, |a, b| a + b),
        Expr::Sub { lhs, rhs } => number_arith(at, "sub", child(*lhs)?, child(*rhs)?, |a, b| a - b),
        Expr::Mul { lhs, rhs } => number_arith(at, "mul", child(*lhs)?, child(*rhs)?, |a, b| a * b),
        Expr::Div { lhs, rhs } => {
            let rhs = as_number(at, "div", child(*rhs)?)?;
            if rhs == 0.0 {
                return Err(EvalError::DivisionByZero { at });
            }
            let lhs = as_number(at, "div", child(*lhs)?)?;
            Ok(MotionValue::Number(lhs / rhs))
        }
        Expr::Neg { input } => Ok(MotionValue::Number(-as_number(at, "neg", child(*input)?)?)),
        Expr::Compare { op, lhs, rhs } => compare(at, *op, child(*lhs)?, child(*rhs)?),
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            let MotionValue::Bool(condition) = child(*condition)? else {
                return Err(EvalError::TypeMismatch { at, op: "select" });
            };
            Ok(child(if *condition { *when_true } else { *when_false })?.clone())
        }
        Expr::Template { parts } => {
            let mut value = String::new();
            for part in parts {
                match part {
                    TemplatePart::Text { value: text } => value.push_str(text),
                    TemplatePart::Expr { expr } => value.push_str(&css_token(child(*expr)?)),
                }
                if value.len() > MAX_TEMPLATE_OUTPUT_BYTES {
                    return Err(EvalError::TemplateTooLong { at });
                }
            }
            Ok(MotionValue::Str(value))
        }
        Expr::MathUnary { op, input } => {
            let value = as_number(at, "math", child(*input)?)?;
            crate::builtin::math_unary(*op, value)
                .map(MotionValue::Number)
                .map_err(|_| EvalError::BuiltinDomain { at, op: "math" })
        }
        Expr::MathBinary { op, lhs, rhs } => {
            let lhs = as_number(at, "math", child(*lhs)?)?;
            let rhs = as_number(at, "math", child(*rhs)?)?;
            crate::builtin::math_binary(*op, lhs, rhs)
                .map(MotionValue::Number)
                .map_err(|_| EvalError::BuiltinDomain { at, op: "math" })
        }
        Expr::Noise1D { seed, x } => Ok(MotionValue::Number(
            crate::compute::noise::value_noise_1d(*seed, as_number(at, "noise1d", child(*x)?)?),
        )),
        Expr::Noise2D { seed, x, y } => {
            Ok(MotionValue::Number(crate::compute::noise::value_noise_2d(
                *seed,
                as_number(at, "noise2d", child(*x)?)?,
                as_number(at, "noise2d", child(*y)?)?,
            )))
        }
        Expr::FormatNumber { input, format } => {
            let value = as_number(at, "formatNumber", child(*input)?)?;
            crate::builtin::format_number(value, *format)
                .map(MotionValue::Str)
                .map_err(|_| EvalError::BuiltinDomain {
                    at,
                    op: "number formatting",
                })
        }
        // Convert elapsed frames to seconds with the actual render frame rate so equal physical
        // times share the same spring result.
        Expr::Spring {
            elapsed_frames,
            mass,
            stiffness,
            damping,
            initial_velocity,
            output,
        } => {
            let frames = as_number(at, "spring", child(*elapsed_frames)?)?;
            let fps = crate::frame_rate_as_f64(inputs.ctx.fps);
            if !fps.is_finite() || fps <= 0.0 {
                return Err(EvalError::TypeMismatch { at, op: "spring" });
            }
            let sample = crate::spring::spring_sample_at(
                frames / fps,
                crate::spring::SpringParams {
                    mass: *mass,
                    stiffness: *stiffness,
                    damping: *damping,
                    initial_velocity: *initial_velocity,
                },
            );
            Ok(MotionValue::Number(match output {
                crate::spring::SpringOutput::Position => sample.position,
                crate::spring::SpringOutput::Velocity => sample.velocity,
            }))
        }
        Expr::Interpolate {
            input,
            stops,
            easings,
            color_space,
            extrapolate_left,
            extrapolate_right,
        } => interpolate(
            at,
            as_number(at, "interpolate", child(*input)?)?,
            stops,
            easings,
            *color_space,
            *extrapolate_left,
            *extrapolate_right,
        ),
        Expr::SimulationSample {
            time,
            dt,
            duration,
            samples,
        } => {
            let time = as_number(at, "simulate.at", child(*time)?)?;
            if samples.len() < 2 || !dt.is_finite() || *dt <= 0.0 || !duration.is_finite() {
                return Err(EvalError::BadInterpolate { at });
            }
            let position = time.clamp(0.0, *duration) / dt;
            let index = (position.floor() as usize).min(samples.len() - 2);
            let fraction = position - index as f64;
            finite(
                at,
                "simulate.at",
                MotionValue::Number(
                    samples[index] + (samples[index + 1] - samples[index]) * fraction,
                ),
            )
        }
        Expr::AudioSample { time, fps, samples } => {
            let time = as_number(at, "audioAnalysis", child(*time)?)?;
            if *fps == 0 || samples.is_empty() {
                return Err(EvalError::BadInterpolate { at });
            }
            let index = if time <= 0.0 {
                0
            } else {
                crate::time::frame_index_floor(time * f64::from(*fps)) as usize
            }
            .min(samples.len() - 1);
            finite(at, "audioAnalysis", MotionValue::Number(samples[index]))
        }
    }
}

/// Missing unit or viewport contexts remain undefined and produce errors, never zero-valued
/// geometry.
fn context_value(
    input: ContextInput,
    ctx: &MotionContext,
    unit: Option<UnitContext>,
    viewport: Option<(f64, f64)>,
) -> Option<MotionValue> {
    let number = |value| MotionValue::Number(f64::from(value));
    Some(match input {
        ContextInput::LocalFrame => number(ctx.local_frame),
        ContextInput::LocalProgress => MotionValue::Number(ctx.progress),
        ContextInput::CompositionSeconds => MotionValue::Number(ctx.sample.composition().as_f64()),
        ContextInput::HostSeconds => MotionValue::Number(ctx.host.sample.composition().as_f64()),
        ContextInput::HostDuration => MotionValue::Number(ctx.host.duration.as_f64()),
        ContextInput::HostProgress => MotionValue::Number(ctx.host.progress.as_f64()),
        ContextInput::DurationFrames => number(ctx.duration_frames),
        ContextInput::FpsNum => MotionValue::Number(ctx.fps.numerator() as f64),
        ContextInput::FpsDen => number(ctx.fps.denominator()),
        ContextInput::UnitIndex => number(unit?.index),
        ContextInput::UnitCount => number(unit?.count),
        ContextInput::UnitStart => number(unit?.start),
        ContextInput::UnitEnd => number(unit?.end),
        ContextInput::ViewportWidth => MotionValue::Number(viewport?.0),
        ContextInput::ViewportHeight => MotionValue::Number(viewport?.1),
    })
}

/// Evaluate one selector against the unit's normalized leading edge. Using `index / count`
/// means an eight-letter word has sample positions 0, 1/8, …, 7/8; a 50% reveal therefore puts
/// the fifth letter at the feathered front rather than jumping past it.
fn range_selector(
    unit: UnitContext,
    start: f64,
    end: f64,
    offset: f64,
    softness: f64,
    shape: RangeShape,
) -> Option<f64> {
    if unit.count == 0
        || unit.index >= unit.count
        || !start.is_finite()
        || !end.is_finite()
        || !offset.is_finite()
        || !softness.is_finite()
        || !(0.0..=1.0).contains(&start)
        || !(0.0..=1.0).contains(&end)
        || !(0.0..=1.0).contains(&softness)
    {
        return None;
    }
    if start >= end {
        return Some(0.0);
    }
    let position = f64::from(unit.index) / f64::from(unit.count);
    let left = start + offset;
    let right = end + offset;
    if !left.is_finite() || !right.is_finite() {
        return None;
    }
    if shape == RangeShape::Square || softness == 0.0 {
        return Some(f64::from(position >= left && position <= right));
    }
    let left_edge = if left <= 0.0 {
        1.0
    } else {
        (0.5 + (position - left) / softness).clamp(0.0, 1.0)
    };
    let right_edge = if right >= 1.0 {
        1.0
    } else {
        (0.5 + (right - position) / softness).clamp(0.0, 1.0)
    };
    let value = match shape {
        RangeShape::Square => unreachable!(),
        RangeShape::Ramp => left_edge.min(right_edge),
        RangeShape::Smooth => {
            let smooth = |value: f64| value * value * (3.0 - 2.0 * value);
            smooth(left_edge).min(smooth(right_edge))
        }
        RangeShape::Triangle => {
            let center = left / 2.0 + right / 2.0;
            let half_width = (right - left + softness) / 2.0;
            if half_width <= 0.0 {
                0.0
            } else {
                (1.0 - (position - center).abs() / half_width).clamp(0.0, 1.0)
            }
        }
    };
    Some(value)
}

fn as_number(at: usize, op: &'static str, value: &MotionValue) -> Result<f64, EvalError> {
    match value {
        MotionValue::Number(value) => Ok(*value),
        _ => Err(EvalError::TypeMismatch { at, op }),
    }
}

fn as_path<'a>(
    at: usize,
    op: &'static str,
    value: &'a MotionValue,
) -> Result<&'a PathData, EvalError> {
    match value {
        MotionValue::PathData(value) => Ok(value),
        _ => Err(EvalError::TypeMismatch { at, op }),
    }
}

fn as_point<'a>(
    at: usize,
    op: &'static str,
    value: &'a MotionValue,
) -> Result<&'a Point, EvalError> {
    match value {
        MotionValue::Point(value) => Ok(value),
        _ => Err(EvalError::TypeMismatch { at, op }),
    }
}

fn number_arith(
    at: usize,
    op: &'static str,
    lhs: &MotionValue,
    rhs: &MotionValue,
    f: impl FnOnce(f64, f64) -> f64,
) -> Result<MotionValue, EvalError> {
    Ok(MotionValue::Number(f(
        as_number(at, op, lhs)?,
        as_number(at, op, rhs)?,
    )))
}

fn compare(
    at: usize,
    op: CompareOp,
    lhs: &MotionValue,
    rhs: &MotionValue,
) -> Result<MotionValue, EvalError> {
    let text_enum_pair = matches!(
        (lhs, rhs),
        (MotionValue::Str(_), MotionValue::Enum(_)) | (MotionValue::Enum(_), MotionValue::Str(_))
    );
    if core::mem::discriminant(lhs) != core::mem::discriminant(rhs)
        && !(matches!(op, CompareOp::Eq | CompareOp::NotEq) && text_enum_pair)
    {
        return Err(EvalError::TypeMismatch { at, op: "compare" });
    }
    let equal = match (lhs, rhs) {
        (MotionValue::Str(lhs), MotionValue::Enum(rhs))
        | (MotionValue::Enum(lhs), MotionValue::Str(rhs)) => lhs == rhs,
        _ => lhs == rhs,
    };
    let value = match op {
        CompareOp::Eq => equal,
        CompareOp::NotEq => !equal,
        CompareOp::Lt => as_number(at, "compare", lhs)? < as_number(at, "compare", rhs)?,
        CompareOp::Lte => as_number(at, "compare", lhs)? <= as_number(at, "compare", rhs)?,
        CompareOp::Gt => as_number(at, "compare", lhs)? > as_number(at, "compare", rhs)?,
        CompareOp::Gte => as_number(at, "compare", lhs)? >= as_number(at, "compare", rhs)?,
    };
    Ok(MotionValue::Bool(value))
}

fn interpolate(
    at: usize,
    input: f64,
    stops: &[InterpolateStop],
    easings: &[MotionEasing],
    color_space: GradientInterpolation,
    left: Extrapolation,
    right: Extrapolation,
) -> Result<MotionValue, EvalError> {
    if stops.len() < 2 || (!easings.is_empty() && easings.len() + 1 != stops.len()) {
        return Err(EvalError::BadInterpolate { at });
    }
    let first = &stops[0];
    let last = &stops[stops.len() - 1];
    if input < first.input {
        return match left {
            Extrapolation::Clamp => Ok(first.output.clone()),
            Extrapolation::Extend => {
                interpolate_segment(at, input, &stops[0], &stops[1], None, color_space)
            }
        };
    }
    if input >= last.input {
        return match right {
            Extrapolation::Clamp => Ok(last.output.clone()),
            Extrapolation::Extend => {
                interpolate_segment(at, input, &stops[stops.len() - 2], last, None, color_space)
            }
        };
    }
    let segment = stops
        .windows(2)
        .position(|pair| input >= pair[0].input && input < pair[1].input)
        .ok_or(EvalError::BadInterpolate { at })?;
    interpolate_segment(
        at,
        input,
        &stops[segment],
        &stops[segment + 1],
        easings.get(segment).copied(),
        color_space,
    )
}

fn interpolate_segment(
    at: usize,
    input: f64,
    lhs: &InterpolateStop,
    rhs: &InterpolateStop,
    easing: Option<MotionEasing>,
    color_space: GradientInterpolation,
) -> Result<MotionValue, EvalError> {
    let span = rhs.input - lhs.input;
    if span <= 0.0 || !span.is_finite() {
        return Err(EvalError::BadInterpolate { at });
    }
    let raw = (input - lhs.input) / span;
    let t = easing.map_or(raw, |easing| ease(easing, raw));
    lerp_value(at, &lhs.output, &rhs.output, t, color_space)
}

fn ease(easing: MotionEasing, t: f64) -> f64 {
    easing.evaluate(t)
}

fn lerp_value(
    at: usize,
    lhs: &MotionValue,
    rhs: &MotionValue,
    t: f64,
    color_space: GradientInterpolation,
) -> Result<MotionValue, EvalError> {
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    let value = match (lhs, rhs) {
        (MotionValue::Number(a), MotionValue::Number(b)) => MotionValue::Number(lerp(*a, *b)),
        (MotionValue::Length(a), MotionValue::Length(b)) if a.unit == b.unit => {
            MotionValue::Length(Length {
                value: lerp(a.value, b.value),
                unit: a.unit,
            })
        }
        (MotionValue::Length2(a), MotionValue::Length2(b))
            if a.x.unit == b.x.unit && a.y.unit == b.y.unit =>
        {
            MotionValue::Length2(Length2 {
                x: Length {
                    value: lerp(a.x.value, b.x.value),
                    unit: a.x.unit,
                },
                y: Length {
                    value: lerp(a.y.value, b.y.value),
                    unit: a.y.unit,
                },
            })
        }
        (MotionValue::Angle(a), MotionValue::Angle(b)) => MotionValue::Angle(Angle {
            value: lerp(a.as_degrees(), b.as_degrees()),
            unit: AngleUnit::Deg,
        }),
        (MotionValue::Color(a), MotionValue::Color(b)) => MotionValue::Color(
            valle_draw::program::interpolate_author_colors(*a, *b, t, color_space),
        ),
        (MotionValue::Vec2(a), MotionValue::Vec2(b)) => {
            MotionValue::Vec2(Vec2::new(lerp(a.x, b.x), lerp(a.y, b.y)))
        }
        (MotionValue::Point(a), MotionValue::Point(b)) => {
            MotionValue::Point(Point::new(lerp(a.x, b.x), lerp(a.y, b.y)))
        }
        (MotionValue::Rect(a), MotionValue::Rect(b)) => MotionValue::Rect(Rect::new(
            lerp(a.x, b.x),
            lerp(a.y, b.y),
            lerp(a.width, b.width),
            lerp(a.height, b.height),
        )),
        (MotionValue::PathData(a), MotionValue::PathData(b)) => MotionValue::PathData(
            a.morph(b, t)
                .map_err(|reason| EvalError::BadGeometry { at, reason })?,
        ),
        _ => {
            return Err(EvalError::TypeMismatch {
                at,
                op: "interpolate",
            });
        }
    };
    finite(at, "interpolate", value)
}

fn finite(at: usize, op: &'static str, value: MotionValue) -> Result<MotionValue, EvalError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(EvalError::NonFinite { at, op })
    }
}

#[cfg(test)]
mod tests {
    /// Verify easing changes evaluated values, including cubic-bezier overshoot.
    #[test]
    fn easings_actually_bend_the_curve_and_overshoot_passes_the_endpoint() {
        use super::*;
        let at = |easing: MotionEasing, t: f64| -> f64 {
            let stops = [
                InterpolateStop {
                    input: 0.0,
                    output: MotionValue::Number(0.0),
                },
                InterpolateStop {
                    input: 1.0,
                    output: MotionValue::Number(1.0),
                },
            ];
            let MotionValue::Number(value) = interpolate_segment(
                0,
                t,
                &stops[0],
                &stops[1],
                Some(easing),
                GradientInterpolation::Srgb,
            )
            .unwrap() else {
                panic!("number");
            };
            value
        };

        let linear_mid = at(MotionEasing::Linear, 0.5);
        assert!(
            (linear_mid - 0.5).abs() < 1e-12,
            "the linear midpoint is 0.5"
        );
        assert!(
            at(MotionEasing::EaseOut, 0.5) > linear_mid + 0.05,
            "easeOut must exceed the linear midpoint",
        );
        assert!(
            at(MotionEasing::EaseIn, 0.5) < linear_mid - 0.05,
            "easeIn must remain below the linear midpoint",
        );

        // Unclamped Bezier y values permit back-out overshoot.
        let back_out = MotionEasing::CubicBezier {
            p: [0.34, 1.56, 0.64, 1.0],
        };
        let peak = (60..=90)
            .map(|step| at(back_out, f64::from(step) / 100.0))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            peak > 1.0,
            "elastic easing must overshoot; actual peak {peak}"
        );
        assert!(
            (at(back_out, 1.0) - 1.0).abs() < 1e-12,
            "the endpoint must remain exactly one"
        );
    }

    use super::*;
    use crate::{ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, NodeId, NodeKind, SceneNode};
    use valle_timeline::FrameRate;

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            assets: BTreeMap::new(),
        }
    }

    fn artifact(exprs: Vec<Expr>) -> SceneArtifact {
        SceneArtifact {
            role: valle_timeline::MotionRole::Clip,
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "eval-test".into(),
            composition: None,
            controls: controls(),
            resource_refs: vec![],
            exprs,
            instance_groups: vec![],
            nodes: vec![SceneNode {
                key: "root".into(),
                kind: NodeKind::Group,
                space: None,
                class_names: vec![],
                class_conditions: Default::default(),
                styles: vec![],
                visibility: None,
                children: ChildRange::EMPTY,
                semantic: None,
            }],
            node_children: vec![],
            root: NodeId(0),
        }
    }

    fn context(frame: u32) -> MotionContext {
        crate::motion_context_at_frame(frame, 8, FrameRate::new(30, 1).unwrap()).unwrap()
    }

    #[test]
    fn audio_samples_hold_onset_and_wrapped_phase_within_each_frame() {
        let sample_at = |time| {
            let artifact = artifact(vec![
                Expr::Const {
                    value: MotionValue::Number(time),
                },
                Expr::AudioSample {
                    time: ExprId(0),
                    fps: 30,
                    samples: vec![0.0, 1.0, 0.0],
                },
            ]);
            artifact.validate().unwrap();
            let ctx = context(0);
            eval_all(
                &artifact,
                EvalInputs {
                    ctx: &ctx,
                    props: &ResolvedProps::default(),
                    unit: None,
                    viewport: None,
                },
            )
            .unwrap()[1]
                .clone()
        };
        assert_eq!(sample_at(0.0), MotionValue::Number(0.0));
        assert_eq!(sample_at(0.05), MotionValue::Number(1.0));
        assert_eq!(sample_at(0.066), MotionValue::Number(1.0));
        assert_eq!(sample_at(2.0 / 30.0), MotionValue::Number(0.0));
    }

    #[test]
    fn instance_template_reuses_frame_expression_and_evaluates_rows_independently() {
        use crate::expr::ExprType;
        use crate::{
            InstanceColumn, InstanceColumnValues, InstanceGroup, StyleBinding, StyleValue,
        };

        let group = InstanceGroup {
            template: SceneNode {
                key: "template".into(),
                kind: NodeKind::Box,
                space: None,
                class_names: vec!["absolute".into()],
                class_conditions: Default::default(),
                styles: vec![StyleBinding {
                    property: "left".into(),
                    value: StyleValue::Expr { expr: ExprId(6) },
                }],
                visibility: None,
                children: ChildRange::EMPTY,
                semantic: None,
            },
            template_key_prefix: "template".into(),
            template_children: Vec::new(),
            keys: crate::InstanceKeys::Explicit(vec!["a".into(), "b".into(), "c".into()]),
            columns: vec![InstanceColumn {
                name: "delay".into(),
                values: InstanceColumnValues::Numbers(vec![0.0, 2.0, 4.0]),
            }],
            exprs: vec![
                Expr::Context {
                    input: ContextInput::LocalFrame,
                },
                Expr::Const {
                    value: MotionValue::Number(2.0),
                },
                Expr::Mul {
                    lhs: ExprId(0),
                    rhs: ExprId(1),
                },
                Expr::InstanceField {
                    column: 0,
                    value_type: ExprType::Number,
                },
                Expr::Sub {
                    lhs: ExprId(2),
                    rhs: ExprId(3),
                },
                Expr::InstanceIndex,
                Expr::Add {
                    lhs: ExprId(4),
                    rhs: ExprId(5),
                },
                Expr::InstanceCount,
            ],
        };
        let mut artifact = artifact(vec![]);
        artifact.capability_set = CapabilitySet::new(
            artifact
                .capability_set
                .names
                .iter()
                .cloned()
                .chain(std::iter::once(crate::GEOMETRY_BATCH_CAPABILITY.into())),
        );
        artifact.instance_groups.push(group.clone());
        artifact.nodes[0].children = ChildRange { start: 0, end: 1 };
        artifact.node_children.push(NodeId(1));
        artifact.nodes.push(SceneNode {
            key: "batch".into(),
            kind: NodeKind::InstanceBatch { group: 0 },
            space: None,
            class_names: vec!["absolute".into()],
            class_conditions: Default::default(),
            styles: vec![],
            visibility: None,
            children: ChildRange::EMPTY,
            semantic: None,
        });
        artifact.validate().unwrap();
        assert_eq!(group.frame_expression_evaluations(), 16);
        let props = ResolvedProps::default();
        let values_at = |frame| {
            let ctx = context(frame);
            eval_instances(
                &group,
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: None,
                },
            )
            .unwrap()
        };
        assert_eq!(
            values_at(5).iter().map(|row| &row[6]).collect::<Vec<_>>(),
            [
                &MotionValue::Number(10.0),
                &MotionValue::Number(9.0),
                &MotionValue::Number(8.0)
            ]
        );
        assert_eq!(values_at(5)[0][7], MotionValue::Number(3.0));
        assert_eq!(values_at(2)[2][6], MotionValue::Number(2.0));
        assert_eq!(values_at(5)[1][6], MotionValue::Number(9.0));

        let expected = values_at(5);
        let ctx = context(5);
        let mut value_table = None;
        try_for_each_instance(
            &group,
            EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            },
            |row, values| {
                assert_eq!(values, expected[row]);
                assert_eq!(value_table.get_or_insert(values.as_ptr()), &values.as_ptr());
                Ok::<_, EvalError>(())
            },
        )
        .unwrap();

        let plan = InstanceEvalPlan::new(&group);
        assert_eq!(
            plan.work_for_rows(group.rows()),
            group.frame_expression_evaluations()
        );
        assert_eq!(plan.work_for_rows(0), 0);
        let mut shared = SampleValueCache::for_len(plan.len());
        for (frame, selected) in [(5, 0..3), (2, 1..3), (5, 0..3)] {
            let ctx = context(frame);
            let inputs = EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            };
            let expected = eval_instances(&group, inputs).unwrap();
            let mut actual = Vec::new();
            try_for_each_instance_rows_planned(
                &group,
                &plan,
                inputs,
                selected.clone(),
                Some(&mut shared),
                |row, values| {
                    actual.push((row, values.to_vec()));
                    Ok::<_, EvalError>(())
                },
            )
            .unwrap();
            assert_eq!(
                actual,
                selected
                    .map(|row| (row, expected[row].clone()))
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(shared.counts(), (1, 2));

        let mut forged = artifact;
        forged.instance_groups[0].columns[0].values = InstanceColumnValues::Other(vec![
            MotionValue::Number(0.0),
            MotionValue::Bool(true),
            MotionValue::Number(4.0),
        ]);
        assert!(forged.validate().is_err());
    }

    #[test]
    fn planned_slice_only_evaluates_requested_ancestors() {
        let mut exprs = vec![
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Const {
                value: MotionValue::Number(2.0),
            },
            Expr::Mul {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
        ];
        exprs.extend((0..4096).map(|value| Expr::Const {
            value: MotionValue::Number(value as f64),
        }));
        let root = ExprId(exprs.len() as u32);
        exprs.push(Expr::Add {
            lhs: ExprId(2),
            rhs: ExprId(1),
        });
        let artifact = artifact(exprs);
        let plan = EvalPlan::new(&artifact.exprs);
        let order = plan.schedule([root]);
        assert_eq!(order, vec![ExprId(0), ExprId(1), ExprId(2), root]);

        let props = ResolvedProps::default();
        for frame in [1, 3, 1] {
            let ctx = context(frame);
            let inputs = EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            };
            let sparse = eval_roots_planned(&artifact, &plan, inputs, &order).unwrap();
            let dense = eval_all_planned(&artifact, &plan, inputs).unwrap();
            assert_eq!(sparse.len(), 4);
            assert_eq!(sparse[&root], dense[root.0 as usize]);
            assert_eq!(
                sparse[&root],
                MotionValue::Number(f64::from(frame) * 2.0 + 2.0)
            );
        }
    }

    #[test]
    fn sampling_rounds_share_static_values_but_recompute_time_values() {
        let artifact = artifact(vec![
            Expr::Context {
                input: ContextInput::DurationFrames,
            },
            Expr::Const {
                value: MotionValue::Number(5.0),
            },
            Expr::Add {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Mul {
                lhs: ExprId(2),
                rhs: ExprId(3),
            },
            Expr::Add {
                lhs: ExprId(4),
                rhs: ExprId(2),
            },
        ]);
        let plan = EvalPlan::new(&artifact.exprs);
        assert!(!plan.sample_dependent(ExprId(0)));
        assert!(!plan.sample_dependent(ExprId(2)));
        assert!(plan.sample_dependent(ExprId(4)));
        let orders = [plan.schedule([ExprId(4)]), plan.schedule([ExprId(5)])];
        let mut shared = BTreeMap::new();
        let props = ResolvedProps::default();
        for (frame, expected_evaluations) in [(2, 6), (5, 3), (2, 3)] {
            let ctx = context(frame);
            let inputs = EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            };
            let mut values = BTreeMap::new();
            let mut evaluated = 0;
            for order in &orders {
                evaluated += eval_roots_planned_into(
                    &artifact,
                    &plan,
                    inputs,
                    order,
                    &mut values,
                    Some(&mut shared),
                )
                .unwrap();
            }
            assert_eq!(evaluated, expected_evaluations);
            assert_eq!(shared.len(), 3);
            assert_eq!(values[&ExprId(5)], eval_all(&artifact, inputs).unwrap()[5]);
        }
    }

    #[test]
    fn request_cache_preserves_full_and_dense_values_across_out_of_order_samples() {
        let artifact = artifact(vec![
            Expr::Context {
                input: ContextInput::DurationFrames,
            },
            Expr::Prop {
                name: "gain".into(),
            },
            Expr::Add {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Mul {
                lhs: ExprId(2),
                rhs: ExprId(3),
            },
            Expr::Add {
                lhs: ExprId(4),
                rhs: ExprId(2),
            },
        ]);
        let plan = EvalPlan::new(&artifact.exprs);
        let order = plan.schedule([ExprId(5)]);
        let mut full_cache = SampleValueCache::new(&plan);
        let mut dense_cache = SampleValueCache::new(&plan);
        let props = ResolvedProps(BTreeMap::from([("gain".into(), MotionValue::Number(5.0))]));
        for frame in [2, 5, 2] {
            let ctx = context(frame);
            let inputs = EvalInputs {
                ctx: &ctx,
                props: &props,
                unit: None,
                viewport: None,
            };
            let expected = eval_all_planned(&artifact, &plan, inputs).unwrap();
            assert_eq!(
                eval_all_planned_cached(&artifact, &plan, inputs, Some(&mut full_cache)).unwrap(),
                expected
            );
            assert_eq!(
                eval_roots_planned_dense_cached(
                    &artifact,
                    &plan,
                    inputs,
                    &order,
                    Some(&mut dense_cache),
                )
                .unwrap(),
                expected
            );
        }
        assert_eq!(full_cache.counts(), (3, 6));
        assert_eq!(dense_cache.counts(), (3, 6));
        let changed_props =
            ResolvedProps(BTreeMap::from([("gain".into(), MotionValue::Number(9.0))]));
        let ctx = context(2);
        let changed_inputs = EvalInputs {
            ctx: &ctx,
            props: &changed_props,
            unit: None,
            viewport: None,
        };
        let mut next_request = SampleValueCache::new(&plan);
        let changed =
            eval_all_planned_cached(&artifact, &plan, changed_inputs, Some(&mut next_request))
                .unwrap();
        assert_eq!(
            changed,
            eval_all_planned(&artifact, &plan, changed_inputs).unwrap()
        );
        assert_eq!(changed[5], MotionValue::Number(51.0));
        assert_eq!(next_request.counts(), (3, 0));
    }

    #[test]
    fn planned_slice_skips_errors_in_unrelated_expressions() {
        let artifact = artifact(vec![
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            Expr::Const {
                value: MotionValue::Number(0.0),
            },
            Expr::Div {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Const {
                value: MotionValue::Number(7.0),
            },
        ]);
        let ctx = context(0);
        let props = ResolvedProps::default();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        };
        let plan = EvalPlan::new(&artifact.exprs);
        let order = plan.schedule([ExprId(3)]);
        assert_eq!(order, vec![ExprId(3)]);
        let sparse = eval_roots_planned(&artifact, &plan, inputs, &order).unwrap();
        assert_eq!(sparse[&ExprId(3)], MotionValue::Number(7.0));
        assert_eq!(
            eval_all(&artifact, inputs),
            Err(EvalError::DivisionByZero { at: 2 })
        );
    }

    /// Base evaluation skips unit expressions; per-unit evaluation recomputes only their dependent
    /// subgraph.
    #[test]
    fn unit_expressions_are_deferred_to_per_unit_evaluation() {
        let artifact = artifact(vec![
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Context {
                input: ContextInput::UnitIndex,
            },
            Expr::Mul {
                lhs: ExprId(1),
                rhs: ExprId(1),
            },
        ]);
        let ctx = context(3);
        let props = crate::ResolvedProps::default();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        };
        let base = eval_all(&artifact, inputs).unwrap();
        assert_eq!(
            base[0],
            MotionValue::Number(3.0),
            "evaluate expressions independent of unit context normally"
        );

        let unit = UnitContext {
            index: 4,
            count: 6,
            start: 12,
            end: 15,
        };
        let values = eval_units(
            &artifact,
            &base,
            inputs,
            unit,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            values[0], base[0],
            "reuse base values for expressions independent of unit context"
        );
        assert_eq!(values[1], MotionValue::Number(4.0));
        assert_eq!(
            values[2],
            MotionValue::Number(16.0),
            "recompute transitive dependencies"
        );
    }

    /// Reject unit inputs outside per-unit evaluation.
    #[test]
    fn unit_input_without_a_unit_is_an_error_not_a_zero() {
        let ctx = context(3);
        let props = crate::ResolvedProps::default();
        assert_eq!(
            eval_one(
                0,
                &Expr::Context {
                    input: ContextInput::UnitIndex,
                },
                &[],
                EvalInputs {
                    ctx: &ctx,
                    props: &props,
                    unit: None,
                    viewport: None,
                },
                None,
                None,
            ),
            Err(EvalError::UnitOutsidePerUnit { at: 0 })
        );
    }

    #[test]
    fn forward_pass_covers_context_arithmetic_select_and_interpolate() {
        let artifact = artifact(vec![
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Const {
                value: MotionValue::Number(2.0),
            },
            Expr::Mul {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Const {
                value: MotionValue::Number(0.0),
            },
            Expr::Compare {
                op: CompareOp::Gt,
                lhs: ExprId(2),
                rhs: ExprId(3),
            },
            Expr::Interpolate {
                input: ExprId(2),
                stops: vec![
                    InterpolateStop {
                        input: 0.0,
                        output: MotionValue::Length(Length::px(0.0)),
                    },
                    InterpolateStop {
                        input: 4.0,
                        output: MotionValue::Length(Length::px(40.0)),
                    },
                ],
                easings: vec![MotionEasing::Linear],
                color_space: GradientInterpolation::Srgb,
                extrapolate_left: Extrapolation::Clamp,
                extrapolate_right: Extrapolation::Clamp,
            },
            Expr::Const {
                value: MotionValue::Str("yes".into()),
            },
            Expr::Const {
                value: MotionValue::Str("no".into()),
            },
            Expr::Select {
                condition: ExprId(4),
                when_true: ExprId(6),
                when_false: ExprId(7),
            },
        ]);
        artifact.validate().unwrap();
        let values = eval_all(
            &artifact,
            EvalInputs {
                ctx: &context(1),
                props: &ResolvedProps::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        assert_eq!(values[2], MotionValue::Number(2.0));
        assert_eq!(values[5], MotionValue::Length(Length::px(20.0)));
        assert_eq!(values[8], MotionValue::Str("yes".into()));
    }

    #[test]
    fn select_enum_compares_with_authored_string_literals() {
        let artifact = artifact(vec![
            Expr::Const {
                value: MotionValue::Enum("multiplane".into()),
            },
            Expr::Const {
                value: MotionValue::Str("multiplane".into()),
            },
            Expr::Compare {
                op: CompareOp::Eq,
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
        ]);
        artifact.validate().unwrap();
        let values = eval_all(
            &artifact,
            EvalInputs {
                ctx: &context(0),
                props: &ResolvedProps::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        assert_eq!(values[2], MotionValue::Bool(true));
    }

    #[test]
    fn division_by_zero_and_non_finite_results_fail_closed() {
        let artifact = artifact(vec![
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            Expr::Const {
                value: MotionValue::Number(0.0),
            },
            Expr::Div {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
        ]);
        let error = eval_all(
            &artifact,
            EvalInputs {
                ctx: &context(0),
                props: &ResolvedProps::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap_err();
        assert_eq!(error, EvalError::DivisionByZero { at: 2 });
    }

    #[test]
    fn css_tokens_cover_the_typed_value_domain() {
        assert_eq!(css_token(&MotionValue::Number(10.0)), "10");
        assert_eq!(css_token(&MotionValue::Length(Length::px(12.0))), "12px");
        assert_eq!(
            css_token(&MotionValue::Length2(Length2::px(0.0, 24.0))),
            "0px 24px"
        );
        assert_eq!(css_token(&MotionValue::Angle(Angle::deg(90.0))), "90deg");
    }

    #[test]
    fn template_expression_formats_scalar_css_tokens_deterministically() {
        let artifact = artifact(vec![
            Expr::Const {
                value: MotionValue::Number(1.5),
            },
            Expr::Const {
                value: MotionValue::Angle(Angle::deg(20.0)),
            },
            Expr::Template {
                parts: vec![
                    TemplatePart::Text {
                        value: "blur(".into(),
                    },
                    TemplatePart::Expr { expr: ExprId(0) },
                    TemplatePart::Text {
                        value: "px) hue-rotate(".into(),
                    },
                    TemplatePart::Expr { expr: ExprId(1) },
                    TemplatePart::Text { value: ")".into() },
                ],
            },
        ]);
        artifact.validate().unwrap();
        let values = eval_all(
            &artifact,
            EvalInputs {
                ctx: &context(0),
                props: &ResolvedProps::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        assert_eq!(
            values[2],
            MotionValue::Str("blur(1.5px) hue-rotate(20deg)".into())
        );
    }

    #[test]
    fn interpolate_covers_every_continuous_value_type() {
        let pairs = [
            (MotionValue::Number(0.0), MotionValue::Number(10.0)),
            (
                MotionValue::Length(Length::px(0.0)),
                MotionValue::Length(Length::px(10.0)),
            ),
            (
                MotionValue::Length2(Length2::px(0.0, 10.0)),
                MotionValue::Length2(Length2::px(10.0, 20.0)),
            ),
            (
                MotionValue::Angle(Angle::deg(0.0)),
                MotionValue::Angle(Angle::deg(90.0)),
            ),
            (
                MotionValue::Color(valle_draw::program::AuthorColor::from_srgb8(
                    valle_draw::Rgba::new(255, 0, 0, 255),
                )),
                MotionValue::Color(valle_draw::program::AuthorColor::from_srgb8(
                    valle_draw::Rgba::new(0, 0, 255, 255),
                )),
            ),
            (
                MotionValue::Vec2(Vec2::new(0.0, 10.0)),
                MotionValue::Vec2(Vec2::new(10.0, 20.0)),
            ),
            (
                MotionValue::Rect(Rect::new(0.0, 10.0, 20.0, 30.0)),
                MotionValue::Rect(Rect::new(10.0, 20.0, 30.0, 40.0)),
            ),
        ];
        let mut exprs = vec![Expr::Const {
            value: MotionValue::Number(0.5),
        }];
        for (from, to) in pairs {
            exprs.push(Expr::Interpolate {
                input: ExprId(0),
                stops: vec![
                    InterpolateStop {
                        input: 0.0,
                        output: from,
                    },
                    InterpolateStop {
                        input: 1.0,
                        output: to,
                    },
                ],
                easings: vec![],
                color_space: GradientInterpolation::Srgb,
                extrapolate_left: Extrapolation::Clamp,
                extrapolate_right: Extrapolation::Clamp,
            });
        }
        let artifact = artifact(exprs);
        artifact.validate().unwrap();
        let values = eval_all(
            &artifact,
            EvalInputs {
                ctx: &context(1),
                props: &ResolvedProps::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        assert_eq!(values[1], MotionValue::Number(5.0));
        assert_eq!(values[2], MotionValue::Length(Length::px(5.0)));
        assert_eq!(values[3], MotionValue::Length2(Length2::px(5.0, 15.0)));
        assert_eq!(values[4], MotionValue::Angle(Angle::deg(45.0)));
        let MotionValue::Color(color) = values[5] else {
            panic!("color")
        };
        assert_eq!(color.to_srgb8(), valle_draw::Rgba::new(128, 0, 128, 255));
        assert_eq!(values[6], MotionValue::Vec2(Vec2::new(5.0, 15.0)));
        assert_eq!(
            values[7],
            MotionValue::Rect(Rect::new(5.0, 15.0, 25.0, 35.0))
        );
    }

    #[test]
    fn prop_resolution_applies_defaults_bounds_and_unknown_name_checks() {
        let mut controls = controls();
        controls.props.insert(
            "amount".into(),
            crate::PropControl {
                control: ControlType::Number {
                    min: Some(0.0),
                    max: Some(1.0),
                    step: Some(0.1),
                },
                default: Some(MotionValue::Number(0.5)),
                required: false,
                label: None,
            },
        );
        let resolved = resolve_props(&controls, &BTreeMap::new()).unwrap();
        assert_eq!(resolved.get("amount"), Some(&MotionValue::Number(0.5)));

        let out_of_range = BTreeMap::from([("amount".into(), MotionValue::Number(2.0))]);
        assert_eq!(
            resolve_props(&controls, &out_of_range),
            Err(EvalError::InvalidProp {
                name: "amount".into()
            })
        );
        let unknown = BTreeMap::from([("other".into(), MotionValue::Number(0.5))]);
        assert_eq!(
            resolve_props(&controls, &unknown),
            Err(EvalError::UnknownProp {
                name: "other".into()
            })
        );
    }
}
