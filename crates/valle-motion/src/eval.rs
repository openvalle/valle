//! Per-frame evaluator for the Motion JSX Scene contract.
//!
//! The evaluator consumes only the closed [`MotionContext`], resolved props, and the expression
//! arena carried by [`SceneArtifact`]. It has no clock, JavaScript runtime, layout engine, or
//! renderer dependency. Expressions reference earlier entries only, so a single forward pass is
//! deterministic and evaluates shared subexpressions exactly once.

use std::collections::{BTreeMap, BTreeSet};

use valle_draw::{Point, Rect, Rgba, Vec2};

use crate::context::{HoldContext, MotionContext, PhaseContext, PhaseKind};
use crate::geometry::{GeometryError, PathData};
pub use crate::value::css_token;
use crate::value::{Angle, AngleUnit, Length, Length2, MotionEasing, MotionValue};

use super::{
    CompareOp, ContextInput, ControlType, ControlsSchema, CueField, Expr, ExprId, Extrapolation,
    GeometryField, InterpolateStop, MAX_TEMPLATE_OUTPUT_BYTES, ResolvedSignals, SceneArtifact,
    TemplatePart,
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
    pub signals: &'a ResolvedSignals,
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
    MissingCue {
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
            EvalError::MissingCue { name } => write!(f, "cue signal `{name}` is not resolved"),
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

/// Evaluate the complete expression arena in one forward pass.
pub fn eval_all(
    artifact: &SceneArtifact,
    inputs: EvalInputs<'_>,
) -> Result<Vec<MotionValue>, EvalError> {
    let unit_dependent = crate::expr::unit_dependent(&artifact.exprs);
    let post_layout_dependent = crate::expr::post_layout_dependent(&artifact.exprs);
    let mut values = Vec::with_capacity(artifact.exprs.len());
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if post_layout_dependent[at] {
            // Skip post-layout expressions in the base pass; admission prevents layout consumers
            // from reading their placeholders.
            values.push(MotionValue::Number(0.0));
            continue;
        }
        if unit_dependent[at] && inputs.unit.is_none() {
            // Skip unit-dependent expressions until a unit exists; admission prevents other
            // consumers from reading their placeholders.
            values.push(MotionValue::Number(0.0));
            continue;
        }
        let value = eval_one(at, expr, &values, inputs, None, None)?;
        values.push(finite(at, "evaluation", value)?);
    }
    Ok(values)
}

/// Evaluate only the listed expression ids plus their ancestors. Missing slots stay `None`.
pub fn eval_slice(
    artifact: &SceneArtifact,
    inputs: EvalInputs<'_>,
    live: &BTreeSet<ExprId>,
) -> Result<Vec<Option<MotionValue>>, EvalError> {
    let unit_dependent = crate::expr::unit_dependent(&artifact.exprs);
    let post_layout_dependent = crate::expr::post_layout_dependent(&artifact.exprs);
    let mut done: Vec<MotionValue> = Vec::with_capacity(artifact.exprs.len());
    let mut values = vec![None; artifact.exprs.len()];
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if !live.contains(&ExprId(at as u32)) {
            done.push(MotionValue::Number(0.0));
            continue;
        }
        if post_layout_dependent[at] {
            let value = MotionValue::Number(0.0);
            done.push(value.clone());
            values[at] = Some(value);
            continue;
        }
        if unit_dependent[at] && inputs.unit.is_none() {
            let value = MotionValue::Number(0.0);
            done.push(value.clone());
            values[at] = Some(value);
            continue;
        }
        let value = finite(
            at,
            "evaluation",
            eval_one(at, expr, &done, inputs, None, None)?,
        )?;
        done.push(value.clone());
        values[at] = Some(value);
    }
    Ok(values)
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
    let unit_dependent = crate::expr::unit_dependent(&artifact.exprs);
    let inputs = EvalInputs {
        unit: Some(unit),
        ..inputs
    };
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if !unit_dependent[at] {
            continue;
        }
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
    let bounds_dependent = crate::expr::bounds_dependent(&artifact.exprs);
    let projection_dependent = crate::expr::projection_dependent(&artifact.exprs);
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if !bounds_dependent[at] || projection_dependent[at] {
            continue;
        }
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
    let dependent = crate::expr::post_layout_dependent(&artifact.exprs);
    let mut values = base.to_vec();
    values.resize(artifact.exprs.len(), MotionValue::Number(0.0));
    for (at, expr) in artifact.exprs.iter().enumerate() {
        if !dependent[at] {
            continue;
        }
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
        Expr::Context { .. }
        | Expr::Prop { .. }
        | Expr::Cue { .. }
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
        | Expr::PathOffset { .. }
        | Expr::PathMorph { .. }
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
        | Expr::Interpolate { .. } => false,
    }
}

