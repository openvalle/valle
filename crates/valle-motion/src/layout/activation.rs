//! Conservative pre-layout activation for independently positioned paint leaves.
//!
//! A hidden absolute leaf does not contribute flow geometry or pixels. Its visibility predicate
//! still runs at the current sample time; unrelated paint expressions can remain unevaluated.
//! Nodes with richer composition semantics stay on the complete evaluation path.

use crate::eval::{EvalInputs, EvalPlan, SampleValueCache, eval_roots_planned_dense_cached};
use crate::time_function::TimeFunction;
use std::collections::BTreeMap;
use std::ops::Range;
use valle_timeline::FrameRate;
use valle_timeline::internal::SampleTime;

use super::dependencies::SceneDependencies;
use crate::{
    CompareOp, EvalError, Expr, ExprId, IndexFormula, InstanceColumnValues, InstanceGroup,
    MotionContext, MotionValue, NodeId, NodeKind, ResolvedProps, SceneArtifact, StyleValue,
};

fn swapped(op: CompareOp) -> CompareOp {
    match op {
        CompareOp::Eq => CompareOp::Eq,
        CompareOp::NotEq => CompareOp::NotEq,
        CompareOp::Lt => CompareOp::Gt,
        CompareOp::Lte => CompareOp::Gte,
        CompareOp::Gt => CompareOp::Lt,
        CompareOp::Gte => CompareOp::Lte,
    }
}

#[derive(Debug, Clone)]
struct ThresholdGate {
    driver: TimeFunction,
    threshold: f64,
    op: CompareOp,
}

const MAX_VISIBILITY_PROOF_DEPTH: usize = 8;
const MAX_VISIBILITY_PROOF_NODES: usize = 31;

/// Only certified intervals are retained. An unresolved gap evaluates its gate at the sample.
#[derive(Debug, Clone)]
struct VisibilityCertificate {
    intervals: Vec<CertifiedInterval>,
}

#[derive(Debug, Clone, Copy)]
struct CertifiedInterval {
    start: f64,
    end: f64,
    active: bool,
}

impl VisibilityCertificate {
    fn prove(gate: &ThresholdGate, start: f64, end: f64) -> Option<Self> {
        let mut budget = MAX_VISIBILITY_PROOF_NODES;
        let mut intervals = Vec::new();
        Self::prove_into(gate, start, end, 0, &mut budget, &mut intervals);
        (!intervals.is_empty()).then_some(Self { intervals })
    }

    fn prove_into(
        gate: &ThresholdGate,
        start: f64,
        end: f64,
        depth: usize,
        budget: &mut usize,
        intervals: &mut Vec<CertifiedInterval>,
    ) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        if let Some(active) = gate
            .driver
            .compare_on_interval(start, end, gate.op, gate.threshold)
        {
            if let Some(previous) = intervals.last_mut()
                && previous.active == active
                && previous.end == start
            {
                previous.end = end;
            } else {
                intervals.push(CertifiedInterval { start, end, active });
            }
            return;
        }
        if depth == MAX_VISIBILITY_PROOF_DEPTH {
            return;
        }
        let middle = start + (end - start) * 0.5;
        if !middle.is_finite() || middle <= start || middle >= end {
            return;
        }
        Self::prove_into(gate, start, middle, depth + 1, budget, intervals);
        Self::prove_into(gate, middle, end, depth + 1, budget, intervals);
    }

    fn at(&self, time: f64) -> Option<bool> {
        let next = self
            .intervals
            .partition_point(|interval| interval.end < time);
        self.intervals
            .get(next)
            .filter(|interval| interval.start <= time)
            .map(|interval| interval.active)
    }
}

#[derive(Debug, Clone)]
struct ActivationDomain {
    fps: FrameRate,
    duration_frames: u32,
    end: SampleTime,
    end_seconds_upper: f64,
}

impl ActivationDomain {
    fn of(artifact: &SceneArtifact) -> Option<Self> {
        let composition = artifact.composition.as_ref()?;
        let fps = composition.frame_rate().ok()??;
        let duration_frames = composition.duration_frames(fps).ok()?;
        if duration_frames == 0 {
            return None;
        }
        let end = crate::time::sample_time_at_frame(i64::from(duration_frames), fps).ok()?;
        let end_seconds_upper = end.composition().as_f64().next_up();
        if !end_seconds_upper.is_finite() {
            return None;
        }
        Some(Self {
            fps,
            duration_frames,
            end,
            end_seconds_upper,
        })
    }

