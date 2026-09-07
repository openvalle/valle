//! Typed frame-expression lowering and Motion builtin expansion.

use super::*;

impl<'s> Compiler<'s> {
    /// Lower an expression and fold static subtrees using the runtime evaluator. Truncate newly
    /// added arena nodes after successful folding so a folded expression has the same artifact
    /// representation as its literal value.
    pub(super) fn lower_expr(&mut self, expression: &Expression<'_>) -> Option<ExprId> {
        let mark = self.expr_arena.values.len();
        let id = self.lower_expr_raw(expression)?;
        // Reuse existing constants.
        if matches!(
            self.expr_arena.values.get(id.0 as usize),
            Some(Expr::Const { .. })
        ) {
            return Some(id);
        }
        // Use incremental runtime-dependency flags to avoid quadratic scans of the arena during
        // folding.
        if self
            .expr_arena
            .reads_runtime
            .get(id.0 as usize)
            .copied()
            .unwrap_or(true)
        {
            return Some(id);
        }
        let Some(value) = valle_motion::fold_constants(&self.expr_arena.values, &self.controls)
            .get(id.0 as usize)
            .cloned()
            .flatten()
        else {
            return Some(id);
        };
        // Only truncate nodes added by this lowering operation.
        if id.0 as usize >= mark {
            self.expr_arena.values.truncate(mark);
            self.expr_arena.spans.truncate(mark);
            self.expr_arena.expansion_stacks.truncate(mark);
            self.expr_arena.reads_runtime.truncate(mark);
        }
        Some(self.push(Expr::Const { value }, expression.span()))
    }

