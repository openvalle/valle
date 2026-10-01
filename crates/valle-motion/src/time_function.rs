//! Analyzable numeric functions of one Motion time input.
//!
//! The affine coefficients describe the mathematical function. Evaluation retains the authored
//! expression tree and its floating-point operation order, so a threshold at a frame boundary
//! does not move when the analysis is used for activation.

use crate::{CompareOp, ContextInput, Expr, ExprId, MotionContext, MotionValue};

const MAX_TIME_FUNCTION_NODES: usize = 64;

#[derive(Debug, Clone)]
enum TimeExpr {
    Constant(f64),
    Input,
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Div(Box<Self>, Box<Self>),
    Neg(Box<Self>),
}

impl TimeExpr {
    fn evaluate(&self, time: f64) -> f64 {
        match self {
            Self::Constant(value) => *value,
            Self::Input => time,
            Self::Add(lhs, rhs) => lhs.evaluate(time) + rhs.evaluate(time),
            Self::Sub(lhs, rhs) => lhs.evaluate(time) - rhs.evaluate(time),
            Self::Mul(lhs, rhs) => lhs.evaluate(time) * rhs.evaluate(time),
            Self::Div(lhs, rhs) => lhs.evaluate(time) / rhs.evaluate(time),
            Self::Neg(input) => -input.evaluate(time),
        }
    }

