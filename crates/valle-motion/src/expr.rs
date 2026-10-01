use serde::{Deserialize, Serialize};

use crate::geometry::{
    CompatibleBarycentricMorph, GeometryEvalPolicy, MAX_FRAME_GEOMETRY_POINTS,
    MAX_GEOMETRY_TRAJECTORY_FRAMES, PathData,
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
    HostSeconds,
    HostDuration,
    HostProgress,
    DurationFrames,
    FpsNum,
    FpsDen,
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

/// Spatial profile of a text range selector. The selection is evaluated once per text unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RangeShape {
    Square,
    Ramp,
    Triangle,
    Smooth,
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
    /// Discrete frame/bin addressing: absorb only floating-point roundoff at integer boundaries.
    FrameFloor,
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
pub const MAX_SIMULATION_STEPS: usize = 16_384;

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
    /// One typed value from the current instance's structure-of-arrays table.
    InstanceField {
        column: u32,
        value_type: ExprType,
    },
    InstanceIndex,
    InstanceCount,
    Context {
        input: ContextInput,
    },
    /// Unit-local selection weight in [0, 1]. Bounds, offset and softness may vary per frame.
    RangeSelector {
        start: ExprId,
        end: ExprId,
        offset: ExprId,
        softness: ExprId,
        shape: RangeShape,
    },
    Prop {
        name: String,
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
    PathSector {
        center: ExprId,
        inner: ExprId,
        outer: ExprId,
        start: ExprId,
        end: ExprId,
        corner_radius: ExprId,
    },
    PathAreaBand {
        upper: ExprId,
        lower: ExprId,
    },
    PathOffset {
        path: ExprId,
        distance: ExprId,
    },
    PathResample {
        path: ExprId,
        count: u16,
    },
    PathReverse {
        path: ExprId,
    },
    PathRoundCorners {
        path: ExprId,
        radius: ExprId,
    },
    PathZigzag {
        path: ExprId,
        size: ExprId,
        ridges: u16,
    },
    PathNoiseDisplace {
        path: ExprId,
        seed: u64,
        amount: ExprId,
        frequency: ExprId,
        phase: ExprId,
    },
    PathPuckerBloat {
        path: ExprId,
        amount: ExprId,
    },
    PathTwist {
        path: ExprId,
        angle: ExprId,
    },
    PathSimplify {
        path: ExprId,
        tolerance: ExprId,
    },
    PathStrokeToPath {
        path: ExprId,
        width: ExprId,
    },
    PathMorph {
        from: ExprId,
        to: ExprId,
        progress: ExprId,
    },
    PathCompatibleMorph {
        prepared: CompatibleBarycentricMorph,
        progress: ExprId,
    },
    PathCompatibleMorphSequence {
        segments: Vec<CompatibleBarycentricMorph>,
        stops: Vec<f64>,
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
        color_space: valle_draw::program::GradientInterpolation,
        extrapolate_left: Extrapolation,
        extrapolate_right: Extrapolation,
    },
    /// One numeric field of a prepare-time simulation. Samples are spaced by dt from time zero;
    /// arbitrary frame times interpolate adjacent samples after clamping to duration.
    SimulationSample {
        time: ExprId,
        dt: f64,
        duration: f64,
        samples: Vec<f64>,
    },
    /// Prepare-time audio analysis sampled at one output frame per entry. Lookup is held within
    /// each frame; onset pulses and wrapped beat phases must not interpolate into neighbors.
    AudioSample {
        time: ExprId,
        fps: u32,
        samples: Vec<f64>,
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
        initial_velocity: f64,
        output: crate::spring::SpringOutput,
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
        let direct = matches!(expr, Expr::Context { input } if input.is_unit())
            || matches!(expr, Expr::RangeSelector { .. });
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
            | Expr::InstanceField { .. }
            | Expr::InstanceIndex
            | Expr::InstanceCount
            | Expr::Context { .. }
            | Expr::Prop { .. }
            | Expr::NodeBounds { .. }
            | Expr::Project3D { .. } => Vec::new(),
            Expr::MakePoint { x, y } => vec![*x, *y],
            Expr::RangeSelector {
                start,
                end,
                offset,
                softness,
                ..
            } => vec![*start, *end, *offset, *softness],
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
            Expr::PathSector {
                center,
                inner,
                outer,
                start,
                end,
                corner_radius,
            } => vec![*center, *inner, *outer, *start, *end, *corner_radius],
            Expr::PathAreaBand { upper, lower } => vec![*upper, *lower],
            Expr::PathOffset { path, distance } => vec![*path, *distance],
            Expr::PathResample { path, .. } => vec![*path],
            Expr::PathReverse { path } => vec![*path],
            Expr::PathRoundCorners { path, radius } => vec![*path, *radius],
            Expr::PathZigzag { path, size, .. } => vec![*path, *size],
            Expr::PathNoiseDisplace {
                path,
                amount,
                frequency,
                phase,
                ..
            } => {
                vec![*path, *amount, *frequency, *phase]
            }
            Expr::PathPuckerBloat { path, amount } => vec![*path, *amount],
            Expr::PathTwist { path, angle } => vec![*path, *angle],
            Expr::PathSimplify { path, tolerance } => vec![*path, *tolerance],
            Expr::PathStrokeToPath { path, width } => vec![*path, *width],
            Expr::PathMorph { from, to, progress } => vec![*from, *to, *progress],
            Expr::PathCompatibleMorph { progress, .. }
            | Expr::PathCompatibleMorphSequence { progress, .. } => vec![*progress],
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
            Expr::SimulationSample { time, .. } | Expr::AudioSample { time, .. } => vec![*time],
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

    /// Rewrite direct dependencies after extracting an expression closure.
    pub fn remap_children(&mut self, mut map: impl FnMut(ExprId) -> ExprId) {
        match self {
            Expr::Const { .. }
            | Expr::InstanceField { .. }
            | Expr::InstanceIndex
            | Expr::InstanceCount
            | Expr::Context { .. }
            | Expr::Prop { .. }
            | Expr::NodeBounds { .. }
            | Expr::Project3D { .. } => {}
            Expr::MakePoint { x, y } => {
                *x = map(*x);
                *y = map(*y);
            }
            Expr::RangeSelector {
                start,
                end,
                offset,
                softness,
                ..
            } => {
                *start = map(*start);
                *end = map(*end);
                *offset = map(*offset);
                *softness = map(*softness);
            }
            Expr::MakeRect {
                x,
                y,
                width,
                height,
            } => {
                *x = map(*x);
                *y = map(*y);
                *width = map(*width);
                *height = map(*height);
            }
            Expr::PathLine { points } => {
                for point in points {
                    *point = map(*point);
                }
            }
            Expr::PathTemplate { points, .. } => {
                for point in points {
                    *point = map(*point);
                }
            }
            Expr::PathCubic {
                from,
                control_1,
                control_2,
                to,
            } => {
                *from = map(*from);
                *control_1 = map(*control_1);
                *control_2 = map(*control_2);
                *to = map(*to);
            }
            Expr::PathArc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                *center = map(*center);
                *radius = map(*radius);
                *start_angle = map(*start_angle);
                *end_angle = map(*end_angle);
            }
            Expr::PathArea { path, baseline } => {
                *path = map(*path);
                *baseline = map(*baseline);
            }
            Expr::PathSector {
                center,
                inner,
                outer,
                start,
                end,
                corner_radius,
            } => {
                *center = map(*center);
                *inner = map(*inner);
                *outer = map(*outer);
                *start = map(*start);
                *end = map(*end);
                *corner_radius = map(*corner_radius);
            }
            Expr::PathAreaBand { upper, lower } => {
                *upper = map(*upper);
                *lower = map(*lower);
            }
            Expr::PathOffset { path, distance } => {
                *path = map(*path);
                *distance = map(*distance);
            }
            Expr::PathResample { path, .. } => {
                *path = map(*path);
            }
            Expr::PathReverse { path } => {
                *path = map(*path);
            }
            Expr::PathRoundCorners { path, radius } => {
                *path = map(*path);
                *radius = map(*radius);
            }
            Expr::PathZigzag { path, size, .. } => {
                *path = map(*path);
                *size = map(*size);
            }
            Expr::PathNoiseDisplace {
                path,
                amount,
                frequency,
                phase,
                ..
            } => {
                *path = map(*path);
                *amount = map(*amount);
                *frequency = map(*frequency);
                *phase = map(*phase);
            }
            Expr::PathPuckerBloat { path, amount } => {
                *path = map(*path);
                *amount = map(*amount);
            }
            Expr::PathTwist { path, angle } => {
                *path = map(*path);
                *angle = map(*angle);
            }
            Expr::PathSimplify { path, tolerance } => {
                *path = map(*path);
                *tolerance = map(*tolerance);
            }
            Expr::PathStrokeToPath { path, width } => {
                *path = map(*path);
                *width = map(*width);
            }
            Expr::PathMorph { from, to, progress } => {
                *from = map(*from);
                *to = map(*to);
                *progress = map(*progress);
            }
            Expr::PathCompatibleMorph { progress, .. }
            | Expr::PathCompatibleMorphSequence { progress, .. } => {
                *progress = map(*progress);
            }
            Expr::PathPointAt { path, progress }
            | Expr::PathTangentAt { path, progress }
            | Expr::PathAngleAt { path, progress } => {
                *path = map(*path);
                *progress = map(*progress);
            }
            Expr::PathLength { path } => {
                *path = map(*path);
            }
            Expr::PathTrajectory { frame, .. } => {
                *frame = map(*frame);
            }
            Expr::GeometryField { input, .. }
            | Expr::ToLength2 { input }
            | Expr::Neg { input }
            | Expr::MathUnary { input, .. }
            | Expr::Noise1D { x: input, .. }
            | Expr::FormatNumber { input, .. }
            | Expr::Interpolate { input, .. } => {
                *input = map(*input);
            }
            Expr::SimulationSample { time, .. } | Expr::AudioSample { time, .. } => {
                *time = map(*time);
            }
            Expr::MathBinary { lhs, rhs, .. } => {
                *lhs = map(*lhs);
                *rhs = map(*rhs);
            }
            Expr::Noise2D { x, y, .. } => {
                *x = map(*x);
                *y = map(*y);
            }
            Expr::Spring { elapsed_frames, .. } => {
                *elapsed_frames = map(*elapsed_frames);
            }
            Expr::Add { lhs, rhs }
            | Expr::Sub { lhs, rhs }
            | Expr::Mul { lhs, rhs }
            | Expr::Div { lhs, rhs }
            | Expr::Compare { lhs, rhs, .. } => {
                *lhs = map(*lhs);
                *rhs = map(*rhs);
            }
            Expr::Select {
                condition,
                when_true,
                when_false,
            } => {
                *condition = map(*condition);
                *when_true = map(*when_true);
                *when_false = map(*when_false);
            }
            Expr::Template { parts } => {
                for part in parts {
                    if let TemplatePart::Expr { expr } = part {
                        *expr = map(*expr);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    pub fn of_value(value: &MotionValue) -> Self {
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
            ControlType::String => ExprType::String,
            ControlType::Bool => ExprType::Bool,
            ControlType::Color => ExprType::Color,
            ControlType::Length => ExprType::Length,
            ControlType::Angle => ExprType::Angle,
            ControlType::Point => ExprType::Point,
            ControlType::Rect => ExprType::Rect,
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
        let ty = validate_next_expr(expr, &exprs[..index], &types, controls, errors);
        let children_ok = expr
            .children()
            .into_iter()
            .all(|id| admitted.get(id.0 as usize).copied().unwrap_or(false));
        admitted.push(errors.len() == errors_before && children_ok);
        types.push(ty);
    }
    ExprAdmission { types, admitted }
}

/// Check one expression against an already-typed prefix. Source compilers can retain
/// these types while building the arena instead of repeatedly scanning the whole DAG.
/// Errors use the same expression indices and rules as complete artifact admission.
pub fn validate_next_expr(
    expr: &Expr,
    exprs: &[Expr],
    types: &[Option<ExprType>],
    controls: &ControlsSchema,
    errors: &mut Vec<ValidationError>,
) -> Option<ExprType> {
    let index = exprs.len();
    let path = format!("/exprs/{index}");
    let mut child = |id: ExprId| -> Option<ExprType> {
        if id.0 as usize >= index {
            errors.push(ValidationError::new(
                format!("{path}/expr"),
                "expressions may only reference earlier expressions",
            ));
            None
        } else {
            types.get(id.0 as usize).copied().flatten()
        }
    };
    match expr {
        Expr::Const { value } => {
            if !value.is_finite() {
                errors.push(ValidationError::new(
                    format!("{path}/value"),
                    "constant must be finite",
                ));
            }
            Some(ExprType::of_value(value))
        }
        Expr::InstanceField { value_type, .. } => Some(*value_type),
        Expr::InstanceIndex | Expr::InstanceCount => Some(ExprType::Number),
        Expr::Context { .. } => Some(ExprType::Number),
        Expr::RangeSelector {
            start,
            end,
            offset,
            softness,
            ..
        } => {
            let numeric = [*start, *end, *offset, *softness]
                .into_iter()
                .all(|id| child(id) == Some(ExprType::Number));
            for (name, id) in [("start", *start), ("end", *end), ("softness", *softness)] {
                if let Some(Expr::Const {
                    value: MotionValue::Number(value),
                }) = exprs.get(id.0 as usize)
                    && !(0.0..=1.0).contains(value)
                {
                    errors.push(ValidationError::new(
                        format!("{path}/{name}"),
                        "rangeSelector start, end and softness must be within 0..=1",
                    ));
                }
            }
            if numeric {
                Some(ExprType::Number)
            } else {
                errors.push(ValidationError::new(
                    path,
                    "rangeSelector needs numeric start, end, offset, and softness",
                ));
                None
            }
        }
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
                || path_point_bound(exprs, *input).saturating_add(2) > MAX_FRAME_GEOMETRY_POINTS
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
        Expr::PathSector {
            center,
            inner,
            outer,
            start,
            end,
            corner_radius,
        } => {
            if child(*center) != Some(ExprType::Point)
                || [*inner, *outer, *start, *end, *corner_radius]
                    .into_iter()
                    .any(|number| child(number) != Some(ExprType::Number))
                || crate::geometry::PATH_SECTOR_POINTS > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "sector requires Point center and numeric inner/outer/start/end/cornerRadius",
                ));
                None
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathAreaBand { upper, lower } => {
            if child(*upper) != Some(ExprType::PathData)
                || child(*lower) != Some(ExprType::PathData)
                || path_point_bound(exprs, *upper).saturating_add(path_point_bound(exprs, *lower))
                    > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "areaBand requires two bounded PathData values within the per-frame geometry budget",
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
        Expr::PathResample { path: input, count } => {
            let bound = path_contour_bound(exprs, *input).saturating_mul(usize::from(*count));
            if child(*input) != Some(ExprType::PathData)
                || *count < 2
                || bound > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "resamplePath requires PathData and a static point count within the per-frame geometry budget",
                ));
                None
            } else if let Some(input) = constant_path(exprs, *input) {
                match input.resample(usize::from(*count)) {
                    Ok(_) => Some(ExprType::PathData),
                    Err(reason) => {
                        errors.push(ValidationError::new(
                            path,
                            format!("resamplePath: {reason}"),
                        ));
                        None
                    }
                }
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathReverse { path: input } => {
            if child(*input) != Some(ExprType::PathData)
                || path_point_bound(exprs, *input) > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "reversePath requires bounded PathData within the per-frame geometry budget",
                ));
                None
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathRoundCorners {
            path: input,
            radius,
        } => {
            if child(*input) != Some(ExprType::PathData)
                || child(*radius) != Some(ExprType::Number)
                || path_point_bound(exprs, *input).saturating_mul(4) > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "roundCorners requires bounded line-only PathData and a numeric radius",
                ));
                None
            } else if let (Some(input), Some(radius)) = (
                constant_path(exprs, *input),
                constant_number(exprs, *radius),
            ) {
                match input.round_corners(radius) {
                    Ok(_) => Some(ExprType::PathData),
                    Err(reason) => {
                        errors.push(ValidationError::new(
                            path,
                            format!("roundCorners: {reason}"),
                        ));
                        None
                    }
                }
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathZigzag {
            path: input,
            size,
            ridges,
        } => {
            if child(*input) != Some(ExprType::PathData)
                || child(*size) != Some(ExprType::Number)
                || *ridges == 0
                || *ridges > 1023
            {
                errors.push(ValidationError::new(path, "zigzag requires one open PathData contour, numeric size, and 1–1023 static ridges"));
                None
            } else if let (Some(input), Some(size)) =
                (constant_path(exprs, *input), constant_number(exprs, *size))
            {
                match input.zigzag(size, usize::from(*ridges)) {
                    Ok(_) => Some(ExprType::PathData),
                    Err(reason) => {
                        errors.push(ValidationError::new(path, format!("zigzag: {reason}")));
                        None
                    }
                }
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathNoiseDisplace {
            path: input,
            amount,
            frequency,
            phase,
            ..
        } => {
            if child(*input) != Some(ExprType::PathData)
                || [*amount, *frequency, *phase]
                    .into_iter()
                    .any(|id| child(id) != Some(ExprType::Number))
                || path_contour_bound(exprs, *input)
                    .saturating_mul(crate::geometry::POLAR_MORPH_POINTS)
                    > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(path, "noiseDisplace requires bounded PathData and numeric amount, frequency and phase"));
                None
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathPuckerBloat {
            path: input,
            amount: parameter,
        }
        | Expr::PathTwist {
            path: input,
            angle: parameter,
        }
        | Expr::PathSimplify {
            path: input,
            tolerance: parameter,
        }
        | Expr::PathStrokeToPath {
            path: input,
            width: parameter,
        } => {
            let name = match expr {
                Expr::PathPuckerBloat { .. } => "puckerBloat",
                Expr::PathTwist { .. } => "twist",
                Expr::PathSimplify { .. } => "simplify",
                Expr::PathStrokeToPath { .. } => "strokeToPath",
                _ => unreachable!(),
            };
            if child(*input) != Some(ExprType::PathData)
                || child(*parameter) != Some(ExprType::Number)
                || (!matches!(expr, Expr::PathStrokeToPath { .. })
                    && path_contour_bound(exprs, *input)
                        .saturating_mul(crate::geometry::PATH_MODIFIER_SAMPLES)
                        > MAX_FRAME_GEOMETRY_POINTS)
            {
                errors.push(ValidationError::new(
                    path,
                    format!("{name} requires bounded PathData and a numeric parameter"),
                ));
                None
            } else if let Some(input) = constant_path(exprs, *input) {
                let value = constant_number(exprs, *parameter).unwrap_or(0.0);
                let result = match expr {
                    Expr::PathPuckerBloat { .. } => input.pucker_bloat(value),
                    Expr::PathTwist { .. } => input.twist(value),
                    Expr::PathSimplify { .. } => input.simplify(value),
                    Expr::PathStrokeToPath { .. } => input.stroke_to_path(value),
                    _ => unreachable!(),
                };
                match result {
                    Ok(_) => Some(ExprType::PathData),
                    Err(reason) => {
                        errors.push(ValidationError::new(path, format!("{name}: {reason}")));
                        None
                    }
                }
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
        Expr::PathCompatibleMorph { prepared, progress } => {
            if child(*progress) != Some(ExprType::Number)
                || !prepared.is_valid()
                || prepared.mesh().outline_vertices.len() > MAX_FRAME_GEOMETRY_POINTS
            {
                errors.push(ValidationError::new(
                    path,
                    "compatible morph requires a valid prepared mesh and numeric progress",
                ));
                None
            } else {
                Some(ExprType::PathData)
            }
        }
        Expr::PathCompatibleMorphSequence {
            segments,
            stops,
            progress,
        } => {
            let outline_count = segments
                .first()
                .map_or(0, |segment| segment.mesh().outline_vertices.len());
            if child(*progress) != Some(ExprType::Number)
                || segments.is_empty()
                || segments.len() + 1 != stops.len()
                || stops.iter().any(|stop| !stop.is_finite())
                || stops.windows(2).any(|pair| pair[0] >= pair[1])
                || outline_count > MAX_FRAME_GEOMETRY_POINTS
                || segments.iter().any(|segment| {
                    !segment.is_valid() || segment.mesh().outline_vertices.len() != outline_count
                })
            {
                errors.push(ValidationError::new(
                    path,
                    "compatible morph sequence requires valid equal-topology meshes, increasing stops, and numeric progress",
                ));
                None
            } else {
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
                && frames
                    .first()
                    .is_some_and(|first| frames.iter().all(|value| first.has_same_topology(value)));
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
                let ty = types.get(expr.0 as usize).copied().flatten();
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
            initial_velocity,
            ..
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
                || !initial_velocity.is_finite()
            {
                errors.push(ValidationError::new(
                    format!("{path}"),
                    "spring mass/stiffness must be finite and positive, damping non-negative, initialVelocity finite",
                ));
            }
            Some(ExprType::Number)
        }
        Expr::SimulationSample {
            time,
            dt,
            duration,
            samples,
        } => {
            if child(*time) != Some(ExprType::Number) {
                errors.push(ValidationError::new(
                    format!("{path}/time"),
                    "simulation time input must be a number",
                ));
            }
            let steps = duration / dt;
            if !dt.is_finite()
                || *dt <= 0.0
                || !duration.is_finite()
                || *duration <= 0.0
                || !steps.is_finite()
                || steps > MAX_SIMULATION_STEPS as f64
                || steps.ceil() as usize + 1 != samples.len()
                || samples.len() < 2
                || samples.len() > MAX_SIMULATION_STEPS + 1
                || samples.iter().any(|value| !value.is_finite())
                || samples
                    .windows(2)
                    .any(|pair| !(pair[1] - pair[0]).is_finite())
            {
                errors.push(ValidationError::new(
                    format!("{path}/samples"),
                    "simulation requires finite samples at fixed positive dt through duration, within 16384 steps",
                ));
            }
            Some(ExprType::Number)
        }
        Expr::AudioSample { time, fps, samples } => {
            if child(*time) != Some(ExprType::Number)
                || !(1..=120).contains(fps)
                || samples.len() < 2
                || samples.len() > MAX_SIMULATION_STEPS + 1
                || samples.iter().any(|sample| !sample.is_finite())
            {
                errors.push(ValidationError::new(
                    path,
                    "audio sample needs numeric time, fps 1..=120, and a finite bounded table",
                ));
            }
            Some(ExprType::Number)
        }
        Expr::Interpolate {
            input,
            stops,
            easings,
            color_space,
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
                if output_ty != ExprType::Color
                    && *color_space != valle_draw::program::GradientInterpolation::Srgb
                {
                    errors.push(ValidationError::new(
                        format!("{path}/colorSpace"),
                        "interpolate colorSpace requires color output",
                    ));
                }
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
                    if !easing.is_valid() {
                        errors.push(ValidationError::new(
                            format!("{path}/easings/{easing_index}"),
                            "easing requires finite control points with Bezier x coordinates in [0,1], or a positive step count",
                        ));
                    }
                }
                Some(output_ty)
            }
        }
    }
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

fn constant_number(exprs: &[Expr], id: ExprId) -> Option<f64> {
    match exprs.get(id.0 as usize)? {
        Expr::Const {
            value: MotionValue::Number(value),
        } => Some(*value),
        _ => None,
    }
}

fn path_point_bound(exprs: &[Expr], id: ExprId) -> usize {
    match exprs.get(id.0 as usize) {
        Some(Expr::PathStrokeToPath { .. }) => MAX_FRAME_GEOMETRY_POINTS,
        Some(Expr::Const {
            value: MotionValue::PathData(path),
        }) => path.points.len(),
        Some(Expr::Prop { .. }) => MAX_FRAME_GEOMETRY_POINTS,
        Some(Expr::PathMorph { from, to, .. }) => {
            path_point_bound(exprs, *from).max(path_point_bound(exprs, *to))
        }
        Some(Expr::PathCompatibleMorph { prepared, .. }) => prepared.mesh().outline_vertices.len(),
        Some(Expr::PathCompatibleMorphSequence { segments, .. }) => segments
            .first()
            .map_or(0, |segment| segment.mesh().outline_vertices.len()),
        Some(Expr::PathLine { points }) => points.len(),
        Some(Expr::PathTemplate { points, .. }) => points.len(),
        Some(Expr::PathCubic { .. }) => 4,
        Some(Expr::PathArc { .. }) => 1 + crate::geometry::PATH_ARC_SEGMENTS * 3,
        Some(Expr::PathSector { .. }) => crate::geometry::PATH_SECTOR_POINTS,
        Some(Expr::PathArea { path, .. }) => path_point_bound(exprs, *path).saturating_add(2),
        Some(Expr::PathAreaBand { upper, lower }) => {
            path_point_bound(exprs, *upper).saturating_add(path_point_bound(exprs, *lower))
        }
        Some(Expr::PathOffset { path, .. }) => path_flattened_point_bound(exprs, *path),
        Some(Expr::PathResample { path, count }) => {
            path_contour_bound(exprs, *path).saturating_mul(usize::from(*count))
        }
        Some(Expr::PathReverse { path }) => path_point_bound(exprs, *path),
        Some(Expr::PathRoundCorners { path, .. }) => {
            path_point_bound(exprs, *path).saturating_mul(4)
        }
        Some(Expr::PathZigzag { ridges, .. }) => 2 * usize::from(*ridges) + 1,
        Some(Expr::PathNoiseDisplace { path, .. }) => {
            path_contour_bound(exprs, *path).saturating_mul(crate::geometry::POLAR_MORPH_POINTS)
        }
        Some(
            Expr::PathPuckerBloat { path, .. }
            | Expr::PathTwist { path, .. }
            | Expr::PathSimplify { path, .. },
        ) => {
            path_contour_bound(exprs, *path).saturating_mul(crate::geometry::PATH_MODIFIER_SAMPLES)
        }
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
        Some(Expr::PathStrokeToPath { .. }) => MAX_FRAME_GEOMETRY_POINTS,
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
        Some(Expr::PathCompatibleMorph { prepared, .. }) => prepared.mesh().outline_vertices.len(),
        Some(Expr::PathCompatibleMorphSequence { segments, .. }) => segments
            .first()
            .map_or(0, |segment| segment.mesh().outline_vertices.len()),
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
        Some(Expr::PathSector { .. }) => {
            crate::geometry::PATH_SECTOR_VERBS.saturating_mul(crate::geometry::PATH_CURVE_STEPS)
        }
        Some(Expr::PathArea { path, .. }) => {
            path_flattened_point_bound(exprs, *path).saturating_add(2)
        }
        Some(Expr::PathAreaBand { upper, lower }) => path_flattened_point_bound(exprs, *upper)
            .saturating_add(path_flattened_point_bound(exprs, *lower)),
        Some(Expr::PathOffset { path, .. }) => path_flattened_point_bound(exprs, *path),
        Some(Expr::PathResample { path, count }) => {
            path_contour_bound(exprs, *path).saturating_mul(usize::from(*count))
        }
        Some(Expr::PathReverse { path }) => path_flattened_point_bound(exprs, *path),
        Some(Expr::PathRoundCorners { path, .. }) => {
            path_point_bound(exprs, *path).saturating_mul(crate::geometry::PATH_CURVE_STEPS * 2)
        }
        Some(Expr::PathZigzag { ridges, .. }) => 2 * usize::from(*ridges) + 1,
        Some(Expr::PathNoiseDisplace { path, .. }) => {
            path_contour_bound(exprs, *path).saturating_mul(crate::geometry::POLAR_MORPH_POINTS)
        }
        Some(
            Expr::PathPuckerBloat { path, .. }
            | Expr::PathTwist { path, .. }
            | Expr::PathSimplify { path, .. },
        ) => {
            path_contour_bound(exprs, *path).saturating_mul(crate::geometry::PATH_MODIFIER_SAMPLES)
        }
        _ => MAX_FRAME_GEOMETRY_POINTS.saturating_mul(crate::geometry::PATH_CURVE_STEPS),
    }
}

fn path_contour_bound(exprs: &[Expr], id: ExprId) -> usize {
    match exprs.get(id.0 as usize) {
        Some(Expr::Const {
            value: MotionValue::PathData(path),
        }) => path
            .verbs
            .iter()
            .filter(|verb| **verb == PathVerb::Move)
            .count(),
        Some(Expr::PathTemplate { verbs, .. }) => {
            verbs.iter().filter(|verb| **verb == PathVerb::Move).count()
        }
        Some(
            Expr::PathLine { .. }
            | Expr::PathCubic { .. }
            | Expr::PathArc { .. }
            | Expr::PathArea { .. }
            | Expr::PathSector { .. }
            | Expr::PathAreaBand { .. },
        ) => 1,
        Some(Expr::PathMorph { from, to, .. }) => {
            path_contour_bound(exprs, *from).max(path_contour_bound(exprs, *to))
        }
        Some(Expr::PathCompatibleMorph { .. } | Expr::PathCompatibleMorphSequence { .. }) => 1,
        Some(
            Expr::PathOffset { path, .. }
            | Expr::PathResample { path, .. }
            | Expr::PathReverse { path }
            | Expr::PathRoundCorners { path, .. }
            | Expr::PathZigzag { path, .. }
            | Expr::PathNoiseDisplace { path, .. },
        ) => path_contour_bound(exprs, *path),
        Some(
            Expr::PathPuckerBloat { path, .. }
            | Expr::PathTwist { path, .. }
            | Expr::PathSimplify { path, .. },
        ) => path_contour_bound(exprs, *path),
        Some(Expr::PathStrokeToPath { path, .. }) => {
            path_contour_bound(exprs, *path).saturating_mul(2)
        }
        _ => MAX_FRAME_GEOMETRY_POINTS,
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
        Some(Expr::PathSector {
            center,
            inner,
            outer,
            start,
            end,
            corner_radius,
        }) => [*center, *inner, *outer, *start, *end, *corner_radius]
            .into_iter()
            .all(|input| frame_invariant(exprs, input)),
        Some(Expr::PathAreaBand { upper, lower }) => {
            frame_invariant(exprs, *upper) && frame_invariant(exprs, *lower)
        }
        Some(Expr::PathOffset { path, distance }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *distance)
        }
        Some(Expr::PathResample { path, .. }) => frame_invariant(exprs, *path),
        Some(Expr::PathReverse { path }) => frame_invariant(exprs, *path),
        Some(Expr::PathRoundCorners { path, radius }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *radius)
        }
        Some(Expr::PathZigzag { path, size, .. }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *size)
        }
        Some(Expr::PathNoiseDisplace {
            path,
            amount,
            frequency,
            phase,
            ..
        }) => [*path, *amount, *frequency, *phase]
            .into_iter()
            .all(|id| frame_invariant(exprs, id)),
        Some(Expr::PathPuckerBloat { path, amount }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *amount)
        }
        Some(Expr::PathTwist { path, angle }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *angle)
        }
        Some(Expr::PathSimplify { path, tolerance }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *tolerance)
        }
        Some(Expr::PathStrokeToPath { path, width }) => {
            frame_invariant(exprs, *path) && frame_invariant(exprs, *width)
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
        | Expr::PathCompatibleMorph { .. }
        | Expr::PathCompatibleMorphSequence { .. }
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
        | Expr::Interpolate { .. }
        | Expr::Select { .. } => GeometryEvalPolicy::FrameExpr,
        _ => return None,
    })
}