    fn projected_seconds(&self, ctx: &MotionContext) -> Option<f64> {
        let in_domain = ctx.fps == self.fps
            && ctx.duration_frames == self.duration_frames
            && ctx.local_frame < self.duration_frames
            && ctx.sample >= SampleTime::ZERO
            && ctx.sample < self.end;
        if !in_domain {
            return None;
        }
        let seconds = ctx.sample.composition().as_f64();
        (seconds.is_finite() && seconds >= 0.0 && seconds <= self.end_seconds_upper)
            .then_some(seconds)
    }

    fn certify(&self, gate: &ThresholdGate) -> Option<VisibilityCertificate> {
        let end = match gate.driver.input() {
            crate::ContextInput::LocalFrame => f64::from(self.duration_frames - 1),
            crate::ContextInput::CompositionSeconds => self.end_seconds_upper,
            _ => unreachable!("time functions only admit seconds or local frames"),
        };
        VisibilityCertificate::prove(gate, 0.0, end)
    }
}

impl ThresholdGate {
    fn of(artifact: &SceneArtifact, id: ExprId) -> Option<Self> {
        let Expr::Compare { op, lhs, rhs } = artifact.exprs.get(id.0 as usize)? else {
            return None;
        };
        let (driver, threshold, op) =
            if let Some(driver) = TimeFunction::compile(&artifact.exprs, *lhs) {
                (driver, *constant_number(&artifact.exprs, *rhs)?, *op)
            } else {
                (
                    TimeFunction::compile(&artifact.exprs, *rhs)?,
                    *constant_number(&artifact.exprs, *lhs)?,
                    swapped(*op),
                )
            };
        Some(Self {
            driver,
            threshold,
            op,
        })
    }

    fn active(
        &self,
        ctx: &MotionContext,
        drivers: &mut BTreeMap<Vec<u64>, f64>,
    ) -> Option<(bool, usize)> {
        let (value, cost) = if let Some(&value) = drivers.get(self.driver.structural_key()) {
            (value, 0)
        } else {
            let value = self.driver.evaluate(ctx);
            drivers.insert(self.driver.structural_key().clone(), value);
            (
                value,
                if self.driver.simple_driver().is_some() {
                    2
                } else {
                    self.driver.operation_count()
                },
            )
        };
        if !value.is_finite() {
            return None;
        }
        Some((
            match self.op {
                CompareOp::Eq => value == self.threshold,
                CompareOp::NotEq => value != self.threshold,
                CompareOp::Lt => value < self.threshold,
                CompareOp::Lte => value <= self.threshold,
                CompareOp::Gt => value > self.threshold,
                CompareOp::Gte => value >= self.threshold,
            },
            cost,
        ))
    }
}

fn constant_number(exprs: &[Expr], id: ExprId) -> Option<&f64> {
    let Expr::Const {
        value: MotionValue::Number(value),
    } = exprs.get(id.0 as usize)?
    else {
        return None;
    };
    value.is_finite().then_some(value)
}

#[derive(Debug, Clone)]
pub(super) struct InstanceRangeGate {
    driver: TimeFunction,
    op: CompareOp,
}

#[derive(Debug, Clone)]
pub(super) struct InstanceRowSelection {
    pub rows: Range<usize>,
    pub gate_evaluations: usize,
}

impl InstanceRangeGate {
    pub fn of(group: &InstanceGroup) -> Option<Self> {
        let Expr::Compare { op, lhs, rhs } =
            group.exprs.get(group.template.visibility?.0 as usize)?
        else {
            return None;
        };
        let (driver, op) = if indexed_operand(group, *rhs) {
            (TimeFunction::compile(&group.exprs, *lhs)?, *op)
        } else if indexed_operand(group, *lhs) {
            (TimeFunction::compile(&group.exprs, *rhs)?, swapped(*op))
        } else {
            return None;
        };
        if !matches!(
            op,
            CompareOp::Gte | CompareOp::Gt | CompareOp::Lt | CompareOp::Lte
        ) {
            return None;
        }
        Some(Self { driver, op })
    }