/// Dummy folding context required by the evaluator signature; runtime-dependent branches are never
/// evaluated during folding.
fn folding_context() -> MotionContext {
    let phase = PhaseContext {
        active: false,
        frame: 0,
        elapsed_frames: 0,
        duration_frames: 0,
        progress: 0.0,
    };
    MotionContext {
        local_frame: 0,
        sample: valle_timeline::internal::SampleTime::ZERO,
        progress: 0.0,
        duration_frames: 0,
        fps: valle_timeline::FrameRate::new(1, 1).expect("1/1 is a valid frame rate"),
        current_phase: PhaseKind::Hold,
        enter: phase,
        hold: HoldContext {
            active: false,
            frame: 0,
            elapsed_frames: 0,
            duration_frames: 0,
            progress: 0.0,
            iteration: 0,
            cycle_frame: 0,
            cycle_progress: 0.0,
        },
        exit: phase,
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
    let empty_signals = crate::ResolvedSignals::default();
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
                    signals: &empty_signals,
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
    let child = |id: ExprId| {
        done.get(id.0 as usize).ok_or(EvalError::BadExpr {
            at,
            referenced: id.0 as usize,
        })
    };
    match expr {
        Expr::Const { value } => Ok(value.clone()),
        Expr::Context { input } => context_value(*input, inputs.ctx, inputs.unit, inputs.viewport)
            .ok_or_else(|| match input {
                ContextInput::ViewportWidth | ContextInput::ViewportHeight => {
                    EvalError::ViewportUnavailable { at }
                }
                _ => EvalError::UnitOutsidePerUnit { at },
            }),
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
        Expr::Cue { name, field } => {
            let cue = inputs
                .signals
                .cue(name)
                .ok_or_else(|| EvalError::MissingCue { name: name.clone() })?;
            Ok(match field {
                CueField::Active => MotionValue::Bool(cue.active),
                CueField::Progress => MotionValue::Number(cue.progress),
                CueField::Enter => MotionValue::Number(cue.enter),
                CueField::Hold => MotionValue::Number(cue.hold),
                CueField::Exit => MotionValue::Number(cue.exit),
                CueField::LocalFrame => MotionValue::Number(f64::from(cue.local_frame)),
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
        Expr::PathOffset { path, distance } => as_path(at, "offsetPath", child(*path)?)?
            .offset_path(as_number(at, "offsetPath", child(*distance)?)?)
            .map(MotionValue::PathData)
            .map_err(|reason| EvalError::BadGeometry { at, reason }),
        Expr::PathMorph { from, to, progress } => {
            let from = as_path(at, "morphPath", child(*from)?)?;
            let to = as_path(at, "morphPath", child(*to)?)?;
            from.morph(to, as_number(at, "morphPath", child(*progress)?)?)
                .map(MotionValue::PathData)
                .map_err(|reason| EvalError::BadGeometry { at, reason })
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
        } => {
            let frames = as_number(at, "spring", child(*elapsed_frames)?)?;
            let fps = crate::frame_rate_as_f64(inputs.ctx.fps);
            if !fps.is_finite() || fps <= 0.0 {
                return Err(EvalError::TypeMismatch { at, op: "spring" });
            }
            Ok(MotionValue::Number(crate::spring::spring_at(
                frames / fps,
                crate::spring::SpringParams {
                    mass: *mass,
                    stiffness: *stiffness,
                    damping: *damping,
                },
            )))
        }
        Expr::Interpolate {
            input,
            stops,
            easings,
            extrapolate_left,
            extrapolate_right,
        } => interpolate(
            at,
            as_number(at, "interpolate", child(*input)?)?,
            stops,
            easings,
            *extrapolate_left,
            *extrapolate_right,
        ),
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
    let phase = |kind: PhaseKind| match kind {
        PhaseKind::Enter => (
            ctx.enter.active,
            ctx.enter.frame,
            ctx.enter.duration_frames,
            ctx.enter.progress,
        ),
        PhaseKind::Hold => (
            ctx.hold.active,
            ctx.hold.frame,
            ctx.hold.duration_frames,
            ctx.hold.progress,
        ),
        PhaseKind::Exit => (
            ctx.exit.active,
            ctx.exit.frame,
            ctx.exit.duration_frames,
            ctx.exit.progress,
        ),
    };
    let number = |value| MotionValue::Number(f64::from(value));
    Some(match input {
        ContextInput::LocalFrame => number(ctx.local_frame),
        ContextInput::LocalProgress => MotionValue::Number(ctx.progress),
        ContextInput::CompositionSeconds => MotionValue::Number(ctx.sample.composition().as_f64()),
        ContextInput::DurationFrames => number(ctx.duration_frames),
        ContextInput::FpsNum => MotionValue::Number(ctx.fps.numerator() as f64),
        ContextInput::FpsDen => number(ctx.fps.denominator()),
        ContextInput::PhaseFrame { phase: kind } => number(phase(kind).1),
        ContextInput::PhaseDurationFrames { phase: kind } => number(phase(kind).2),
        ContextInput::PhaseProgress { phase: kind } => MotionValue::Number(phase(kind).3),
        ContextInput::PhaseElapsedFrames { phase: kind } => {
            MotionValue::Number(f64::from(match kind {
                PhaseKind::Enter => ctx.enter.elapsed_frames,
                PhaseKind::Hold => ctx.hold.elapsed_frames,
                PhaseKind::Exit => ctx.exit.elapsed_frames,
            }))
        }
        ContextInput::PhaseActive { phase: kind } => MotionValue::Bool(phase(kind).0),
        ContextInput::HoldIteration => number(ctx.hold.iteration),
        ContextInput::HoldCycleFrame => number(ctx.hold.cycle_frame),
        ContextInput::HoldCycleProgress => MotionValue::Number(ctx.hold.cycle_progress),
        ContextInput::UnitIndex => number(unit?.index),
        ContextInput::UnitCount => number(unit?.count),
        ContextInput::UnitStart => number(unit?.start),
        ContextInput::UnitEnd => number(unit?.end),
        ContextInput::ViewportWidth => MotionValue::Number(viewport?.0),
        ContextInput::ViewportHeight => MotionValue::Number(viewport?.1),
    })
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
    left: Extrapolation,
    right: Extrapolation,
) -> Result<MotionValue, EvalError> {
    if stops.len() < 2 || (!easings.is_empty() && easings.len() + 1 != stops.len()) {
        return Err(EvalError::BadInterpolate { at });
    }
    let first = &stops[0];
    let last = &stops[stops.len() - 1];
    if input <= first.input {
        return match left {
            Extrapolation::Clamp => Ok(first.output.clone()),
            Extrapolation::Extend => interpolate_segment(at, input, &stops[0], &stops[1], None),
        };
    }
    if input >= last.input {
        return match right {
            Extrapolation::Clamp => Ok(last.output.clone()),
            Extrapolation::Extend => {
                interpolate_segment(at, input, &stops[stops.len() - 2], last, None)
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
    )
}

fn interpolate_segment(
    at: usize,
    input: f64,
    lhs: &InterpolateStop,
    rhs: &InterpolateStop,
    easing: Option<MotionEasing>,
) -> Result<MotionValue, EvalError> {
    let span = rhs.input - lhs.input;
    if span <= 0.0 || !span.is_finite() {
        return Err(EvalError::BadInterpolate { at });
    }
    let raw = (input - lhs.input) / span;
    let t = easing.map_or(raw, |easing| ease(easing, raw));
    lerp_value(at, &lhs.output, &rhs.output, t)
}

fn ease(easing: MotionEasing, t: f64) -> f64 {
    easing.evaluate(t)
}

fn lerp_value(
    at: usize,
    lhs: &MotionValue,
    rhs: &MotionValue,
    t: f64,
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
        (MotionValue::Color(a), MotionValue::Color(b)) => {
            let (alpha_a, alpha_b) = (f64::from(a.a) / 255.0, f64::from(b.a) / 255.0);
            let alpha = lerp(alpha_a, alpha_b).clamp(0.0, 1.0);
            let channel = |a: u8, b: u8| {
                if alpha <= 0.0 {
                    0
                } else {
                    (lerp(f64::from(a) * alpha_a, f64::from(b) * alpha_b) / alpha)
                        .round()
                        .clamp(0.0, 255.0) as u8
                }
            };
            MotionValue::Color(Rgba::new(
                channel(a.r, b.r),
                channel(a.g, b.g),
                channel(a.b, b.b),
                (alpha * 255.0).round() as u8,
            ))
        }
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
            let MotionValue::Number(value) =
                interpolate_segment(0, t, &stops[0], &stops[1], Some(easing)).unwrap()
            else {
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
    use crate::{
        ARTIFACT_FORMAT_VERSION, CapabilitySet, ChildRange, FrameControl, NodeId, NodeKind,
        OptionalFrameControl, SceneNode, TimingControls,
    };
    use valle_timeline::FrameRate;

    fn controls() -> ControlsSchema {
        ControlsSchema {
            props: BTreeMap::new(),
            data: BTreeMap::new(),
            timing: TimingControls {
                enter_frames: FrameControl {
                    default: 2,
                    min: 0,
                    max: None,
                },
                hold_cycle_frames: OptionalFrameControl {
                    default: Some(2),
                    min: 1,
                    max: None,
                },
                exit_frames: FrameControl {
                    default: 2,
                    min: 0,
                    max: None,
                },
            },
            cues: BTreeMap::new(),
            assets: BTreeMap::new(),
            camera: Default::default(),
        }
    }

    fn artifact(exprs: Vec<Expr>) -> SceneArtifact {
        SceneArtifact {
            camera: None,
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::base(),
            component: "eval-test".into(),
            controls: controls(),
            resource_refs: vec![],
            exprs,
            nodes: vec![SceneNode {
                key: "root".into(),
                kind: NodeKind::Group,
                space: None,
                class_names: vec![],
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
        let layout = crate::phase_windows(&controls().phase_spec(), 8);
        crate::motion_context_at(frame, &layout, FrameRate::new(30, 1).unwrap()).unwrap()
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
        let signals = crate::ResolvedSignals::default();
        let inputs = EvalInputs {
            ctx: &ctx,
            props: &props,
            signals: &signals,
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
        let signals = crate::ResolvedSignals::default();
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
                    signals: &signals,
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
                input: ContextInput::PhaseFrame {
                    phase: PhaseKind::Enter,
                },
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
                signals: &ResolvedSignals::default(),
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
                signals: &ResolvedSignals::default(),
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
                signals: &ResolvedSignals::default(),
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
                signals: &ResolvedSignals::default(),
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
                MotionValue::Color(Rgba::new(255, 0, 0, 255)),
                MotionValue::Color(Rgba::new(0, 0, 255, 255)),
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
                signals: &ResolvedSignals::default(),
                unit: None,
                viewport: None,
            },
        )
        .unwrap();
        assert_eq!(values[1], MotionValue::Number(5.0));
        assert_eq!(values[2], MotionValue::Length(Length::px(5.0)));
        assert_eq!(values[3], MotionValue::Length2(Length2::px(5.0, 15.0)));
        assert_eq!(values[4], MotionValue::Angle(Angle::deg(45.0)));
        assert_eq!(values[5], MotionValue::Color(Rgba::new(128, 0, 128, 255)));
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