    pub(super) fn lower_expr_raw(&mut self, expression: &Expression<'_>) -> Option<ExprId> {
        // Bound all expression recursion, including speculative folding, to protect the host stack.
        if self.expr_arena.depth >= MAX_EXPR_NESTING {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!(
                    "expression nesting exceeds the compiler budget of {MAX_EXPR_NESTING} \
                     levels; split the chain into intermediate `const`s, or precompute it at \
                     prepare time (an array + `.reduce`)"
                ),
            );
            return None;
        }
        self.expr_arena.depth += 1;
        let result = self.lower_expr_raw_inner(expression);
        self.expr_arena.depth -= 1;
        result
    }

    pub(super) fn lower_expr_raw_inner(&mut self, expression: &Expression<'_>) -> Option<ExprId> {
        let expression = strip_parens(expression);
        // Interpolated templates always enter the typed Template IR, even when every hole happens
        // to be prepare-time constant. Letting QuickJS coerce a Point/Rect/Path to
        // "[object Object]" would bypass the scalar CSS-token admission contract.
        let typed_template = matches!(
            expression,
            Expression::TemplateLiteral(template) if !template.expressions.is_empty()
        ) || matches!(
            expression,
            Expression::TaggedTemplateExpression(tagged)
                if matches!(&tagged.tag, Expression::Identifier(tag) if tag.name == "pathTemplate")
        );
        // Route admitted Math calls, including static calls, through shared Rust math to prevent
        // host-dependent transcendental results.
        let deterministic_math_alias = is_deterministic_math_alias(expression);
        if !typed_template
            && !deterministic_math_alias
            && let Some(value) = self.eval_static(expression)
            && let Some(value) = motion_value_from_json(&value)
        {
            return Some(self.push(Expr::Const { value }, expression.span()));
        }
        let expr = match expression {
            Expression::NumericLiteral(value) => Expr::Const {
                value: MotionValue::Number(value.value),
            },
            Expression::StringLiteral(value) => Expr::Const {
                value: motion_value_from_string(value.value.as_str()),
            },
            Expression::BooleanLiteral(value) => Expr::Const {
                value: MotionValue::Bool(value.value),
            },
            Expression::Identifier(identifier) => {
                if let Some(expr) = self.bindings.scalars.get(identifier.name.as_str()) {
                    return Some(*expr);
                } else if self.bindings.tuples.contains_key(identifier.name.as_str()) {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        format!(
                            "dynamic tuple `{}` must be read with a compile-time-known index",
                            identifier.name
                        ),
                    );
                    return None;
                } else if identifier.name == "TAU" {
                    Expr::Const {
                        value: MotionValue::Number(core::f64::consts::TAU),
                    }
                } else {
                    self.illegal(
                        DiagCode::UnknownIdentifier,
                        expression.span(),
                        format!("unknown runtime identifier `{}`", identifier.name),
                    );
                    return None;
                }
            }
            Expression::StaticMemberExpression(_) => return self.lower_member(expression),
            Expression::ComputedMemberExpression(member) => {
                return self.lower_dynamic_tuple_index(member);
            }
            Expression::UnaryExpression(unary)
                if unary.operator == oxc::ast::ast::UnaryOperator::UnaryNegation =>
            {
                let input = self.lower_expr(&unary.argument)?;
                Expr::Neg { input }
            }
            Expression::UnaryExpression(unary)
                if unary.operator == oxc::ast::ast::UnaryOperator::LogicalNot =>
            {
                let input = self.lower_expr(&unary.argument)?;
                return Some(self.not_expr(input, unary.span()));
            }
            Expression::BinaryExpression(binary) => {
                let lhs = self.lower_expr(&binary.left)?;
                let rhs = self.lower_expr(&binary.right)?;
                match binary.operator {
                    BinaryOperator::Addition => Expr::Add { lhs, rhs },
                    BinaryOperator::Subtraction => Expr::Sub { lhs, rhs },
                    BinaryOperator::Multiplication => Expr::Mul { lhs, rhs },
                    BinaryOperator::Division => Expr::Div { lhs, rhs },
                    BinaryOperator::Remainder => {
                        return self.lower_remainder(lhs, rhs, &binary.right, binary.span());
                    }
                    BinaryOperator::Equality | BinaryOperator::StrictEquality => Expr::Compare {
                        op: CompareOp::Eq,
                        lhs,
                        rhs,
                    },
                    BinaryOperator::Inequality | BinaryOperator::StrictInequality => {
                        Expr::Compare {
                            op: CompareOp::NotEq,
                            lhs,
                            rhs,
                        }
                    }
                    BinaryOperator::LessThan => Expr::Compare {
                        op: CompareOp::Lt,
                        lhs,
                        rhs,
                    },
                    BinaryOperator::LessEqualThan => Expr::Compare {
                        op: CompareOp::Lte,
                        lhs,
                        rhs,
                    },
                    BinaryOperator::GreaterThan => Expr::Compare {
                        op: CompareOp::Gt,
                        lhs,
                        rhs,
                    },
                    BinaryOperator::GreaterEqualThan => Expr::Compare {
                        op: CompareOp::Gte,
                        lhs,
                        rhs,
                    },
                    _ => {
                        self.unsupported(binary.span(), format!("binary operator `{}` is legal JavaScript but is not in the typed expression IR", binary.operator.as_str()));
                        return None;
                    }
                }
            }
            Expression::ConditionalExpression(conditional) => {
                let condition = self.lower_expr(&conditional.test)?;
                let when_true = self.lower_expr(&conditional.consequent)?;
                let when_false = self.lower_expr(&conditional.alternate)?;
                Expr::Select {
                    condition,
                    when_true,
                    when_false,
                }
            }
            Expression::LogicalExpression(logical) => {
                let lhs = self.lower_expr(&logical.left)?;
                let rhs = self.lower_expr(&logical.right)?;
                let bool_value = |this: &mut Self, value: bool| {
                    this.push(
                        Expr::Const {
                            value: MotionValue::Bool(value),
                        },
                        logical.span(),
                    )
                };
                match logical.operator {
                    LogicalOperator::And => Expr::Select {
                        condition: lhs,
                        when_true: rhs,
                        when_false: bool_value(self, false),
                    },
                    LogicalOperator::Or => Expr::Select {
                        condition: lhs,
                        when_true: bool_value(self, true),
                        when_false: rhs,
                    },
                    LogicalOperator::Coalesce => {
                        self.unsupported(
                            logical.span(),
                            "nullish coalescing is only admitted for prepare-time static values",
                        );
                        return None;
                    }
                }
            }
            Expression::TaggedTemplateExpression(tagged) => {
                return self.lower_path_template(tagged);
            }
            Expression::TemplateLiteral(template) => {
                if template.quasis.len() != template.expressions.len() + 1 {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        template.span,
                        "malformed template literal",
                    );
                    return None;
                }
                let mut parts = Vec::with_capacity(
                    template
                        .quasis
                        .len()
                        .saturating_add(template.expressions.len()),
                );
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let Some(cooked) = quasi.value.cooked else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            quasi.span,
                            "template literal contains an invalid escape",
                        );
                        return None;
                    };
                    if !cooked.is_empty() {
                        parts.push(valle_motion::TemplatePart::Text {
                            value: cooked.to_string(),
                        });
                    }
                    if let Some(expression) = template.expressions.get(index) {
                        parts.push(valle_motion::TemplatePart::Expr {
                            expr: self.lower_expr(expression)?,
                        });
                    }
                }
                Expr::Template { parts }
            }
            Expression::CallExpression(call) => {
                let Expression::Identifier(callee) = &call.callee else {
                    // Precisely specified Math operations may fold with static inputs; report
                    // runtime-dependent calls explicitly.
                    if let Expression::StaticMemberExpression(member) = &call.callee
                        && let Expression::Identifier(object) = &member.object
                        && object.name == "Math"
                    {
                        if let Some(result) = self.lower_math_alias(
                            member.property.name.as_str(),
                            &call.arguments,
                            call.span(),
                        ) {
                            return result;
                        }
                        // Compose exactly specified min, max, and abs from existing runtime
                        // primitives.
                        if let Some(result) = self.lower_math_composed(
                            member.property.name.as_str(),
                            &call.arguments,
                            call.span(),
                        ) {
                            return result;
                        }
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            format!(
                                "`Math.{}` folds only when its inputs are static; frame-time \
                                 expressions evaluate named motion primitives only — express \
                                 the curve with `interpolate` (+ easing), or use the exactly \
                                 specified `Math.min` / `Math.max` / `Math.abs` / `clamp`",
                                member.property.name
                            ),
                        );
                        return None;
                    }
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        call.callee.span(),
                        "only named motion primitives may be called at runtime",
                    );
                    return None;
                };
                match callee.name.as_str() {
                    "useTheme" => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            if self.current_theme.is_some() {
                                "useTheme() is prepare-time only; bind it first with `const theme = useTheme()` and combine the resulting static token with frame expressions"
                            } else {
                                "useTheme() requires an enclosing <ThemeProvider value={...}> compile-time scope"
                            },
                        );
                        return None;
                    }
                    "interpolate" => return self.lower_interpolate(&call.arguments, call.span()),
                    "spring" => return self.lower_spring(&call.arguments, call.span()),
                    "stageProgress" => {
                        let arguments = call.arguments.iter().collect::<Vec<_>>();
                        return self.lower_stage_progress(&arguments, call.span(), false);
                    }
                    "staggerProgress" => {
                        let arguments = call.arguments.iter().collect::<Vec<_>>();
                        return self.lower_stage_progress(&arguments, call.span(), true);
                    }
                    "repeatProgress" => {
                        return self.lower_sequence_loop(&call.arguments, call.span(), false);
                    }
                    "yoyoProgress" => {
                        return self.lower_sequence_loop(&call.arguments, call.span(), true);
                    }
                    "freezeFrame" => {
                        return self.lower_freeze_frame(&call.arguments, call.span());
                    }
                    "sin" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Sin,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "exp" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Exp,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "cos" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Cos,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "floor" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Floor,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "ceil" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Ceil,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "round" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Round,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "fract" => {
                        return self.lower_math_unary(
                            MathUnaryOp::Fract,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "atan2" => {
                        return self.lower_math_binary(
                            MathBinaryOp::Atan2,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "pow" => {
                        return self.lower_math_binary(
                            MathBinaryOp::Pow,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "mod" => {
                        return self.lower_math_binary(
                            MathBinaryOp::Mod,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "pingPong" => {
                        return self.lower_math_binary(
                            MathBinaryOp::PingPong,
                            &call.arguments,
                            call.span(),
                        );
                    }
                    "noise1d" => return self.lower_noise(&call.arguments, call.span(), false),
                    "noise2d" => return self.lower_noise(&call.arguments, call.span(), true),
                    "wiggle" => return self.lower_wiggle(&call.arguments, call.span()),
                    "wiggle2D" => return self.lower_wiggle_2d(&call.arguments, call.span()),
                    "deg" => return self.lower_angle_number(&call.arguments, call.span(), true),
                    "rad" => return self.lower_angle_number(&call.arguments, call.span(), false),
                    "trail" => return self.lower_trail(&call.arguments, call.span()),
                    "autoRotate" => {
                        return self.lower_auto_rotate(&call.arguments, call.span());
                    }
                    "formatNumber" => {
                        return self.lower_number_format(&call.arguments, call.span(), "number");
                    }
                    "formatPercent" => {
                        return self.lower_number_format(&call.arguments, call.span(), "percent");
                    }
                    "padNumber" => {
                        return self.lower_number_format(&call.arguments, call.span(), "pad");
                    }
                    "clamp" => return self.lower_clamp(&call.arguments, call.span()),
                    "point" => return self.lower_point(&call.arguments, call.span()),
                    "rect" => return self.lower_rect(&call.arguments, call.span()),
                    "line" => return self.lower_line(&call.arguments, call.span()),
                    "cubic" => return self.lower_cubic(&call.arguments, call.span()),
                    "arc" => return self.lower_arc(&call.arguments, call.span()),
                    "area" => return self.lower_area(&call.arguments, call.span()),
                    "offsetPath" => {
                        return self.lower_offset_path(&call.arguments, call.span());
                    }
                    "boolean" => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            "boolean(left, right, op) changes topology and requires prepare-time static PathData inputs",
                        );
                        return None;
                    }
                    "motionPath" | "follow" => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            "motionPath/follow(path, progress) is only valid as style.motionPath",
                        );
                        return None;
                    }
                    "morphPath" => return self.lower_morph_path(&call.arguments, call.span()),
                    "bounds" => return self.lower_bounds(&call.arguments, call.span()),
                    "project3d" => return self.lower_project3d(&call.arguments, call.span()),
                    "anchor" => return self.lower_anchor(&call.arguments, call.span()),
                    "connect" => return self.lower_connect(&call.arguments, call.span()),
                    "pointAt" => {
                        return self.lower_path_sample(&call.arguments, call.span(), false);
                    }
                    "tangentAt" => {
                        return self.lower_path_sample(&call.arguments, call.span(), true);
                    }
                    "pathLength" => return self.lower_path_length(&call.arguments, call.span()),
                    "pathTrajectory" => {
                        return self.lower_path_trajectory(&call.arguments, call.span());
                    }
                    "path" => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            "path(svgD) requires prepare-time static SVG data",
                        );
                        return None;
                    }
                    // Distinguish missing measurement fonts from frame-dependent measurement
                    // inputs.
                    "measureText" => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            call.span(),
                            if self.has_measure {
                                "measureText is a prepare-time capability: its arguments must be \
                                 statically known. Frame-varying text/size cannot be measured at \
                                 compile time — measure the static extremes instead"
                            } else {
                                "measureText needs a font bundle at compile time; pass \
                                 `--font <path>` so measurement uses the same fonts as rendering"
                            },
                        );
                        return None;
                    }
                    other => {
                        if let Some((function, captures_scope)) = self.authored_fn(other)
                            && !is_component_name(other)
                        {
                            return self.lower_dynamic_helper(
                                other,
                                function,
                                captures_scope,
                                &call.arguments,
                                call.span(),
                            );
                        }
                        // Give separate guidance for prepare-only computation builtins and authored
                        // helpers.
                        let advice = if valle_motion::compute::AUTHOR_SURFACE.contains(&other) {
                            format!(
                                "`{other}` is a prepare-time builtin: it runs once at compile \
                                 time, so every argument must be known then. `ctx.*` (viewport, \
                                 phase, frame) is only known per frame. Compute the layout in a \
                                 fixed Canvas space and scale it at frame time — \
                                 `const S = {other}(…)` at module scope, then \
                                 `S.map(i) * (ctx.viewport.width / CANVAS_W)` inside the component"
                            )
                        } else {
                            format!(
                                "helper `{other}` cannot take frame-varying arguments here; its \
                                 body is executed at prepare time. Either pass only \
                                 compile-time-known arguments, or inline the expression at the \
                                 call site so it lowers into the frame-time IR"
                            )
                        };
                        self.unsupported(call.span(), advice);
                        return None;
                    }
                }
            }
            _ => {
                self.unsupported(
                    expression.span(),
                    "this side-effect-free expression shape is not in the typed expression IR subset",
                );
                return None;
            }
        };
        Some(self.push(expr, expression.span()))
    }

    /// Lower runtime min, max, and abs into Compare/Select primitives without changing the wire
    /// format. DAG references share operands; artifact validation enforces numeric ordered
    /// comparisons. Return None for unrelated names.
    pub(super) fn lower_math_composed(
        &mut self,
        member: &str,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<Option<ExprId>> {
        let op = match member {
            "min" => CompareOp::Lt,
            "max" => CompareOp::Gt,
            "abs" => {
                let mut inputs = match self.composed_arguments(member, arguments, span) {
                    Some(inputs) => inputs,
                    None => return Some(None),
                };
                if inputs.len() != 1 {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "`Math.abs` takes exactly one argument",
                    );
                    return Some(None);
                }
                let input = inputs.remove(0);
                let zero = self.push(
                    Expr::Const {
                        value: MotionValue::Number(0.0),
                    },
                    span,
                );
                let negative = self.push(
                    Expr::Compare {
                        op: CompareOp::Lt,
                        lhs: input,
                        rhs: zero,
                    },
                    span,
                );
                let flipped = self.push(Expr::Neg { input }, span);
                return Some(Some(self.push(
                    Expr::Select {
                        condition: negative,
                        when_true: flipped,
                        when_false: input,
                    },
                    span,
                )));
            }
            _ => return None,
        };
        let inputs = match self.composed_arguments(member, arguments, span) {
            Some(inputs) => inputs,
            None => return Some(None),
        };
        if inputs.is_empty() {
            // Reject an empty min/max call before it can produce a non-finite constant.
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("`Math.{member}` needs at least one argument"),
            );
            return Some(None);
        }
        Some(Some(self.fold_pairwise(op, inputs, span)))
    }

    /// Lower all arguments or fail the call. Reject nonnumeric constants here for a call-specific
    /// diagnostic; artifact validation checks dynamic types.
    pub(super) fn composed_arguments(
        &mut self,
        name: &str,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<Vec<ExprId>> {
        let mut inputs = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let Some(expression) = argument.as_expression() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("`{name}` arguments cannot spread"),
                );
                return None;
            };
            let id = self.lower_expr(expression)?;
            if let Some(Expr::Const { value }) = self.expr_arena.values.get(id.0 as usize)
                && !matches!(value, MotionValue::Number(_))
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!(
                        "`{name}` compares its arguments, so every one must be a plain number; \
                         this one is {}",
                        value_kind_name(value)
                    ),
                );
                return None;
            }
            inputs.push(id);
        }
        Some(inputs)
    }

    /// Reduce inputs left-associatively into a Compare/Select chain.
    pub(super) fn fold_pairwise(
        &mut self,
        op: CompareOp,
        inputs: Vec<ExprId>,
        span: Span,
    ) -> ExprId {
        let mut iter = inputs.into_iter();
        let mut accumulator = iter.next().expect("caller rejects the empty case");
        for next in iter {
            let wins = self.push(
                Expr::Compare {
                    op,
                    lhs: accumulator,
                    rhs: next,
                },
                span,
            );
            accumulator = self.push(
                Expr::Select {
                    condition: wins,
                    when_true: accumulator,
                    when_false: next,
                },
                span,
            );
        }
        accumulator
    }

    /// Lower clamp(value, low, high) as min(max(value, low), high).
    pub(super) fn lower_clamp(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        let inputs = self.composed_arguments("clamp", arguments, span)?;
        let [value, low, high] = inputs.as_slice() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "clamp(value, low, high) takes exactly three arguments",
            );
            return None;
        };
        // Reject reversed bounds when both are compile-time constants.
        if let (Some(Expr::Const { value: low_value }), Some(Expr::Const { value: high_value })) = (
            self.expr_arena.values.get(low.0 as usize).cloned(),
            self.expr_arena.values.get(high.0 as usize).cloned(),
        ) && let (MotionValue::Number(low_value), MotionValue::Number(high_value)) =
            (low_value, high_value)
            && low_value > high_value
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "clamp(value, low, high) needs low <= high, but low is {low_value} and \
                     high is {high_value}"
                ),
            );
            return None;
        }
        let lifted = self.fold_pairwise(CompareOp::Gt, vec![*value, *low], span);
        Some(self.fold_pairwise(CompareOp::Lt, vec![lifted, *high], span))
    }

    pub(super) fn lower_member(&mut self, expression: &Expression<'_>) -> Option<ExprId> {
        if let Expression::StaticMemberExpression(member) = expression
            && !matches!(
                strip_parens(&member.object),
                Expression::Identifier(_) | Expression::StaticMemberExpression(_)
            )
        {
            let input = self.lower_expr(&member.object)?;
            return self.lower_geometry_field(
                input,
                member.property.name.as_str(),
                expression.span(),
            );
        }
        let mut segments = Vec::new();
        let mut current = expression;
        loop {
            match current {
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.to_string());
                    current = &member.object;
                }
                Expression::Identifier(identifier) => {
                    segments.push(identifier.name.to_string());
                    break;
                }
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        "only static `ctx.*` and `props.*` paths are allowed",
                    );
                    return None;
                }
            }
        }
        segments.reverse();
        match segments.as_slice() {
            [local, field] if self.bindings.scalars.contains_key(local) => {
                let input = self.bindings.scalars[local];
                self.lower_geometry_field(input, field, expression.span())
            }
            [root, name, field] if root == "props" => {
                let input = if let Some(props) = &self.bindings.component_props {
                    match props.get(name).cloned() {
                        Some(AuthorValue::Dynamic(expr)) => expr,
                        Some(AuthorValue::DynamicTuple(_)) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                expression.span(),
                                format!(
                                    "component prop {name} is a dynamic tuple; read it with a compile-time-known index"
                                ),
                            );
                            return None;
                        }
                        Some(AuthorValue::Static(value)) => {
                            let value = motion_value_from_json(&value).or_else(|| {
                                self.unsupported(
                                    expression.span(),
                                    format!(
                                        "component prop {name} is structured prepare data without a typed geometry value"
                                    ),
                                );
                                None
                            })?;
                            self.push(Expr::Const { value }, expression.span())
                        }
                        _ => {
                            self.illegal(
                                DiagCode::UnknownProp,
                                expression.span(),
                                format!("component prop {name} was not provided as geometry"),
                            );
                            return None;
                        }
                    }
                } else {
                    if !self.controls.props.contains_key(name) {
                        self.illegal(
                            DiagCode::UnknownProp,
                            expression.span(),
                            format!("props.{name} is not declared in controls.props"),
                        );
                        return None;
                    }
                    self.push(Expr::Prop { name: name.clone() }, expression.span())
                };
                self.lower_geometry_field(input, field, expression.span())
            }
            [root, name] if root == "props" => {
                if let Some(props) = &self.bindings.component_props {
                    return match props.get(name).cloned() {
                        Some(AuthorValue::Dynamic(expr)) => Some(expr),
                        Some(AuthorValue::DynamicTuple(_)) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                expression.span(),
                                format!(
                                    "component prop `{name}` is a dynamic tuple and cannot be used as a scalar expression"
                                ),
                            );
                            None
                        }
                        Some(AuthorValue::Static(value)) => motion_value_from_json(&value)
                            .map(|value| self.push(Expr::Const { value }, expression.span()))
                            .or_else(|| {
                                self.unsupported(
                                    expression.span(),
                                    format!("component prop `{name}` is structured prepare data and cannot be used as a scalar expression"),
                                );
                                None
                            }),
                        Some(AuthorValue::Children(_)) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                expression.span(),
                                "children can only appear in JSX child position",
                            );
                            None
                        }
                        None => {
                            self.illegal(
                                DiagCode::UnknownProp,
                                expression.span(),
                                format!("component prop `{name}` was not provided or defaulted"),
                            );
                            None
                        }
                    };
                }
                if !self.controls.props.contains_key(name) {
                    self.illegal(
                        DiagCode::UnknownProp,
                        expression.span(),
                        format!("`props.{name}` is not declared in controls.props"),
                    );
                    None
                } else {
                    Some(self.push(Expr::Prop { name: name.clone() }, expression.span()))
                }
            }
            [root, rest @ ..] if root == "ctx" => {
                let input = context_input(rest).or_else(|| {
                    self.illegal(
                        DiagCode::UnknownContextPath,
                        expression.span(),
                        format!("`ctx.{}` is not a MotionContext scalar", rest.join(".")),
                    );
                    None
                })?;
                // Declare viewport capability only when used so unsupported consumers reject the
                // artifact.
                if matches!(
                    input,
                    ContextInput::ViewportWidth | ContextInput::ViewportHeight
                ) {
                    self.extra_capabilities
                        .insert(VIEWPORT_CAPABILITY.to_owned());
                }
                Some(self.push(Expr::Context { input }, expression.span()))
            }
            [root, name, field] if root == "signals" => {
                if self.controls.cues.contains_key(name) {
                    let field = match field.as_str() {
                        "active" => CueField::Active,
                        "progress" => CueField::Progress,
                        "enter" => CueField::Enter,
                        "hold" => CueField::Hold,
                        "exit" => CueField::Exit,
                        "localFrame" => CueField::LocalFrame,
                        _ => {
                            self.illegal(
                                DiagCode::UnknownProp,
                                expression.span(),
                                format!("`signals.{name}.{field}` is not a CueState scalar"),
                            );
                            return None;
                        }
                    };
                    Some(self.push(
                        Expr::Cue {
                            name: name.clone(),
                            field,
                        },
                        expression.span(),
                    ))
                } else {
                    self.illegal(
                        DiagCode::UnknownProp,
                        expression.span(),
                        format!("`signals.{name}` is not a declared cue"),
                    );
                    None
                }
            }
            [root, ..] if root == "signals" => {
                self.illegal(
                    DiagCode::UnknownProp,
                    expression.span(),
                    "signals must use `signals.<cue>.<active|progress|enter|hold|exit|localFrame>`",
                );
                None
            }
            _ => {
                self.illegal(
                    DiagCode::UnknownIdentifier,
                    expression.span(),
                    "runtime member access must start with ctx, props, or signals",
                );
                None
            }
        }
    }

    pub(super) fn lower_dynamic_tuple_index(
        &mut self,
        member: &oxc::ast::ast::ComputedMemberExpression<'_>,
    ) -> Option<ExprId> {
        let Some(values) = self.dynamic_tuple_value(&member.object) else {
            self.unsupported(
                member.span,
                "computed member access is only supported for fixed-length dynamic tuples",
            );
            return None;
        };
        let Some(index_value) = self.eval_static(&member.expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.expression.span(),
                "dynamic tuple index must be known at compile time",
            );
            return None;
        };
        let Some(index_number) = index_value.as_f64() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.expression.span(),
                "dynamic tuple index must be a non-negative integer",
            );
            return None;
        };
        if !index_number.is_finite()
            || index_number < 0.0
            || index_number.fract() != 0.0
            || index_number > usize::MAX as f64
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.expression.span(),
                "dynamic tuple index must be a non-negative integer",
            );
            return None;
        }
        let index = index_number as usize;
        values.get(index).copied().or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.expression.span(),
                format!(
                    "dynamic tuple index {index} is out of bounds for length {}",
                    values.len()
                ),
            );
            None
        })
    }

    pub(super) fn lower_geometry_field(
        &mut self,
        input: ExprId,
        field: &str,
        span: Span,
    ) -> Option<ExprId> {
        let field = match field {
            "x" => GeometryField::X,
            "y" => GeometryField::Y,
            "width" => GeometryField::Width,
            "height" => GeometryField::Height,
            other => {
                self.illegal(
                    DiagCode::UnknownProp,
                    span,
                    format!("geometry field {other} is unknown; use x/y or Rect width/height"),
                );
                return None;
            }
        };
        Some(self.push(Expr::GeometryField { input, field }, span))
    }
}