    pub fn select(&self, ctx: &MotionContext, count: usize) -> Option<InstanceRowSelection> {
        let time = self.driver.evaluate(ctx);
        if !time.is_finite() {
            return None;
        }
        let mut low = 0;
        let mut high = count;
        let mut comparisons = 0;
        while low < high {
            let middle = low + (high - low) / 2;
            comparisons += 1;
            let in_prefix = match self.op {
                CompareOp::Gte | CompareOp::Lt => time >= middle as f64,
                CompareOp::Gt | CompareOp::Lte => time > middle as f64,
                _ => unreachable!("only interval comparisons are admitted"),
            };
            if in_prefix {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let rows = match self.op {
            CompareOp::Gte | CompareOp::Gt => 0..low,
            CompareOp::Lt | CompareOp::Lte => low..count,
            _ => unreachable!("only interval comparisons are admitted"),
        };
        Some(InstanceRowSelection {
            rows,
            gate_evaluations: comparisons
                + if self.driver.simple_driver().is_some() {
                    2
                } else {
                    self.driver.operation_count()
                },
        })
    }
}

fn indexed_operand(group: &InstanceGroup, id: ExprId) -> bool {
    match group.exprs.get(id.0 as usize) {
        Some(Expr::InstanceIndex) => true,
        Some(Expr::InstanceField {
            column,
            value_type: crate::expr::ExprType::Number,
        }) => match group
            .columns
            .get(*column as usize)
            .map(|column| &column.values)
        {
            Some(InstanceColumnValues::Formula {
                expression: IndexFormula::Index,
                ..
            }) => true,
            Some(InstanceColumnValues::Numbers(values)) => values
                .iter()
                .enumerate()
                .all(|(index, value)| value.to_bits() == (index as f64).to_bits()),
            _ => false,
        },
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    node: usize,
    visibility: ExprId,
    gate: ThresholdGate,
    certificate: Option<VisibilityCertificate>,
    roots: Vec<ExprId>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ActivationPlan {
    base_roots: Vec<ExprId>,
    candidates: Vec<Candidate>,
    domain: Option<ActivationDomain>,
}

#[derive(Debug)]
pub(super) struct BaseEvaluation {
    pub values: Vec<MotionValue>,
    pub evaluated_expressions: usize,
    pub active_nodes: usize,
    pub inactive_nodes: Vec<bool>,
}

#[derive(Debug)]
struct Selection {
    order: Vec<ExprId>,
    inactive_visibility: Vec<ExprId>,
    active_visibility: Vec<ExprId>,
    inactive_nodes: Vec<bool>,
    evaluated_expressions: usize,
    active_nodes: usize,
}

impl ActivationPlan {
    pub fn new(
        artifact: &SceneArtifact,
        eval_plan: &EvalPlan,
        dependencies: &SceneDependencies,
    ) -> Self {
        // A camera can consume scene-wide values outside node expression roots. Composition and
        // per-unit siblings are included in the base plan through their own expression roots.
        if artifact.camera.is_some() {
            return Self::default();
        }

        let root = &artifact.nodes[artifact.root.0 as usize];
        if !matches!(root.kind, NodeKind::Group | NodeKind::Box) {
            return Self::default();
        }
        let mut base_roots = Vec::new();
        let mut candidates = Vec::new();
        let domain = ActivationDomain::of(artifact);
        for (at, node) in artifact.nodes.iter().enumerate() {
            let facts = dependencies
                .node(NodeId(at as u32))
                .expect("validated node is present in its dependency graph");
            let roots: Vec<_> = node
                .expr_refs_outside_per_unit()
                .into_iter()
                .map(|(_, id)| id)
                .collect();
            base_roots.extend(node.per_unit_expr_refs().into_iter().map(|(_, id)| id));
            let eligible = at != artifact.root.0 as usize
                && facts.ordinary_container_path
                && matches!(node.kind, NodeKind::Box)
                && node.children.start == node.children.end
                && node.space.is_none()
                && node.semantic.is_none()
                && !facts.geometry_referenced
                && !facts.paint_referenced
                && roots.iter().all(|id| {
                    !eval_plan.post_layout_dependent(*id) && !eval_plan.unit_dependent(*id)
                })
                && node.class_names.len() == 1
                && node.class_names[0] == "absolute"
                && node.class_conditions.is_empty()
                && node.styles.iter().all(|style| {
                    matches!(
                        style.property.as_str(),
                        "left" | "top" | "width" | "height" | "background-color" | "opacity"
                    ) && (matches!(style.value, StyleValue::Static { .. })
                        || style.property == "opacity")
                });
            if eligible
                && let Some(visibility) = node.visibility
                && let Some(gate) = ThresholdGate::of(artifact, visibility)
            {
                let certificate = domain.as_ref().and_then(|domain| domain.certify(&gate));
                candidates.push(Candidate {
                    node: at,
                    visibility,
                    gate,
                    certificate,
                    roots,
                });
            } else {
                base_roots.extend(roots);
            }
        }
        Self {
            base_roots,
            candidates,
            domain,
        }
    }

    fn select(
        &self,
        artifact: &SceneArtifact,
        plan: &EvalPlan,
        ctx: &MotionContext,
    ) -> Option<Selection> {
        if self.candidates.is_empty() {
            return None;
        }
        let mut roots = self.base_roots.clone();
        let mut inactive_visibility = Vec::new();
        let mut active_visibility = Vec::new();
        let mut inactive_nodes = vec![false; artifact.nodes.len()];
        let mut drivers = BTreeMap::new();
        let mut gate_cost = 0;
        let projected_seconds = self
            .domain
            .as_ref()
            .and_then(|domain| domain.projected_seconds(ctx));
        for candidate in &self.candidates {
            let certified = projected_seconds.and_then(|seconds| {
                let time = match candidate.gate.driver.input() {
                    crate::ContextInput::CompositionSeconds => seconds,
                    crate::ContextInput::LocalFrame => f64::from(ctx.local_frame),
                    _ => unreachable!("time functions only admit seconds or local frames"),
                };
                candidate.certificate.as_ref()?.at(time)
            });
            let (active, cost) = if let Some(active) = certified {
                (active, 0)
            } else {
                let (active, driver_cost) = candidate.gate.active(ctx, &mut drivers)?;
                (active, driver_cost + 1)
            };
            gate_cost += cost;
            if active {
                if certified == Some(true) {
                    roots.extend(
                        candidate
                            .roots
                            .iter()
                            .copied()
                            .filter(|id| *id != candidate.visibility),
                    );
                    active_visibility.push(candidate.visibility);
                } else {
                    roots.extend(&candidate.roots);
                }
            } else {
                inactive_visibility.push(candidate.visibility);
                inactive_nodes[candidate.node] = true;
            }
        }
        if inactive_visibility.is_empty() && active_visibility.is_empty() {
            return None;
        }
        let order = plan.schedule_dense(roots);
        let base_expressions = order
            .iter()
            .filter(|id| !plan.post_layout_dependent(**id) && !plan.unit_dependent(**id))
            .count();
        Some(Selection {
            // Each shared time driver is sampled once, then each predicate compared once. Active
            // candidates also run through the ordinary expression plan.
            evaluated_expressions: base_expressions + gate_cost,
            active_nodes: artifact.nodes.len() - inactive_visibility.len(),
            order,
            inactive_visibility,
            active_visibility,
            inactive_nodes,
        })
    }

    pub fn stats(
        &self,
        artifact: &SceneArtifact,
        plan: &EvalPlan,
        ctx: &MotionContext,
    ) -> (usize, usize) {
        self.select(artifact, plan, ctx).map_or_else(
            || (plan.base_eval_count(), artifact.nodes.len()),
            |selection| (selection.evaluated_expressions, selection.active_nodes),
        )
    }

    pub fn evaluate(
        &self,
        artifact: &SceneArtifact,
        plan: &EvalPlan,
        ctx: &MotionContext,
        props: &ResolvedProps,
        viewport: Option<(f64, f64)>,
        shared: Option<&mut SampleValueCache>,
    ) -> Result<Option<BaseEvaluation>, EvalError> {
        let Some(selection) = self.select(artifact, plan, ctx) else {
            return Ok(None);
        };
        let mut values = eval_roots_planned_dense_cached(
            artifact,
            plan,
            EvalInputs {
                ctx,
                props,
                unit: None,
                viewport,
            },
            &selection.order,
            shared,
        )?;
        for id in &selection.inactive_visibility {
            values[id.0 as usize] = MotionValue::Bool(false);
        }
        for id in &selection.active_visibility {
            values[id.0 as usize] = MotionValue::Bool(true);
        }
        Ok(Some(BaseEvaluation {
            values,
            evaluated_expressions: selection.evaluated_expressions,
            active_nodes: selection.active_nodes,
            inactive_nodes: selection.inactive_nodes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextInput;

    #[test]
    fn local_proof_keeps_the_crossing_unresolved() {
        let exprs = vec![
            Expr::Context {
                input: ContextInput::CompositionSeconds,
            },
            Expr::Const {
                value: MotionValue::Number(100.0),
            },
            Expr::Mul {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
        ];
        let gate = ThresholdGate {
            driver: TimeFunction::compile(&exprs, ExprId(2)).unwrap(),
            threshold: 2.0,
            op: CompareOp::Gte,
        };
        let proof = VisibilityCertificate::prove(&gate, 0.0, 2.0).unwrap();
        assert_eq!(proof.at(0.0), Some(false));
        assert_eq!(proof.at(1.0), Some(true));
        assert_eq!(proof.at(0.02), None);
    }
}
