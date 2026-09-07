use serde::{Deserialize, Serialize};

use crate::context::PhaseKind;
use crate::geometry::{
    GeometryEvalPolicy, MAX_FRAME_GEOMETRY_POINTS, MAX_GEOMETRY_TRAJECTORY_FRAMES, PathData,
};
use crate::value::{MotionEasing, MotionValue};
use valle_draw::PathVerb;

use super::artifact::ValidationError;
use super::controls::{ControlType, ControlsSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct ExprId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum ContextInput {
    LocalFrame,
    /// Continuous clip progress in `[0, 1)`. Glass track-affecting time base.
    LocalProgress,
    /// Clip-local composition seconds as a finite f64 projection of the exact sample identity.
    CompositionSeconds,
    DurationFrames,
    FpsNum,
    FpsDen,
    PhaseFrame {
        phase: PhaseKind,
    },
    PhaseDurationFrames {
        phase: PhaseKind,
    },
    PhaseProgress {
        phase: PhaseKind,
    },
    /// Unclamped frames since phase start for spring timing, negative before the phase and
    /// increasing after it ends.
    PhaseElapsedFrames {
        phase: PhaseKind,
    },
    PhaseActive {
        phase: PhaseKind,
    },
    HoldIteration,
    HoldCycleFrame,
    HoldCycleProgress,
    /// Current text-unit index, valid only in per-unit Text styles. Admission rejects use
    /// elsewhere.
    UnitIndex,
    /// Total unit count for this text node.
    UnitCount,
    /// Unit start byte relative to its own text node.
    UnitStart,
    /// Unit end byte relative to its own text node.
    UnitEnd,
    /// Render-request viewport width in pixels. Unlike post-layout node bounds, viewport dimensions
    /// are available before layout and may drive layout properties.
    ViewportWidth,
    /// Viewport height in pixels (`ctx.viewport.height`).
    ViewportHeight,
}

impl ContextInput {
    /// Whether this value requires a per-unit evaluation context.
    pub fn is_unit(self) -> bool {
        matches!(
            self,
            ContextInput::UnitIndex
                | ContextInput::UnitCount
                | ContextInput::UnitStart
                | ContextInput::UnitEnd
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CueField {
    Active,
    Progress,
    Enter,
    Hold,
    Exit,
    LocalFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeometryField {
    X,
    Y,
    Width,
    Height,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Extrapolation {
    Clamp,
    Extend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MathUnaryOp {
    Sqrt,
    Exp,
    Sin,
    Cos,
    Tan,
    Floor,
    Ceil,
    Round,
    Trunc,
    Fract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MathBinaryOp {
    Atan2,
    Pow,
    Mod,
    Remainder,
    PingPong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum NumberFormat {
    Number { decimals: u8, grouping: bool },
    Percent { decimals: u8, grouping: bool },
    Pad { width: u8 },
}

impl NumberFormat {
    /// One bounded contract shared by prepare-time folding, frame-time evaluation and Artifact
    /// validation. Keeping the limits here prevents a static call from baking a value that the
    /// equivalent dynamic expression would reject.
    pub fn validate(self) -> Result<(), &'static str> {
        match self {
            Self::Number { decimals, .. } | Self::Percent { decimals, .. } if decimals <= 12 => {
                Ok(())
            }
            Self::Pad { width } if (1..=64).contains(&width) => Ok(()),
            _ => Err("number format exceeds decimals 0..=12 or width 1..=64"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterpolateStop {
    pub input: f64,
    pub output: MotionValue,
}

pub const MAX_TEMPLATE_PARTS: usize = 64;
pub const MAX_TEMPLATE_STATIC_BYTES: usize = 8 * 1024;
pub const MAX_TEMPLATE_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum TemplatePart {
    Text { value: String },
    Expr { expr: ExprId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum Expr {
    Const {
        value: MotionValue,
    },
    Context {
        input: ContextInput,
    },
    Prop {
        name: String,
    },
    Cue {
        name: String,
        field: CueField,
    },
    MakePoint {
        x: ExprId,
        y: ExprId,
    },
    MakeRect {
        x: ExprId,
        y: ExprId,
        width: ExprId,
        height: ExprId,
    },
    PathLine {
        points: Vec<ExprId>,
    },
    /// Fixed-topology path template. Every entry in `points` is a typed Point expression; verbs
    /// are frozen at compile time so frame evaluation can only move coordinates, never topology.
    PathTemplate {
        verbs: Vec<PathVerb>,
        points: Vec<ExprId>,
    },
    PathCubic {
        from: ExprId,
        control_1: ExprId,
        control_2: ExprId,
        to: ExprId,
    },
    PathArc {
        center: ExprId,
        radius: ExprId,
        start_angle: ExprId,
        end_angle: ExprId,
    },
    PathArea {
        path: ExprId,
        baseline: ExprId,
    },
    PathOffset {
        path: ExprId,
        distance: ExprId,
    },
    PathMorph {
        from: ExprId,
        to: ExprId,
        progress: ExprId,
    },
    PathPointAt {
        path: ExprId,
        progress: ExprId,
    },
    PathTangentAt {
        path: ExprId,
        progress: ExprId,
    },
    PathAngleAt {
        path: ExprId,
        progress: ExprId,
    },
    PathLength {
        path: ExprId,
    },
    PathTrajectory {
        frames: Vec<PathData>,
        frame: ExprId,
    },
    GeometryField {
        input: ExprId,
        field: GeometryField,
    },
    ToLength2 {
        input: ExprId,
    },
    Add {
        lhs: ExprId,
        rhs: ExprId,
    },
    Sub {
        lhs: ExprId,
        rhs: ExprId,
    },
    Mul {
        lhs: ExprId,
        rhs: ExprId,
    },
    Div {
        lhs: ExprId,
        rhs: ExprId,
    },
    Neg {
        input: ExprId,
    },
    Compare {
        op: CompareOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    Select {
        condition: ExprId,
        when_true: ExprId,
        when_false: ExprId,
    },
    Template {
        parts: Vec<TemplatePart>,
    },
    MathUnary {
        op: MathUnaryOp,
        input: ExprId,
    },
    MathBinary {
        op: MathBinaryOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    Noise1D {
        seed: u64,
        x: ExprId,
    },
    Noise2D {
        seed: u64,
        x: ExprId,
        y: ExprId,
    },
    FormatNumber {
        input: ExprId,
        format: NumberFormat,
    },
    Interpolate {
        input: ExprId,
        stops: Vec<InterpolateStop>,
        easings: Vec<MotionEasing>,
        extrapolate_left: Extrapolation,
        extrapolate_right: Extrapolation,
    },
    /// Closed-form damped harmonic oscillator, normalized from 0 to 1. Physical parameters are
    /// compile-time constants; presets and explicit parameters are mutually exclusive. The frame
    /// rate comes from `MotionContext`. Use arithmetic or interpolation to map the output to
    /// another range.
    Spring {
        /// Unclamped phase-relative frame, usually `ctx.<phase>.elapsedFrames`.
        elapsed_frames: ExprId,
        mass: f64,
        stiffness: f64,
        damping: f64,
    },
    /// Post-layout node border box. Anchors and connectors compose this with existing geometry and
    /// arithmetic expressions. It is unavailable during base evaluation; admission rejects its use
    /// in layout-affecting slots to prevent layout cycles.
    NodeBounds {
        key: String,
    },
    /// A named Scene3D anchor projected into the Motion scene's post-layout world coordinates.
    /// Both address segments are compile-time constants; evaluation happens only after the
    /// Scene3D content box and current frame state are known.
    Project3D {
        scene_key: String,
        anchor_key: String,
    },
}

/// Compute transitive per-unit dependencies in one pass over the topologically ordered arena. A
/// parent depends on unit context whenever any child does, so slot validation only needs to inspect
/// the directly referenced expression.
pub fn unit_dependent(exprs: &[Expr]) -> Vec<bool> {
    let mut flags = vec![false; exprs.len()];
    for (at, expr) in exprs.iter().enumerate() {
        let direct = matches!(expr, Expr::Context { input } if input.is_unit());
        flags[at] = direct
            || expr
                .children()
                .into_iter()
                .any(|id| flags.get(id.0 as usize).copied().unwrap_or(false));
    }
    flags
}

/// Compute transitive dependencies on [`Expr::NodeBounds`] in one pass over the topologically
/// ordered arena. These expressions have no value before layout.
pub fn bounds_dependent(exprs: &[Expr]) -> Vec<bool> {
    let mut flags = vec![false; exprs.len()];
    for (at, expr) in exprs.iter().enumerate() {
        let direct = matches!(expr, Expr::NodeBounds { .. });
        flags[at] = direct
            || expr
                .children()
                .into_iter()
                .any(|id| flags.get(id.0 as usize).copied().unwrap_or(false));
    }
    flags
}

/// Expressions that transitively depend on a Scene3D anchor projection.
pub fn projection_dependent(exprs: &[Expr]) -> Vec<bool> {
    let mut flags = vec![false; exprs.len()];
    for (at, expr) in exprs.iter().enumerate() {
        let direct = matches!(expr, Expr::Project3D { .. });
        flags[at] = direct
            || expr
                .children()
                .into_iter()
                .any(|id| flags.get(id.0 as usize).copied().unwrap_or(false));
    }
    flags
}

/// All values unavailable in the base pre-layout evaluation pass.
pub fn post_layout_dependent(exprs: &[Expr]) -> Vec<bool> {
    let bounds = bounds_dependent(exprs);
    let projected = projection_dependent(exprs);
    bounds
        .into_iter()
        .zip(projected)
        .map(|(bounds, projected)| bounds || projected)
        .collect()
}

impl Expr {
    /// Direct child expressions. Keep this match exhaustive so every new variant participates in
    /// dependency analysis.
    pub fn children(&self) -> Vec<ExprId> {
        match self {
            Expr::Const { .. }
            | Expr::Context { .. }
            | Expr::Prop { .. }
            | Expr::Cue { .. }
            | Expr::NodeBounds { .. }
            | Expr::Project3D { .. } => Vec::new(),
            Expr::MakePoint { x, y } => vec![*x, *y],
            Expr::MakeRect {
                x,
                y,
                width,
                height,
            } => vec![*x, *y, *width, *height],
            Expr::PathLine { points } => points.clone(),
            Expr::PathTemplate { points, .. } => points.clone(),
            Expr::PathCubic {
                from,
                control_1,
                control_2,
                to,
            } => vec![*from, *control_1, *control_2, *to],
            Expr::PathArc {
                center,
                radius,
                start_angle,
                end_angle,
            } => vec![*center, *radius, *start_angle, *end_angle],
            Expr::PathArea { path, baseline } => vec![*path, *baseline],
            Expr::PathOffset { path, distance } => vec![*path, *distance],
            Expr::PathMorph { from, to, progress } => vec![*from, *to, *progress],
            Expr::PathPointAt { path, progress }
            | Expr::PathTangentAt { path, progress }
            | Expr::PathAngleAt { path, progress } => vec![*path, *progress],
            Expr::PathLength { path } => vec![*path],
            Expr::PathTrajectory { frame, .. } => vec![*frame],
            Expr::GeometryField { input, .. }
            | Expr::ToLength2 { input }
            | Expr::Neg { input }
            | Expr::MathUnary { input, .. }
            | Expr::Noise1D { x: input, .. }
            | Expr::FormatNumber { input, .. }
            | Expr::Interpolate { input, .. } => vec![*input],
            Expr::MathBinary { lhs, rhs, .. } => vec![*lhs, *rhs],
            Expr::Noise2D { x, y, .. } => vec![*x, *y],
            Expr::Spring { elapsed_frames, .. } => vec![*elapsed_frames],
            Expr::Add { lhs, rhs }
            | Expr::Sub { lhs, rhs }
            | Expr::Mul { lhs, rhs }
            | Expr::Div { lhs, rhs }
            | Expr::Compare { lhs, rhs, .. } => vec![*lhs, *rhs],
            Expr::Select {
                condition,
                when_true,
                when_false,
            } => vec![*condition, *when_true, *when_false],
            Expr::Template { parts } => parts
                .iter()
                .filter_map(|part| match part {
                    TemplatePart::Expr { expr } => Some(*expr),
                    TemplatePart::Text { .. } => None,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprType {
    Number,
    Bool,
    String,
    Enum,
    Length,
    Length2,
    Angle,
    Color,
    Point,
    Vec2,
    Rect,
    PathData,
}

impl ExprType {
    pub(crate) fn of_value(value: &MotionValue) -> Self {
        match value {
            MotionValue::Number(_) => ExprType::Number,
            MotionValue::Bool(_) => ExprType::Bool,
            MotionValue::Str(_) => ExprType::String,
            MotionValue::Enum(_) => ExprType::Enum,
            MotionValue::Length(_) => ExprType::Length,
            MotionValue::Length2(_) => ExprType::Length2,
            MotionValue::Angle(_) => ExprType::Angle,
            MotionValue::Color(_) => ExprType::Color,
            MotionValue::Point(_) => ExprType::Point,
            MotionValue::Vec2(_) => ExprType::Vec2,
            MotionValue::Rect(_) => ExprType::Rect,
            MotionValue::PathData(_) => ExprType::PathData,
        }
    }

    fn of_control(control: &ControlType) -> Self {
        match control {
            ControlType::Number { .. } => ExprType::Number,
            ControlType::String | ControlType::NodeTarget => ExprType::String,
            ControlType::Bool => ExprType::Bool,
            ControlType::Color => ExprType::Color,
            ControlType::Length => ExprType::Length,
            ControlType::Angle => ExprType::Angle,
            ControlType::Point => ExprType::Point,
            ControlType::Rect => ExprType::Rect,
            ControlType::PathData => ExprType::PathData,
            ControlType::Select { .. } => ExprType::Enum,
        }
    }

    fn is_continuous(self) -> bool {
        matches!(
            self,
            ExprType::Number
                | ExprType::Length
                | ExprType::Length2
                | ExprType::Angle
                | ExprType::Color
                | ExprType::Point
                | ExprType::Vec2
                | ExprType::Rect
                | ExprType::PathData
        )
    }
}

/// Expression types and subtree validity from admission. Constant folding must use these results:
/// evaluation alone accepts some expressions that admission rejects, and folding them would hide
/// the original violation.
pub(crate) struct ExprAdmission {
    pub types: Vec<Option<ExprType>>,
    /// Whether this expression and every referenced descendant passed admission.
    pub admitted: Vec<bool>,
}

pub(crate) fn validate_exprs(
    exprs: &[Expr],
    controls: &ControlsSchema,
    errors: &mut Vec<ValidationError>,
) -> ExprAdmission {
    let mut types = Vec::with_capacity(exprs.len());
    let mut admitted: Vec<bool> = Vec::with_capacity(exprs.len());
    for (index, expr) in exprs.iter().enumerate() {
        let errors_before = errors.len();
        let path = format!("/exprs/{index}");
        let mut child = |id: ExprId| -> Option<ExprType> {
            if id.0 as usize >= index {
                errors.push(ValidationError::new(
                    format!("{path}/expr"),
                    "expressions may only reference earlier expressions",
                ));
                None
            } else {
                types[id.0 as usize]
            }
        };
        let ty = match expr {
            Expr::Const { value } => {
                if !value.is_finite() {
                    errors.push(ValidationError::new(
                        format!("{path}/value"),
                        "constant must be finite",
                    ));
                }
                Some(ExprType::of_value(value))
            }
            Expr::Context { input } => Some(match input {
                ContextInput::PhaseActive { .. } => ExprType::Bool,
                _ => ExprType::Number,
            }),
            Expr::Prop { name } => match controls.props.get(name) {
                Some(control) => Some(ExprType::of_control(&control.control)),
                None => {
                    errors.push(ValidationError::new(
                        format!("{path}/name"),
                        "unknown props control",
                    ));
                    None
                }
            },
            Expr::Cue { name, field } => {
                if controls.cues.contains_key(name) {
                    Some(if *field == CueField::Active {
                        ExprType::Bool
                    } else {
                        ExprType::Number
                    })
                } else {
                    errors.push(ValidationError::new(
                        format!("{path}/name"),
                        "unknown cue control",
                    ));
                    None
                }
            }
            Expr::Project3D {
                scene_key,
                anchor_key,
            } => {
                if scene_key.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/sceneKey"),
                        "project3d scene key is empty",
                    ));
                }
                if anchor_key.is_empty() {
                    errors.push(ValidationError::new(
                        format!("{path}/anchorKey"),
                        "project3d anchor key is empty",
                    ));
                }
                Some(ExprType::Point)
            }
            Expr::MakePoint { x, y } => {
                if child(*x) != Some(ExprType::Number) || child(*y) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        path,
                        "point coordinates must be numbers",
                    ));
                    None
                } else {
                    Some(ExprType::Point)
                }
            }
            Expr::MakeRect {
                x,
                y,
                width,
                height,
            } => {
                if [*x, *y, *width, *height]
                    .into_iter()
                    .any(|input| child(input) != Some(ExprType::Number))
                {
                    errors.push(ValidationError::new(
                        path,
                        "rect coordinates and size must be numbers",
                    ));
                    None
                } else {
                    Some(ExprType::Rect)
                }
            }
            Expr::PathLine { points } => {
                if points.len() < 2
                    || points.len() > MAX_FRAME_GEOMETRY_POINTS
                    || points
                        .iter()
                        .any(|point| child(*point) != Some(ExprType::Point))
                {
                    errors.push(ValidationError::new(
                        path,
                        "line requires 2..=2048 Point values",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathTemplate { verbs, points } => {
                let topology_valid = PathData::new(
                    verbs.clone(),
                    vec![valle_draw::Point::default(); points.len()],
                )
                .is_ok();
                if verbs.is_empty()
                    || points.len() > MAX_FRAME_GEOMETRY_POINTS
                    || !topology_valid
                    || points
                        .iter()
                        .any(|point| child(*point) != Some(ExprType::Point))
                {
                    errors.push(ValidationError::new(
                        path,
                        "path template requires valid fixed M/L/Q/C/Z topology and bounded Point values",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathCubic {
                from,
                control_1,
                control_2,
                to,
            } => {
                if [*from, *control_1, *control_2, *to]
                    .into_iter()
                    .any(|point| child(point) != Some(ExprType::Point))
                {
                    errors.push(ValidationError::new(
                        path,
                        "cubic requires four Point values",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathArc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                if child(*center) != Some(ExprType::Point)
                    || [*radius, *start_angle, *end_angle]
                        .into_iter()
                        .any(|number| child(number) != Some(ExprType::Number))
                {
                    errors.push(ValidationError::new(
                        path,
                        "arc requires Point center and numeric radius/start/end angles",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathArea {
                path: input,
                baseline,
            } => {
                if child(*input) != Some(ExprType::PathData)
                    || child(*baseline) != Some(ExprType::Number)
                    || path_flattened_point_bound(exprs, *input).saturating_add(2)
                        > MAX_FRAME_GEOMETRY_POINTS
                {
                    errors.push(ValidationError::new(
                        path,
                        "area requires a bounded PathData and numeric baseline within the per-frame geometry budget",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathOffset {
                path: input,
                distance,
            } => {
                if child(*input) != Some(ExprType::PathData)
                    || child(*distance) != Some(ExprType::Number)
                    || path_flattened_point_bound(exprs, *input) > MAX_FRAME_GEOMETRY_POINTS
                {
                    errors.push(ValidationError::new(
                        path,
                        "offsetPath requires bounded PathData and numeric distance within the per-frame geometry budget",
                    ));
                    None
                } else {
                    Some(ExprType::PathData)
                }
            }
            Expr::PathMorph { from, to, progress } => {
                if child(*from) != Some(ExprType::PathData)
                    || child(*to) != Some(ExprType::PathData)
                    || child(*progress) != Some(ExprType::Number)
                {
                    errors.push(ValidationError::new(
                        path,
                        "morphPath requires PathData, PathData, number",
                    ));
                    None
                } else {
                    let from_bound = path_point_bound(exprs, *from);
                    let to_bound = path_point_bound(exprs, *to);
                    if from_bound.max(to_bound) > MAX_FRAME_GEOMETRY_POINTS {
                        errors.push(ValidationError::new(
                            path.clone(),
                            format!(
                                "per-frame path morph exceeds {MAX_FRAME_GEOMETRY_POINTS} points; use a precomputed path trajectory"
                            ),
                        ));
                    }
                    if let (Some(from), Some(to)) =
                        (constant_path(exprs, *from), constant_path(exprs, *to))
                        && !from.has_same_topology(to)
                    {
                        errors.push(ValidationError::new(
                            path,
                            "morphPath inputs must have identical topology",
                        ));
                    }
                    Some(ExprType::PathData)
                }
            }
            Expr::PathPointAt {
                path: input,
                progress,
            } => {
                if child(*input) != Some(ExprType::PathData)
                    || child(*progress) != Some(ExprType::Number)
                {
                    errors.push(ValidationError::new(
                        path,
                        "pointAt requires PathData and number",
                    ));
                    None
                } else {
                    Some(ExprType::Point)
                }
            }
            Expr::PathTangentAt {
                path: input,
                progress,
            } => {
                if child(*input) != Some(ExprType::PathData)
                    || child(*progress) != Some(ExprType::Number)
                {
                    errors.push(ValidationError::new(
                        path,
                        "tangentAt requires PathData and number",
                    ));
                    None
                } else {
                    Some(ExprType::Vec2)
                }
            }
            Expr::PathAngleAt {
                path: input,
                progress,
            } => {
                if child(*input) != Some(ExprType::PathData)
                    || child(*progress) != Some(ExprType::Number)
                {
                    errors.push(ValidationError::new(
                        path,
                        "motionPath angle requires PathData and number",
                    ));
                    None
                } else {
                    Some(ExprType::Angle)
                }
            }
            Expr::PathLength { path: input } => {
                if child(*input) != Some(ExprType::PathData) {
                    errors.push(ValidationError::new(path, "pathLength requires PathData"));
                    None
                } else {
                    Some(ExprType::Number)
                }
            }
            // Bounds are a Rect; field access and anchors use existing geometry expressions.
            // Artifact-level validation checks whether the node key exists.
            Expr::NodeBounds { key } => {
                if key.is_empty() {
                    errors.push(ValidationError::new(path, "bounds() target key is empty"));
                    None
                } else {
                    Some(ExprType::Rect)
                }
            }
            Expr::PathTrajectory { frames, frame } => {
                let frame_type = child(*frame);
                let total_points = frames
                    .iter()
                    .try_fold(0usize, |total, value| total.checked_add(value.points.len()));
                let valid = !frames.is_empty()
                    && frames.len() <= MAX_GEOMETRY_TRAJECTORY_FRAMES
                    && total_points.is_some_and(|total| total <= MAX_PATH_TRAJECTORY_POINTS)
                    && frames.iter().all(|value| value.validate().is_ok())
                    && frames.first().is_some_and(|first| {
                        frames.iter().all(|value| first.has_same_topology(value))
                    });
                if !valid {
                    errors.push(ValidationError::new(
                        format!("{path}/frames"),
                        "path trajectory exceeds its budget or contains invalid/incompatible frames",
                    ));
                }
                if frame_type != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        format!("{path}/frame"),
                        "path trajectory frame must be a number",
                    ));
                }
                Some(ExprType::PathData)
            }
            Expr::GeometryField { input, field } => {
                let input = child(*input);
                let valid = match field {
                    GeometryField::X | GeometryField::Y => {
                        matches!(
                            input,
                            Some(ExprType::Point | ExprType::Vec2 | ExprType::Rect)
                        )
                    }
                    GeometryField::Width | GeometryField::Height => input == Some(ExprType::Rect),
                };
                if !valid {
                    errors.push(ValidationError::new(
                        path,
                        "geometry field does not exist on the input type",
                    ));
                    None
                } else {
                    Some(ExprType::Number)
                }
            }
            Expr::ToLength2 { input } => {
                if matches!(
                    child(*input),
                    Some(ExprType::Length2 | ExprType::Point | ExprType::Vec2)
                ) {
                    Some(ExprType::Length2)
                } else {
                    errors.push(ValidationError::new(
                        path,
                        "a length-pair style accepts Length2, Point, or Vec2",
                    ));
                    None
                }
            }
            Expr::Add { lhs, rhs }
            | Expr::Sub { lhs, rhs }
            | Expr::Mul { lhs, rhs }
            | Expr::Div { lhs, rhs } => {
                let lhs = child(*lhs);
                let rhs = child(*rhs);
                if lhs != Some(ExprType::Number) || rhs != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        path,
                        "arithmetic operands must both be numbers",
                    ));
                    None
                } else {
                    Some(ExprType::Number)
                }
            }
            Expr::Neg { input } => {
                if child(*input) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        path,
                        "negation input must be a number",
                    ));
                    None
                } else {
                    Some(ExprType::Number)
                }
            }
            Expr::Compare { op, lhs, rhs } => {
                let lhs = child(*lhs);
                let rhs = child(*rhs);
                let equality = matches!(op, CompareOp::Eq | CompareOp::NotEq);
                let text_enum_pair = matches!(
                    (lhs, rhs),
                    (Some(ExprType::String), Some(ExprType::Enum))
                        | (Some(ExprType::Enum), Some(ExprType::String))
                );
                if lhs.is_none() || (lhs != rhs && !(equality && text_enum_pair)) {
                    errors.push(ValidationError::new(
                        path,
                        "comparison operands must have the same type",
                    ));
                } else if !equality && lhs != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        path,
                        "ordered comparison operands must be numbers",
                    ));
                }
                Some(ExprType::Bool)
            }
            Expr::Select {
                condition,
                when_true,
                when_false,
            } => {
                let condition = child(*condition);
                let when_true = child(*when_true);
                let when_false = child(*when_false);
                if condition != Some(ExprType::Bool) {
                    errors.push(ValidationError::new(
                        format!("{path}/condition"),
                        "select condition must be bool",
                    ));
                }
                if when_true.is_none() || when_true != when_false {
                    errors.push(ValidationError::new(
                        path,
                        "select branches must have the same type",
                    ));
                    None
                } else {
                    when_true
                }
            }
            Expr::Template { parts } => {
                let static_bytes = parts.iter().fold(0usize, |total, part| {
                    total.saturating_add(match part {
                        TemplatePart::Text { value } => value.len(),
                        TemplatePart::Expr { .. } => 0,
                    })
                });
                let mut invalid_substitutions = Vec::new();
                let mut forward_references = Vec::new();
                for (part_index, part) in parts.iter().enumerate() {
                    let TemplatePart::Expr { expr } = part else {
                        continue;
                    };
                    if expr.0 as usize >= index {
                        forward_references.push(part_index);
                        continue;
                    }
                    let ty = types[expr.0 as usize];
                    if !matches!(
                        ty,
                        Some(
                            ExprType::Number
                                | ExprType::Bool
                                | ExprType::String
                                | ExprType::Enum
                                | ExprType::Length
                                | ExprType::Angle
                                | ExprType::Color
                        )
                    ) {
                        invalid_substitutions.push(part_index);
                    }
                }
                if parts.is_empty()
                    || parts.len() > MAX_TEMPLATE_PARTS
                    || static_bytes > MAX_TEMPLATE_STATIC_BYTES
                {
                    errors.push(ValidationError::new(
                        format!("{path}/parts"),
                        "template must contain 1..=64 parts and at most 8192 static UTF-8 bytes",
                    ));
                }
                for part_index in forward_references {
                    errors.push(ValidationError::new(
                        format!("{path}/parts/{part_index}/expr"),
                        "expressions may only reference earlier expressions",
                    ));
                }
                for part_index in invalid_substitutions {
                    errors.push(ValidationError::new(
                        format!("{path}/parts/{part_index}/expr"),
                        "template substitutions must be scalar CSS-token values",
                    ));
                }
                Some(ExprType::String)
            }
            Expr::MathUnary { input, .. } | Expr::Noise1D { x: input, .. } => {
                if child(*input) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(path, "math input must be a number"));
                }
                Some(ExprType::Number)
            }
            Expr::MathBinary { lhs, rhs, .. } => {
                if child(*lhs) != Some(ExprType::Number) || child(*rhs) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(path, "math inputs must be numbers"));
                }
                Some(ExprType::Number)
            }
            Expr::Noise2D { x, y, .. } => {
                if child(*x) != Some(ExprType::Number) || child(*y) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(path, "noise inputs must be numbers"));
                }
                Some(ExprType::Number)
            }
            Expr::FormatNumber { input, format } => {
                if child(*input) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        format!("{path}/input"),
                        "format input must be a number",
                    ));
                }
                if format.validate().is_err() {
                    errors.push(ValidationError::new(
                        format!("{path}/format"),
                        "number format exceeds decimals 0..=12 or width 1..=64",
                    ));
                }
                Some(ExprType::String)
            }
            Expr::Spring {
                elapsed_frames,
                mass,
                stiffness,
                damping,
            } => {
                if child(*elapsed_frames) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        format!("{path}/elapsedFrames"),
                        "spring time input must be a number",
                    ));
                }
                // Require finite, positive physical parameters to prevent division by zero or
                // non-finite oscillator output.
                if !mass.is_finite()
                    || *mass <= 0.0
                    || !stiffness.is_finite()
                    || *stiffness <= 0.0
                    || !damping.is_finite()
                    || *damping < 0.0
                {
                    errors.push(ValidationError::new(
                        format!("{path}"),
                        "spring mass/stiffness must be finite and positive, damping non-negative",
                    ));
                }
                Some(ExprType::Number)
            }
            Expr::Interpolate {
                input,
                stops,
                easings,
                ..
            } => {
                if child(*input) != Some(ExprType::Number) {
                    errors.push(ValidationError::new(
                        format!("{path}/input"),
                        "interpolate input must be a number",
                    ));
                }
                if stops.len() < 2 {
                    errors.push(ValidationError::new(
                        format!("{path}/stops"),
                        "interpolate needs at least two stops",
                    ));
                    None
                } else {
                    let output_ty = ExprType::of_value(&stops[0].output);
                    let mut previous = f64::NEG_INFINITY;
                    for (stop_index, stop) in stops.iter().enumerate() {
                        if !stop.input.is_finite()
                            || stop.input <= previous
                            || !stop.output.is_finite()
                            || ExprType::of_value(&stop.output) != output_ty
                        {
                            errors.push(ValidationError::new(
                                format!("{path}/stops/{stop_index}"),
                                "stops must be finite, strictly increasing, and type-consistent",
                            ));
                        }
                        previous = stop.input;
                    }
                    if output_ty == ExprType::PathData
                        && let MotionValue::PathData(first) = &stops[0].output
                    {
                        for (stop_index, stop) in stops.iter().enumerate() {
                            let MotionValue::PathData(geometry) = &stop.output else {
                                continue;
                            };
                            if geometry.points.len() > MAX_FRAME_GEOMETRY_POINTS
                                || !first.has_same_topology(geometry)
                            {
                                errors.push(ValidationError::new(
                                    format!("{path}/stops/{stop_index}"),
                                    "PathData interpolation requires identical topology within the per-frame geometry budget",
                                ));
                            }
                        }
                    }
                    if !output_ty.is_continuous() {
                        errors.push(ValidationError::new(
                            format!("{path}/stops"),
                            "interpolate outputs must be continuous values",
                        ));
                    }
                    if !easings.is_empty() && easings.len() + 1 != stops.len() {
                        errors.push(ValidationError::new(
                            format!("{path}/easings"),
                            "easings must be empty or have one entry per segment",
                        ));
                    }
                    for (easing_index, easing) in easings.iter().enumerate() {
                        if !easing.is_finite() {
                            errors.push(ValidationError::new(
                                format!("{path}/easings/{easing_index}"),
                                "easing control points must be finite",
                            ));
                        }
                    }
                    Some(output_ty)
                }
            }
        };
        let children_ok = expr
            .children()
            .into_iter()
            .all(|id| admitted.get(id.0 as usize).copied().unwrap_or(false));
        admitted.push(errors.len() == errors_before && children_ok);
        types.push(ty);
    }
    ExprAdmission { types, admitted }
}

/// Admission flags without diagnostics, used by [`crate::fold_constants`].
pub(crate) fn admitted_flags(exprs: &[Expr], controls: &ControlsSchema) -> Vec<bool> {
    let mut errors = Vec::new();
    validate_exprs(exprs, controls, &mut errors).admitted
}

const MAX_PATH_TRAJECTORY_POINTS: usize = 2_000_000;

fn constant_path(exprs: &[Expr], id: ExprId) -> Option<&PathData> {
    match exprs.get(id.0 as usize)? {
        Expr::Const {
            value: MotionValue::PathData(path),
        } => Some(path),
        _ => None,
    }
}

fn path_point_bound(exprs: &[Expr], id: ExprId) -> usize {
    match exprs.get(id.0 as usize) {
        Some(Expr::Const {
            value: MotionValue::PathData(path),
        }) => path.points.len(),
        Some(Expr::Prop { .. }) => MAX_FRAME_GEOMETRY_POINTS,
        Some(Expr::PathMorph { from, to, .. }) => {
            path_point_bound(exprs, *from).max(path_point_bound(exprs, *to))
        }
        Some(Expr::PathLine { points }) => points.len(),
        Some(Expr::PathTemplate { points, .. }) => points.len(),
        Some(Expr::PathCubic { .. }) => 4,
        Some(Expr::PathArc { .. }) => 1 + crate::geometry::PATH_ARC_SEGMENTS * 3,
        Some(Expr::PathArea { path, .. }) => {
            path_flattened_point_bound(exprs, *path).saturating_add(2)
        }
        Some(Expr::PathOffset { path, .. }) => path_flattened_point_bound(exprs, *path),
        Some(Expr::PathTrajectory { frames, .. }) => frames
            .iter()
            .map(|frame| frame.points.len())
            .max()
            .unwrap_or(0),
        _ => MAX_FRAME_GEOMETRY_POINTS,
    }
}

fn path_flattened_point_bound(exprs: &[Expr], id: ExprId) -> usize {
    match exprs.get(id.0 as usize) {
        Some(Expr::Const {
            value: MotionValue::PathData(path),
        }) => path.verbs.iter().fold(0usize, |total, verb| {
            total.saturating_add(match verb {
                valle_draw::PathVerb::Move | valle_draw::PathVerb::Line => 1,
                valle_draw::PathVerb::Quad | valle_draw::PathVerb::Cubic => {
                    crate::geometry::PATH_CURVE_STEPS
                }
                valle_draw::PathVerb::Close => 0,
            })
        }),
        Some(Expr::PathLine { points }) => points.len(),
        Some(Expr::PathTemplate { verbs, .. }) => verbs.iter().fold(0usize, |total, verb| {
            total.saturating_add(match verb {
                PathVerb::Move | PathVerb::Line => 1,
                PathVerb::Quad | PathVerb::Cubic => crate::geometry::PATH_CURVE_STEPS,
                PathVerb::Close => 0,
            })
        }),
        Some(Expr::PathCubic { .. }) => 1 + crate::geometry::PATH_CURVE_STEPS,
        Some(Expr::PathArc { .. }) => {
            1 + crate::geometry::PATH_ARC_SEGMENTS * crate::geometry::PATH_CURVE_STEPS
        }
        Some(Expr::PathMorph { from, to, .. }) => {
            path_flattened_point_bound(exprs, *from).max(path_flattened_point_bound(exprs, *to))
        }
        Some(Expr::PathTrajectory { frames, .. }) => frames
            .iter()
            .map(|frame| {
                frame.verbs.iter().fold(0usize, |total, verb| {
                    total.saturating_add(match verb {
                        valle_draw::PathVerb::Move | valle_draw::PathVerb::Line => 1,
                        valle_draw::PathVerb::Quad | valle_draw::PathVerb::Cubic => {
                            crate::geometry::PATH_CURVE_STEPS
                        }
                        valle_draw::PathVerb::Close => 0,
                    })
                })
            })
            .max()
            .unwrap_or(0),
        Some(Expr::PathArea { path, .. }) => {
            path_flattened_point_bound(exprs, *path).saturating_add(2)
        }
        Some(Expr::PathOffset { path, .. }) => path_flattened_point_bound(exprs, *path),
        _ => MAX_FRAME_GEOMETRY_POINTS.saturating_mul(crate::geometry::PATH_CURVE_STEPS),
    }
}

fn frame_invariant(exprs: &[Expr], id: ExprId) -> bool {
    match exprs.get(id.0 as usize) {
        Some(Expr::Const { .. } | Expr::Prop { .. }) => true,
        Some(Expr::MakePoint { x, y }) => frame_invariant(exprs, *x) && frame_invariant(exprs, *y),
        Some(Expr::PathLine { points }) => {
            points.iter().all(|point| frame_invariant(exprs, *point))
        }
        Some(Expr::PathTemplate { points, .. }) => {
            points.iter().all(|point| frame_invariant(exprs, *point))
        }
        Some(Expr::PathCubic {
            from,
            control_1,
            control_2,
            to,
        }) => [*from, *control_1, *control_2, *to]
            .into_iter()
            .all(|point| frame_invariant(exprs, point)),
        Some(Expr::PathArc {
            center,
            radius,
            start_angle,
            end_angle,
        }) => [*center, *radius, *start_angle, *end_angle]
            .into_iter()
            .all(|input| frame_invariant(exprs, input)),
        Some(Expr::PathArea { path, baseline }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *baseline)
        }
        Some(Expr::PathOffset { path, distance }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *distance)
        }
        _ => false,
    }
}

pub fn geometry_eval_policy(exprs: &[Expr], id: ExprId) -> Option<GeometryEvalPolicy> {
    let expr = exprs.get(id.0 as usize)?;
    if frame_invariant(exprs, id) {
        return Some(GeometryEvalPolicy::PrepareCached);
    }
    Some(match expr {
        Expr::PathTrajectory { .. } => GeometryEvalPolicy::PrecomputedTrajectory,
        Expr::PathMorph { .. }
        | Expr::PathLine { .. }
        | Expr::PathTemplate { .. }
        | Expr::PathCubic { .. }
        | Expr::PathArc { .. }
        | Expr::PathArea { .. }
        | Expr::PathOffset { .. }
        | Expr::Interpolate { .. }
        | Expr::Select { .. } => GeometryEvalPolicy::FrameExpr,
        _ => return None,
    })
}