    fn simple_driver(&self) -> Option<f64> {
        match self {
            Self::Input => Some(1.0),
            Self::Mul(lhs, rhs) => match (&**lhs, &**rhs) {
                (Self::Input, Self::Constant(scale)) | (Self::Constant(scale), Self::Input) => {
                    Some(*scale)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn encode(&self, key: &mut Vec<u64>) {
        match self {
            Self::Constant(value) => key.extend([0, value.to_bits()]),
            Self::Input => key.push(1),
            Self::Add(lhs, rhs) => {
                key.push(2);
                lhs.encode(key);
                rhs.encode(key);
            }
            Self::Sub(lhs, rhs) => {
                key.push(3);
                lhs.encode(key);
                rhs.encode(key);
            }
            Self::Mul(lhs, rhs) => {
                key.push(4);
                lhs.encode(key);
                rhs.encode(key);
            }
            Self::Div(lhs, rhs) => {
                key.push(5);
                lhs.encode(key);
                rhs.encode(key);
            }
            Self::Neg(input) => {
                key.push(6);
                input.encode(key);
            }
        }
    }

    fn range(&self, time: ValueRange) -> Option<ValueRange> {
        match self {
            Self::Constant(value) => ValueRange::new(*value, *value),
            Self::Input => Some(time),
            Self::Add(lhs, rhs) => {
                let a = lhs.range(time)?;
                let b = rhs.range(time)?;
                ValueRange::outward(a.lower + b.lower, a.upper + b.upper)
            }
            Self::Sub(lhs, rhs) => {
                let a = lhs.range(time)?;
                let b = rhs.range(time)?;
                ValueRange::outward(a.lower - b.upper, a.upper - b.lower)
            }
            Self::Mul(lhs, rhs) => lhs.range(time)?.corners(rhs.range(time)?, |a, b| a * b),
            Self::Div(lhs, rhs) => {
                let divisor = rhs.range(time)?;
                if divisor.lower <= 0.0 && divisor.upper >= 0.0 {
                    return None;
                }
                lhs.range(time)?.corners(divisor, |a, b| a / b)
            }
            Self::Neg(input) => {
                let range = input.range(time)?;
                ValueRange::new(-range.upper, -range.lower)
            }
        }
    }
}

/// Inclusive numeric enclosure for every finite input in an inclusive time interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueRange {
    pub lower: f64,
    pub upper: f64,
}

impl ValueRange {
    pub fn new(lower: f64, upper: f64) -> Option<Self> {
        (lower.is_finite() && upper.is_finite() && lower <= upper).then_some(Self { lower, upper })
    }

    pub fn outward(lower: f64, upper: f64) -> Option<Self> {
        Self::new(lower.next_down(), upper.next_up())
    }

    pub fn corners(self, other: Self, operation: impl Fn(f64, f64) -> f64) -> Option<Self> {
        let values = [
            operation(self.lower, other.lower),
            operation(self.lower, other.upper),
            operation(self.upper, other.lower),
            operation(self.upper, other.upper),
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let lower = values.iter().copied().fold(f64::INFINITY, f64::min);
        let upper = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::outward(lower, upper)
    }
}

/// Conservative numeric bounds for parameter validation. Unknown nodes remain
/// unknown; no finite sampling is treated as proof. Clamp/min/max use their
/// lowered comparison structure, so explicit clamps can bound unknown inputs.
pub fn numeric_range(
    exprs: &[Expr],
    root: ExprId,
    context: &impl Fn(ContextInput) -> Option<ValueRange>,
) -> Option<ValueRange> {
    fn visit(
        exprs: &[Expr],
        id: ExprId,
        context: &impl Fn(ContextInput) -> Option<ValueRange>,
        budget: &mut usize,
    ) -> Option<ValueRange> {
        *budget = budget.checked_sub(1)?;
        let mut child = |id| visit(exprs, id, context, budget);
        let union = |a: ValueRange, b: ValueRange| {
            ValueRange::new(a.lower.min(b.lower), a.upper.max(b.upper))
        };
        match exprs.get(id.0 as usize)? {
            Expr::Const {
                value: MotionValue::Number(n),
            } => ValueRange::new(*n, *n),
            Expr::Context { input } => context(*input),
            Expr::Add { lhs, rhs } => {
                let a = child(*lhs)?;
                let b = child(*rhs)?;
                ValueRange::outward(a.lower + b.lower, a.upper + b.upper)
            }
            Expr::Sub { lhs, rhs } => {
                let a = child(*lhs)?;
                let b = child(*rhs)?;
                ValueRange::outward(a.lower - b.upper, a.upper - b.lower)
            }
            Expr::Mul { lhs, rhs } => child(*lhs)?.corners(child(*rhs)?, |a, b| a * b),
            Expr::Div { lhs, rhs } => {
                let a = child(*lhs)?;
                let b = child(*rhs)?;
                if b.lower <= 0.0 && b.upper >= 0.0 {
                    None
                } else {
                    a.corners(b, |a, b| a / b)
                }
            }
            Expr::Neg { input } => {
                let r = child(*input)?;
                ValueRange::new(-r.upper, -r.lower)
            }
            Expr::Select {
                condition,
                when_true,
                when_false,
            } => {
                if let Some(Expr::Compare { op, lhs, rhs }) = exprs.get(condition.0 as usize)
                    && lhs == when_true
                    && rhs == when_false
                    && matches!(
                        op,
                        CompareOp::Lt | CompareOp::Lte | CompareOp::Gt | CompareOp::Gte
                    )
                {
                    let unknown = ValueRange {
                        lower: f64::NEG_INFINITY,
                        upper: f64::INFINITY,
                    };
                    let a = child(*lhs).unwrap_or(unknown);
                    let b = child(*rhs).unwrap_or(unknown);
                    return Some(if matches!(op, CompareOp::Lt | CompareOp::Lte) {
                        ValueRange {
                            lower: a.lower.min(b.lower),
                            upper: a.upper.min(b.upper),
                        }
                    } else {
                        ValueRange {
                            lower: a.lower.max(b.lower),
                            upper: a.upper.max(b.upper),
                        }
                    });
                }
                if let Some(Expr::Const {
                    value: MotionValue::Bool(value),
                }) = exprs.get(condition.0 as usize)
                {
                    child(if *value { *when_true } else { *when_false })
                } else {
                    union(child(*when_true)?, child(*when_false)?)
                }
            }
            Expr::MathUnary { op, input } => {
                use crate::MathUnaryOp::*;
                if matches!(op, Sin | Cos) {
                    return ValueRange::new(-1.0, 1.0);
                }
                let r = child(*input)?;
                match op {
                    Floor => ValueRange::new(r.lower.floor(), r.upper.floor()),
                    Ceil => ValueRange::new(r.lower.ceil(), r.upper.ceil()),
                    Round => ValueRange::new(r.lower.round(), r.upper.round()),
                    Trunc => ValueRange::new(r.lower.trunc(), r.upper.trunc()),
                    Sqrt if r.lower >= 0.0 => ValueRange::outward(
                        valle_draw::math::sqrt(r.lower),
                        valle_draw::math::sqrt(r.upper),
                    ),
                    _ => None,
                }
            }
            Expr::Noise1D { .. } | Expr::Noise2D { .. } => ValueRange::new(-1.0, 1.0),
            Expr::SimulationSample { samples, .. } | Expr::AudioSample { samples, .. } => {
                ValueRange::new(
                    samples.iter().copied().fold(f64::INFINITY, f64::min),
                    samples.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            }
            Expr::Spring {
                mass,
                stiffness,
                damping,
                initial_velocity,
                output,
                ..
            } => {
                let omega = valle_draw::math::sqrt(stiffness / mass);
                let zeta = damping / (2.0 * valle_draw::math::sqrt(stiffness * mass));
                if *output == crate::spring::SpringOutput::Velocity {
                    let energy = valle_draw::math::hypot(omega, *initial_velocity);
                    return ValueRange::outward(-energy, energy);
                }
                if *initial_velocity == 0.0 {
                    let overshoot = if zeta >= 1.0 {
                        0.0
                    } else {
                        valle_draw::math::exp(
                            -std::f64::consts::PI * zeta
                                / valle_draw::math::sqrt(1.0 - zeta * zeta),
                        )
                    };
                    // Zero and rest are exact in the runtime. A small outward
                    // margin covers libm rounding in the analytic peak.
                    ValueRange::new(0.0, (1.0 + overshoot).next_up())
                } else {
                    let energy = valle_draw::math::sqrt(
                        1.0 + mass * initial_velocity * initial_velocity / stiffness,
                    );
                    ValueRange::outward(1.0 - energy, 1.0 + energy)
                }
            }
            _ => None,
        }
    }
    if let Some(function) = TimeFunction::compile(exprs, root) {
        if let Some(time) = context(function.input()) {
            return function.range_on(time.lower, time.upper);
        }
    }
    let result = visit(exprs, root, context, &mut 512)?;
    ValueRange::new(result.lower, result.upper)
}

#[derive(Debug, Clone, Copy)]
struct Affine {
    input: Option<ContextInput>,
    slope: f64,
    offset: f64,
}

impl Affine {
    fn new(input: Option<ContextInput>, slope: f64, offset: f64) -> Option<Self> {
        (slope.is_finite() && offset.is_finite()).then_some(Self {
            input,
            slope,
            offset,
        })
    }

    fn input_with(self, other: Self) -> Option<Option<ContextInput>> {
        match (self.input, other.input) {
            (Some(left), Some(right)) if left != right => None,
            (Some(input), _) | (_, Some(input)) => Some(Some(input)),
            (None, None) => Some(None),
        }
    }
}

/// A numeric expression proven affine in one context time input. Unsupported arithmetic and
/// mixed time inputs return `None`, leaving those expressions on the regular evaluator path.
#[derive(Debug, Clone)]
pub struct TimeFunction {
    tree: TimeExpr,
    input: ContextInput,
    slope: f64,
    offset: f64,
    nodes: usize,
    key: Vec<u64>,
}

impl TimeFunction {
    pub fn compile(exprs: &[Expr], root: ExprId) -> Option<Self> {
        let mut remaining = MAX_TIME_FUNCTION_NODES;
        let (tree, affine, nodes) = lower(exprs, root, &mut remaining)?;
        let input = affine.input?;
        let mut key = vec![match input {
            ContextInput::CompositionSeconds => 0,
            ContextInput::LocalFrame => 1,
            _ => unreachable!("time functions only admit seconds or local frames"),
        }];
        tree.encode(&mut key);
        Some(Self {
            tree,
            input,
            slope: affine.slope,
            offset: affine.offset,
            nodes,
            key,
        })
    }

    pub fn input(&self) -> ContextInput {
        self.input
    }

    /// Mathematical affine coefficients; runtime evaluation uses the original operation order.
    pub fn coefficients(&self) -> (f64, f64) {
        (self.slope, self.offset)
    }

    /// Mathematical derivative with respect to the selected time input.
    pub fn derivative(&self) -> f64 {
        self.slope
    }

    pub fn evaluate(&self, ctx: &MotionContext) -> f64 {
        let time = match self.input {
            ContextInput::CompositionSeconds => ctx.sample.composition().as_f64(),
            ContextInput::LocalFrame => f64::from(ctx.local_frame),
            _ => unreachable!("time functions only admit seconds or local frames"),
        };
        self.tree.evaluate(time)
    }

    /// Conservative enclosure over an inclusive interval of the selected time input. Every
    /// arithmetic step rounds its bounds outward; an overflow or zero-crossing division returns
    /// `None` rather than claiming a range that might miss a runtime value.
    pub fn range_on(&self, start: f64, end: f64) -> Option<ValueRange> {
        self.tree.range(ValueRange::new(start, end)?)
    }

    /// Prove a comparison has the same value throughout an inclusive input interval.
    /// `None` means the interval crosses the threshold or the enclosure is unavailable.
    pub fn compare_on_interval(
        &self,
        start: f64,
        end: f64,
        op: CompareOp,
        threshold: f64,
    ) -> Option<bool> {
        if !threshold.is_finite() {
            return None;
        }
        let range = self.range_on(start, end)?;
        match op {
            CompareOp::Eq if range.upper < threshold || range.lower > threshold => Some(false),
            CompareOp::Eq if range.lower == threshold && range.upper == threshold => Some(true),
            CompareOp::NotEq if range.upper < threshold || range.lower > threshold => Some(true),
            CompareOp::NotEq if range.lower == threshold && range.upper == threshold => Some(false),
            CompareOp::Lt if range.upper < threshold => Some(true),
            CompareOp::Lt if range.lower >= threshold => Some(false),
            CompareOp::Lte if range.upper <= threshold => Some(true),
            CompareOp::Lte if range.lower > threshold => Some(false),
            CompareOp::Gt if range.lower > threshold => Some(true),
            CompareOp::Gt if range.upper <= threshold => Some(false),
            CompareOp::Gte if range.lower >= threshold => Some(true),
            CompareOp::Gte if range.upper < threshold => Some(false),
            _ => None,
        }
    }

    pub(crate) fn simple_driver(&self) -> Option<(ContextInput, f64)> {
        self.tree.simple_driver().map(|scale| (self.input, scale))
    }

    pub(crate) fn operation_count(&self) -> usize {
        self.nodes
    }

    pub(crate) fn structural_key(&self) -> &Vec<u64> {
        &self.key
    }
}

fn lower(exprs: &[Expr], id: ExprId, remaining: &mut usize) -> Option<(TimeExpr, Affine, usize)> {
    *remaining = remaining.checked_sub(1)?;
    let expr = exprs.get(id.0 as usize)?;
    match expr {
        Expr::Const {
            value: MotionValue::Number(value),
        } if value.is_finite() => Some((
            TimeExpr::Constant(*value),
            Affine::new(None, 0.0, *value)?,
            1,
        )),
        Expr::Context { input }
            if matches!(
                input,
                ContextInput::CompositionSeconds | ContextInput::LocalFrame
            ) =>
        {
            Some((TimeExpr::Input, Affine::new(Some(*input), 1.0, 0.0)?, 1))
        }
        Expr::Neg { input } => {
            let (tree, affine, count) = lower(exprs, *input, remaining)?;
            Some((
                TimeExpr::Neg(Box::new(tree)),
                Affine::new(affine.input, -affine.slope, -affine.offset)?,
                count + 1,
            ))
        }
        Expr::Add { lhs, rhs }
        | Expr::Sub { lhs, rhs }
        | Expr::Mul { lhs, rhs }
        | Expr::Div { lhs, rhs } => {
            let (left, a, left_count) = lower(exprs, *lhs, remaining)?;
            let (right, b, right_count) = lower(exprs, *rhs, remaining)?;
            let (tree, affine) = match expr {
                Expr::Add { .. } => (
                    TimeExpr::Add(Box::new(left), Box::new(right)),
                    Affine::new(a.input_with(b)?, a.slope + b.slope, a.offset + b.offset)?,
                ),
                Expr::Sub { .. } => (
                    TimeExpr::Sub(Box::new(left), Box::new(right)),
                    Affine::new(a.input_with(b)?, a.slope - b.slope, a.offset - b.offset)?,
                ),
                Expr::Mul { .. } if a.input.is_none() => (
                    TimeExpr::Mul(Box::new(left), Box::new(right)),
                    Affine::new(b.input, a.offset * b.slope, a.offset * b.offset)?,
                ),
                Expr::Mul { .. } if b.input.is_none() => (
                    TimeExpr::Mul(Box::new(left), Box::new(right)),
                    Affine::new(a.input, a.slope * b.offset, a.offset * b.offset)?,
                ),
                Expr::Div { .. } if b.input.is_none() && b.offset != 0.0 => (
                    TimeExpr::Div(Box::new(left), Box::new(right)),
                    Affine::new(a.input, a.slope / b.offset, a.offset / b.offset)?,
                ),
                _ => return None,
            };
            Some((tree, affine, left_count + right_count + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_context_at_frame;
    use valle_timeline::FrameRate;

    #[test]
    fn affine_analysis_preserves_the_original_arithmetic_order() {
        let exprs = vec![
            Expr::Context {
                input: ContextInput::CompositionSeconds,
            },
            Expr::Const {
                value: MotionValue::Number(0.2),
            },
            Expr::Sub {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Const {
                value: MotionValue::Number(1000.0),
            },
            Expr::Mul {
                lhs: ExprId(2),
                rhs: ExprId(3),
            },
        ];
        let function = TimeFunction::compile(&exprs, ExprId(4)).unwrap();
        assert_eq!(function.input(), ContextInput::CompositionSeconds);
        assert_eq!(function.coefficients(), (1000.0, -200.0));
        assert_eq!(function.derivative(), 1000.0);
        for frame in [0, 1, 6, 8, 30] {
            let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let time = ctx.sample.composition().as_f64();
            assert_eq!(
                function.evaluate(&ctx).to_bits(),
                ((time - 0.2) * 1000.0).to_bits()
            );
        }
    }

    #[test]
    fn only_one_affine_time_input_is_admitted() {
        let exprs = vec![
            Expr::Context {
                input: ContextInput::CompositionSeconds,
            },
            Expr::Context {
                input: ContextInput::LocalFrame,
            },
            Expr::Mul {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Add {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
        ];
        assert!(TimeFunction::compile(&exprs, ExprId(2)).is_none());
        assert!(TimeFunction::compile(&exprs, ExprId(3)).is_none());
    }

    #[test]
    fn outward_ranges_enclose_samples_and_leave_threshold_boundaries_unproven() {
        let exprs = vec![
            Expr::Context {
                input: ContextInput::CompositionSeconds,
            },
            Expr::Const {
                value: MotionValue::Number(0.2),
            },
            Expr::Sub {
                lhs: ExprId(0),
                rhs: ExprId(1),
            },
            Expr::Const {
                value: MotionValue::Number(1000.0),
            },
            Expr::Mul {
                lhs: ExprId(2),
                rhs: ExprId(3),
            },
        ];
        let function = TimeFunction::compile(&exprs, ExprId(4)).unwrap();
        let range = function.range_on(0.0, 0.4).unwrap();
        for step in 0..=400 {
            let time = f64::from(step) / 1000.0;
            let value = (time - 0.2) * 1000.0;
            assert!(range.lower <= value && value <= range.upper, "t={time}");
        }
        assert_eq!(
            function.compare_on_interval(0.0, 0.1, CompareOp::Gte, 0.0),
            Some(false)
        );
        assert_eq!(
            function.compare_on_interval(0.3, 0.4, CompareOp::Gte, 0.0),
            Some(true)
        );
        assert_eq!(
            function.compare_on_interval(0.2, 0.2, CompareOp::Gte, 0.0),
            None
        );
    }
}
