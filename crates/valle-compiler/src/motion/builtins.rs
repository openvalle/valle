//! Sequence, deterministic math, modifiers, and formatting builtins.

use super::*;

impl<'s> Compiler<'s> {
    /// Freeze a component-local frame clock over one compile-time interval.
    ///
    /// This is authoring sugar for
    /// `frame < from ? frame : frame < to ? from : frame - (to - from)`. It intentionally grows
    /// no new wire node: the result is composed from the existing compare/select/arithmetic Exprs.
    pub(super) fn lower_freeze_frame(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [frame_argument, options_argument] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "freezeFrame(frame, { from, to }) takes exactly two arguments",
            );
            return None;
        };
        let frame_expression = frame_argument.as_expression()?;
        let Some(Expression::ObjectExpression(options)) =
            options_argument.as_expression().map(strip_parens)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                options_argument.span(),
                "freezeFrame's second argument must be an object literal: { from, to }",
            );
            return None;
        };

        let mut from = None;
        let mut to = None;
        for entry in &options.properties {
            let ObjectPropertyKind::ObjectProperty(property) = entry else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    entry.span(),
                    "freezeFrame options cannot spread",
                );
                return None;
            };
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    "freezeFrame option names must be static identifiers",
                );
                return None;
            };
            let slot = match name.as_str() {
                "from" => &mut from,
                "to" => &mut to,
                other => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.key.span(),
                        format!(
                            "unknown freezeFrame option `{other}`; only `from` and `to` are admitted"
                        ),
                    );
                    return None;
                }
            };
            if slot.is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("freezeFrame option `{name}` is duplicated"),
                );
                return None;
            }
            let Some(value) = self.fold_to_number(&property.value) else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    property.value.span(),
                    format!("freezeFrame `{name}` must be an integer known at compile time"),
                );
                return None;
            };
            *slot = Some((value, property.value.span()));
        }

        let (Some((from_value, from_span)), Some((to_value, to_span))) = (from, to) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                options_argument.span(),
                "freezeFrame requires both `from` and `to`",
            );
            return None;
        };
        if from_value.fract() != 0.0
            || to_value.fract() != 0.0
            || from_value < 0.0
            || to_value <= from_value
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_argument.span(),
                "freezeFrame needs integer frames with 0 <= from < to",
            );
            return None;
        }

        let frame = self.lower_expr(frame_expression)?;
        let from = self.push(
            Expr::Const {
                value: MotionValue::Number(from_value),
            },
            from_span,
        );
        let to = self.push(
            Expr::Const {
                value: MotionValue::Number(to_value),
            },
            to_span,
        );
        let duration = self.push(
            Expr::Const {
                value: MotionValue::Number(to_value - from_value),
            },
            options_argument.span(),
        );
        let before = self.push(
            Expr::Compare {
                op: CompareOp::Lt,
                lhs: frame,
                rhs: from,
            },
            span,
        );
        let inside = self.push(
            Expr::Compare {
                op: CompareOp::Lt,
                lhs: frame,
                rhs: to,
            },
            span,
        );
        let after = self.push(
            Expr::Sub {
                lhs: frame,
                rhs: duration,
            },
            span,
        );
        let held_or_after = self.push(
            Expr::Select {
                condition: inside,
                when_true: from,
                when_false: after,
            },
            span,
        );
        Some(self.push(
            Expr::Select {
                condition: before,
                when_true: frame,
                when_false: held_or_after,
            },
            span,
        ))
    }

    /// Lower a named Sequence stage to arithmetic and clamping. Only resolved start and duration
    /// constants enter the Artifact.
    pub(super) fn lower_stage_progress(
        &mut self,
        arguments: &[&Argument<'_>],
        span: Span,
        staggered: bool,
    ) -> Option<ExprId> {
        let expected = if staggered { 5 } else { 3 };
        let name = if staggered {
            "staggerProgress"
        } else {
            "stageProgress"
        };
        if arguments.len() != expected {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} takes exactly {expected} arguments"),
            );
            return None;
        }
        let frame_expression = arguments[0].as_expression()?;
        let fps_expression = arguments[1].as_expression()?;
        if self.src(fps_expression) != "ctx.fps" {
            self.illegal(
                DiagCode::GrammarForbidden,
                fps_expression.span(),
                format!(
                    "{name}'s second argument must be exactly `ctx.fps`; Sequence seconds use the versioned frame × fps.den / fps.num conversion"
                ),
            );
            return None;
        }
        let stage_expression = arguments[2].as_expression()?;
        let stage_value = self.eval_static(stage_expression).or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                stage_expression.span(),
                format!("{name}'s stage must come from a prepare-time defineSequence result"),
            );
            None
        })?;
        let stage: SequenceStage = serde_json::from_value(stage_value).ok().or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                stage_expression.span(),
                format!("{name}'s stage must be a named stage from defineSequence"),
            );
            None
        })?;
        if stage.marker != "sequenceStage"
            || !stage.start.is_finite()
            || stage.start < 0.0
            || !stage.duration.is_finite()
            || stage.duration <= 0.0
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                stage_expression.span(),
                format!("{name} received an invalid Sequence stage"),
            );
            return None;
        }

        let frame = self.lower_expr(frame_expression)?;
        let sample = match stage.unit {
            SequenceUnit::Frames => frame,
            SequenceUnit::Seconds => {
                // Keep the authored contract's exact operation order: frame × den / num.
                let den = self.push(
                    Expr::Context {
                        input: ContextInput::FpsDen,
                    },
                    fps_expression.span(),
                );
                let numerator = self.push(
                    Expr::Mul {
                        lhs: frame,
                        rhs: den,
                    },
                    span,
                );
                let num = self.push(
                    Expr::Context {
                        input: ContextInput::FpsNum,
                    },
                    fps_expression.span(),
                );
                self.push(
                    Expr::Div {
                        lhs: numerator,
                        rhs: num,
                    },
                    span,
                )
            }
        };
        let start = self.push(
            Expr::Const {
                value: MotionValue::Number(stage.start),
            },
            stage_expression.span(),
        );
        let start = if staggered {
            let index_expression = arguments[3].as_expression()?;
            let index = self.lower_expr(index_expression)?;
            let gap_expression = arguments[4].as_expression()?;
            let gap_value = self.eval_static(gap_expression).or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    gap_expression.span(),
                    "staggerProgress gap must be a prepare-time seconds(...) or frames(...) value",
                );
                None
            })?;
            let gap: SequenceTime = serde_json::from_value(gap_value).ok().or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    gap_expression.span(),
                    "staggerProgress gap must use seconds(...) or frames(...)",
                );
                None
            })?;
            if gap.marker != "sequenceTime"
                || gap.unit != stage.unit
                || !gap.value.is_finite()
                || gap.value < 0.0
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    gap_expression.span(),
                    "staggerProgress gap must be finite, non-negative, and use the stage's unit",
                );
                return None;
            }
            let gap = self.push(
                Expr::Const {
                    value: MotionValue::Number(gap.value),
                },
                gap_expression.span(),
            );
            let offset = self.push(
                Expr::Mul {
                    lhs: index,
                    rhs: gap,
                },
                span,
            );
            self.push(
                Expr::Add {
                    lhs: start,
                    rhs: offset,
                },
                span,
            )
        } else {
            start
        };
        let elapsed = self.push(
            Expr::Sub {
                lhs: sample,
                rhs: start,
            },
            span,
        );
        let duration = self.push(
            Expr::Const {
                value: MotionValue::Number(stage.duration),
            },
            stage_expression.span(),
        );
        let raw = self.push(
            Expr::Div {
                lhs: elapsed,
                rhs: duration,
            },
            span,
        );
        let zero = self.push(
            Expr::Const {
                value: MotionValue::Number(0.0),
            },
            span,
        );
        let one = self.push(
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            span,
        );
        let lifted = self.fold_pairwise(CompareOp::Gt, vec![raw, zero], span);
        Some(self.fold_pairwise(CompareOp::Lt, vec![lifted, one], span))
    }

    /// Express repeat and yoyo with shared math expressions. The stage sampler retains time
    /// conversion and clamping.
    pub(super) fn lower_sequence_loop(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        yoyo: bool,
    ) -> Option<ExprId> {
        let name = if yoyo {
            "yoyoProgress"
        } else {
            "repeatProgress"
        };
        let [frame, fps, stage, cycles] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name}(frame, fps, stage, cycles) takes exactly four arguments"),
            );
            return None;
        };
        let Some(cycles_value) = self.fold_to_number(cycles.as_expression()?) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                cycles.span(),
                format!("{name} cycles must be an integer known at compile time"),
            );
            return None;
        };
        if cycles_value.fract() != 0.0 || !(1.0..=1024.0).contains(&cycles_value) {
            self.illegal(
                DiagCode::BuiltinRejected,
                cycles.span(),
                format!("{name} cycles must be an integer in 1..=1024"),
            );
            return None;
        }
        let base_arguments = [frame, fps, stage];
        let base = self.lower_stage_progress(&base_arguments, span, false)?;
        let cycles = self.push(
            Expr::Const {
                value: MotionValue::Number(cycles_value),
            },
            cycles.span(),
        );
        let scaled = self.push(
            Expr::Mul {
                lhs: base,
                rhs: cycles,
            },
            span,
        );
        let one = self.push(
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            span,
        );
        let looped = self.push(
            Expr::MathBinary {
                op: if yoyo {
                    MathBinaryOp::PingPong
                } else {
                    MathBinaryOp::Mod
                },
                lhs: scaled,
                rhs: one,
            },
            span,
        );
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        if yoyo {
            return Some(looped);
        }
        // Repeat returns the completed state at the right edge instead of snapping to zero.
        let done = self.push(
            Expr::Compare {
                op: CompareOp::Gte,
                lhs: base,
                rhs: one,
            },
            span,
        );
        Some(self.push(
            Expr::Select {
                condition: done,
                when_true: one,
                when_false: looped,
            },
            span,
        ))
    }

    pub(super) fn lower_math_unary(
        &mut self,
        op: MathUnaryOp,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [argument] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "this math builtin takes exactly one argument",
            );
            return None;
        };
        let input = self.lower_expr(argument.as_expression()?)?;
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(Expr::MathUnary { op, input }, span))
    }

    /// Resolve admitted `Math.*` aliases through deterministic Motion math. `None` means
    /// unsupported; `Some(None)` means malformed and already diagnosed.
    pub(super) fn lower_math_alias(
        &mut self,
        member: &str,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<Option<ExprId>> {
        let unary = match member {
            "sqrt" => Some(MathUnaryOp::Sqrt),
            "exp" => Some(MathUnaryOp::Exp),
            "sin" => Some(MathUnaryOp::Sin),
            "cos" => Some(MathUnaryOp::Cos),
            "tan" => Some(MathUnaryOp::Tan),
            "floor" => Some(MathUnaryOp::Floor),
            "ceil" => Some(MathUnaryOp::Ceil),
            "round" => Some(MathUnaryOp::Round),
            "trunc" => Some(MathUnaryOp::Trunc),
            _ => None,
        };
        if let Some(op) = unary {
            return Some(self.lower_math_unary(op, arguments, span));
        }
        if member == "atan2" {
            return Some(self.lower_math_binary(MathBinaryOp::Atan2, arguments, span));
        }
        if member == "pow" {
            return Some(self.lower_math_binary(MathBinaryOp::Pow, arguments, span));
        }
        None
    }

    /// JavaScript `%` is remainder (quotient truncated toward zero), not the existing positive-
    /// period `mod()` helper. Keep it as an explicit typed op: composing division/truncation would
    /// be observably wrong when a finite `lhs / rhs` overflows to infinity.
    pub(super) fn lower_remainder(
        &mut self,
        lhs: ExprId,
        rhs: ExprId,
        rhs_expression: &Expression<'_>,
        span: Span,
    ) -> Option<ExprId> {
        if self.fold_to_number(rhs_expression) == Some(0.0) {
            self.illegal(
                DiagCode::BuiltinRejected,
                rhs_expression.span(),
                "remainder divisor must not be zero",
            );
            return None;
        }
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(
            Expr::MathBinary {
                op: MathBinaryOp::Remainder,
                lhs,
                rhs,
            },
            span,
        ))
    }

    /// `deg(number)` converts degrees to the radians used by geometry constructors; `rad(number)`
    /// is the explicit identity. Both disappear into ordinary arithmetic before the Artifact.
    pub(super) fn lower_angle_number(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        degrees: bool,
    ) -> Option<ExprId> {
        let [argument] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                if degrees {
                    "deg(value) takes exactly one numeric argument"
                } else {
                    "rad(value) takes exactly one numeric argument"
                },
            );
            return None;
        };
        let input = self.lower_expr(argument.as_expression()?)?;
        if !degrees {
            return Some(input);
        }
        let factor = self.push(
            Expr::Const {
                value: MotionValue::Number(core::f64::consts::PI / 180.0),
            },
            span,
        );
        Some(self.push(
            Expr::Mul {
                lhs: input,
                rhs: factor,
            },
            span,
        ))
    }

    pub(super) fn lower_math_binary(
        &mut self,
        op: MathBinaryOp,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [lhs, rhs] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "this math builtin takes exactly two arguments",
            );
            return None;
        };
        if matches!(op, MathBinaryOp::Mod | MathBinaryOp::PingPong)
            && let Some(period) = self.fold_to_number(rhs.as_expression()?)
            && period <= 0.0
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                rhs.span(),
                match op {
                    MathBinaryOp::Mod => "mod period must be finite and positive",
                    MathBinaryOp::PingPong => "pingPong length must be finite and positive",
                    MathBinaryOp::Atan2 | MathBinaryOp::Pow | MathBinaryOp::Remainder => {
                        unreachable!()
                    }
                },
            );
            return None;
        }
        let lhs = self.lower_expr(lhs.as_expression()?)?;
        let rhs = self.lower_expr(rhs.as_expression()?)?;
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(Expr::MathBinary { op, lhs, rhs }, span))
    }

    pub(super) fn lower_noise(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        two_dimensional: bool,
    ) -> Option<ExprId> {
        let expected = if two_dimensional { 3 } else { 2 };
        if arguments.len() != expected {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "noise{}d takes exactly {expected} arguments",
                    if two_dimensional { 2 } else { 1 }
                ),
            );
            return None;
        }
        let seed_expression = arguments[0].as_expression()?;
        let Some(seed) = self.fold_to_number(seed_expression) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                seed_expression.span(),
                "noise seed must be a non-negative integer known at compile time",
            );
            return None;
        };
        if seed < 0.0 || seed.fract() != 0.0 || seed >= 18_446_744_073_709_551_616.0 {
            self.illegal(
                DiagCode::BuiltinRejected,
                seed_expression.span(),
                "noise seed must be a non-negative integer below 2^64",
            );
            return None;
        }
        let x = self.lower_expr(arguments[1].as_expression()?)?;
        let expr = if two_dimensional {
            let y = self.lower_expr(arguments[2].as_expression()?)?;
            Expr::Noise2D {
                seed: seed as u64,
                x,
                y,
            }
        } else {
            Expr::Noise1D {
                seed: seed as u64,
                x,
            }
        };
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(expr, span))
    }

    /// Frame-rate-independent value noise lowered to Noise1D and arithmetic, without a runtime
    /// modifier object.
    pub(super) fn lower_wiggle(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [frame, fps, options] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "wiggle(frame, fps, options) takes exactly three arguments",
            );
            return None;
        };
        let fps_expression = fps.as_expression()?;
        if self.src(fps_expression) != "ctx.fps" {
            self.illegal(
                DiagCode::GrammarForbidden,
                fps_expression.span(),
                "wiggle's second argument must be exactly `ctx.fps` so frequency is measured in physical seconds",
            );
            return None;
        }
        let options_expression = options.as_expression()?;
        let options = self.eval_static(options_expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle options must be known at compile time",
            );
            None
        })?;
        let Some(options) = options.as_object() else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle options must be an object",
            );
            return None;
        };
        if let Some(unknown) = options
            .keys()
            .find(|key| !matches!(key.as_str(), "seed" | "frequency" | "amplitude" | "phase"))
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                format!("unknown wiggle option `{unknown}`"),
            );
            return None;
        }
        let Some(seed) = options.get("seed").and_then(serde_json::Value::as_f64) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle seed must be a non-negative integer below 2^64",
            );
            return None;
        };
        if !seed.is_finite()
            || seed < 0.0
            || seed.fract() != 0.0
            || seed >= 18_446_744_073_709_551_616.0
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle seed must be a non-negative integer below 2^64",
            );
            return None;
        }
        let Some(frequency) = options
            .get("frequency")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle frequency must be finite and positive",
            );
            return None;
        };
        let Some(amplitude) = options
            .get("amplitude")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle amplitude must be finite and non-negative",
            );
            return None;
        };
        let Some(phase) = options
            .get("phase")
            .map_or(Some(0.0), serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle phase must be finite",
            );
            return None;
        };

        let frame = self.lower_expr(frame.as_expression()?)?;
        let fps_den = self.push(
            Expr::Context {
                input: ContextInput::FpsDen,
            },
            fps_expression.span(),
        );
        let frame_den = self.push(
            Expr::Mul {
                lhs: frame,
                rhs: fps_den,
            },
            span,
        );
        let fps_num = self.push(
            Expr::Context {
                input: ContextInput::FpsNum,
            },
            fps_expression.span(),
        );
        let seconds = self.push(
            Expr::Div {
                lhs: frame_den,
                rhs: fps_num,
            },
            span,
        );
        let frequency = self.push(
            Expr::Const {
                value: MotionValue::Number(frequency),
            },
            options_expression.span(),
        );
        let x = self.push(
            Expr::Mul {
                lhs: seconds,
                rhs: frequency,
            },
            span,
        );
        let phase = self.push(
            Expr::Const {
                value: MotionValue::Number(phase),
            },
            options_expression.span(),
        );
        let x = self.push(Expr::Add { lhs: x, rhs: phase }, span);
        let noise = self.push(
            Expr::Noise1D {
                seed: seed as u64,
                x,
            },
            span,
        );
        let two = self.push(
            Expr::Const {
                value: MotionValue::Number(2.0),
            },
            span,
        );
        let doubled = self.push(
            Expr::Mul {
                lhs: noise,
                rhs: two,
            },
            span,
        );
        let one = self.push(
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            span,
        );
        let centered = self.push(
            Expr::Sub {
                lhs: doubled,
                rhs: one,
            },
            span,
        );
        let amplitude = self.push(
            Expr::Const {
                value: MotionValue::Number(amplitude),
            },
            options_expression.span(),
        );
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(
            Expr::Mul {
                lhs: centered,
                rhs: amplitude,
            },
            span,
        ))
    }

    /// Construct a point from two decorrelated deterministic Noise1D samples.
    pub(super) fn lower_wiggle_2d(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [frame, fps, options] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "wiggle2D(frame, fps, options) takes exactly three arguments",
            );
            return None;
        };
        let fps_expression = fps.as_expression()?;
        if self.src(fps_expression) != "ctx.fps" {
            self.illegal(
                DiagCode::GrammarForbidden,
                fps_expression.span(),
                "wiggle2D's second argument must be exactly `ctx.fps`",
            );
            return None;
        }
        let options_expression = options.as_expression()?;
        let options = self.eval_static(options_expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D options must be known at compile time",
            );
            None
        })?;
        let Some(options) = options.as_object() else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D options must be an object",
            );
            return None;
        };
        if let Some(unknown) = options
            .keys()
            .find(|key| !matches!(key.as_str(), "seed" | "frequency" | "amplitude" | "phase"))
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                format!("unknown wiggle2D option `{unknown}`"),
            );
            return None;
        }
        let Some(seed) = options.get("seed").and_then(serde_json::Value::as_f64) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D seed must be a non-negative integer below 2^64",
            );
            return None;
        };
        if !seed.is_finite()
            || seed < 0.0
            || seed.fract() != 0.0
            || seed >= 18_446_744_073_709_551_616.0
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D seed must be a non-negative integer below 2^64",
            );
            return None;
        }
        let Some(frequency) = options
            .get("frequency")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D frequency must be finite and positive",
            );
            return None;
        };
        let Some(amplitudes) = options
            .get("amplitude")
            .and_then(serde_json::Value::as_array)
            .filter(|values| values.len() == 2)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D amplitude must be a static [x, y] pair",
            );
            return None;
        };
        let amplitudes = amplitudes
            .iter()
            .map(serde_json::Value::as_f64)
            .collect::<Option<Vec<_>>>();
        let Some(amplitudes) = amplitudes.filter(|values| {
            values
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
        }) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D amplitude values must be finite and non-negative",
            );
            return None;
        };
        let Some(phase) = options
            .get("phase")
            .map_or(Some(0.0), serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "wiggle2D phase must be finite",
            );
            return None;
        };

        let frame = self.lower_expr(frame.as_expression()?)?;
        let fps_den = self.push(
            Expr::Context {
                input: ContextInput::FpsDen,
            },
            fps_expression.span(),
        );
        let frame_den = self.push(
            Expr::Mul {
                lhs: frame,
                rhs: fps_den,
            },
            span,
        );
        let fps_num = self.push(
            Expr::Context {
                input: ContextInput::FpsNum,
            },
            fps_expression.span(),
        );
        let seconds = self.push(
            Expr::Div {
                lhs: frame_den,
                rhs: fps_num,
            },
            span,
        );
        let frequency = self.push(
            Expr::Const {
                value: MotionValue::Number(frequency),
            },
            options_expression.span(),
        );
        let phase_input = self.push(
            Expr::Mul {
                lhs: seconds,
                rhs: frequency,
            },
            span,
        );
        let phase = self.push(
            Expr::Const {
                value: MotionValue::Number(phase),
            },
            options_expression.span(),
        );
        let phase_input = self.push(
            Expr::Add {
                lhs: phase_input,
                rhs: phase,
            },
            span,
        );
        let channel = |compiler: &mut Self, seed: u64, amplitude: f64| {
            let noise = compiler.push(
                Expr::Noise1D {
                    seed,
                    x: phase_input,
                },
                span,
            );
            let two = compiler.push(
                Expr::Const {
                    value: MotionValue::Number(2.0),
                },
                span,
            );
            let doubled = compiler.push(
                Expr::Mul {
                    lhs: noise,
                    rhs: two,
                },
                span,
            );
            let one = compiler.push(
                Expr::Const {
                    value: MotionValue::Number(1.0),
                },
                span,
            );
            let centered = compiler.push(
                Expr::Sub {
                    lhs: doubled,
                    rhs: one,
                },
                span,
            );
            let amplitude = compiler.push(
                Expr::Const {
                    value: MotionValue::Number(amplitude),
                },
                options_expression.span(),
            );
            compiler.push(
                Expr::Mul {
                    lhs: centered,
                    rhs: amplitude,
                },
                span,
            )
        };
        let seed = seed as u64;
        let x = channel(self, seed, amplitudes[0]);
        let y = channel(self, seed ^ 0x9e37_79b9_7f4a_7c15, amplitudes[1]);
        self.extra_capabilities
            .insert(MOTION_MATH_CAPABILITY.to_owned());
        Some(self.push(Expr::MakePoint { x, y }, span))
    }

    /// Express deterministic history sampling as a progress offset without frame state.
    pub(super) fn lower_trail(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        let [progress, index, options] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "trail(progress, index, options) takes exactly three arguments",
            );
            return None;
        };
        let options_expression = options.as_expression()?;
        let options = self.eval_static(options_expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "trail options must be known at compile time",
            );
            None
        })?;
        let Some(options) = options.as_object() else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "trail options must be an object",
            );
            return None;
        };
        if let Some(unknown) = options
            .keys()
            .find(|key| !matches!(key.as_str(), "gap" | "mode"))
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                format!("unknown trail option `{unknown}`"),
            );
            return None;
        }
        let Some(gap) = options
            .get("gap")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "trail gap must be finite and non-negative",
            );
            return None;
        };
        let mode = options
            .get("mode")
            .map_or(Some("clamp"), serde_json::Value::as_str);
        let Some(mode @ ("clamp" | "wrap")) = mode else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options_expression.span(),
                "trail mode must be `clamp` or `wrap`",
            );
            return None;
        };
        let progress = self.lower_expr(progress.as_expression()?)?;
        let index = self.lower_expr(index.as_expression()?)?;
        let gap = self.push(
            Expr::Const {
                value: MotionValue::Number(gap),
            },
            options_expression.span(),
        );
        let offset = self.push(
            Expr::Mul {
                lhs: index,
                rhs: gap,
            },
            span,
        );
        let raw = self.push(
            Expr::Sub {
                lhs: progress,
                rhs: offset,
            },
            span,
        );
        let zero = self.push(
            Expr::Const {
                value: MotionValue::Number(0.0),
            },
            span,
        );
        let one = self.push(
            Expr::Const {
                value: MotionValue::Number(1.0),
            },
            span,
        );
        if mode == "wrap" {
            self.extra_capabilities
                .insert(MOTION_MATH_CAPABILITY.to_owned());
            return Some(self.push(
                Expr::MathBinary {
                    op: MathBinaryOp::Mod,
                    lhs: raw,
                    rhs: one,
                },
                span,
            ));
        }
        let below = self.push(
            Expr::Compare {
                op: CompareOp::Lt,
                lhs: raw,
                rhs: zero,
            },
            span,
        );
        let lifted = self.push(
            Expr::Select {
                condition: below,
                when_true: zero,
                when_false: raw,
            },
            span,
        );
        let above = self.push(
            Expr::Compare {
                op: CompareOp::Gt,
                lhs: lifted,
                rhs: one,
            },
            span,
        );
        Some(self.push(
            Expr::Select {
                condition: above,
                when_true: one,
                when_false: lifted,
            },
            span,
        ))
    }

    /// Expose the motion-path tangent as a typed angle.
    pub(super) fn lower_auto_rotate(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [path, progress] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "autoRotate(path, progress) takes exactly two arguments; use follow(..., { angleOffset }) when path positioning also needs an offset",
            );
            return None;
        };
        let path = self.lower_expr(path.as_expression()?)?;
        let progress = self.lower_expr(progress.as_expression()?)?;
        Some(self.push(Expr::PathAngleAt { path, progress }, span))
    }

    pub(super) fn lower_number_format(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        kind: &str,
    ) -> Option<ExprId> {
        if !(1..=2).contains(&arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "number formatting takes a value and one optional static options object",
            );
            return None;
        }
        let options = if let Some(options) = arguments.get(1) {
            let expression = options.as_expression()?;
            self.eval_static(expression).or_else(|| {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    expression.span(),
                    "number format options must be known at compile time",
                );
                None
            })?
        } else {
            serde_json::json!({})
        };
        let Some(options) = options.as_object() else {
            self.illegal(
                DiagCode::BuiltinRejected,
                arguments.get(1).map_or(span, GetSpan::span),
                "number format options must be an object",
            );
            return None;
        };
        let allowed: &[&str] = if kind == "pad" {
            &["width"]
        } else {
            &["decimals", "grouping"]
        };
        if let Some(unknown) = options.keys().find(|key| !allowed.contains(&key.as_str())) {
            self.illegal(
                DiagCode::BuiltinRejected,
                arguments.get(1).map_or(span, GetSpan::span),
                format!("unknown number format option `{unknown}`"),
            );
            return None;
        }
        let format = if kind == "pad" {
            let Some(width) = options.get("width").and_then(serde_json::Value::as_u64) else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "padNumber width must be an integer in 1..=64",
                );
                return None;
            };
            let Ok(width) = u8::try_from(width) else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "padNumber width must be an integer in 1..=64",
                );
                return None;
            };
            if !(1..=64).contains(&width) {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "padNumber width must be an integer in 1..=64",
                );
                return None;
            }
            NumberFormat::Pad { width }
        } else {
            let decimals = options
                .get("decimals")
                .map_or(Some(0), serde_json::Value::as_u64);
            let Some(decimals) = decimals.and_then(|value| u8::try_from(value).ok()) else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "decimals must be an integer in 0..=12",
                );
                return None;
            };
            if decimals > 12 {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "decimals must be an integer in 0..=12",
                );
                return None;
            }
            let grouping = options
                .get("grouping")
                .map_or(Some(false), serde_json::Value::as_bool);
            let Some(grouping) = grouping else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    arguments.get(1).map_or(span, GetSpan::span),
                    "grouping must be a static boolean",
                );
                return None;
            };
            if kind == "number" {
                NumberFormat::Number { decimals, grouping }
            } else {
                NumberFormat::Percent { decimals, grouping }
            }
        };
        let input = self.lower_expr(arguments[0].as_expression()?)?;
        self.extra_capabilities
            .insert(NUMBER_FORMAT_CAPABILITY.to_owned());
        Some(self.push(Expr::FormatNumber { input, format }, span))
    }
}
