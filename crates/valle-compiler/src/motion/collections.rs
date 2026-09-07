//! Fixed collections, compile-time map expansion, interpolate, spring, and easing.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn collect_interpolate_inputs(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<Vec<f64>> {
        if self.array_shape_illegal(expression, "interpolate inputRange") {
            return None;
        }
        if let Expression::ArrayExpression(_) = peel_expr(expression) {
            return array_items(expression)?
                .into_iter()
                .map(|item| {
                    static_number(item)
                        .or_else(|| self.fold_to_number(item))
                        .or_else(|| {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                item.span(),
                                "interpolate input stops must be finite numbers known at compile time \
                                 (literals, or arithmetic over module constants — anything depending on \
                                 ctx/props/cues cannot be a stop)",
                            );
                            None
                        })
                })
                .collect();
        }
        if let Expression::Identifier(identifier) = peel_expr(expression)
            && let Some(init) = self
                .module_const_inits
                .get(identifier.name.as_str())
                .copied()
        {
            return self.collect_interpolate_inputs(init);
        }
        match self.eval_static(expression) {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_f64().filter(|value| value.is_finite()).or_else(|| {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            expression.span(),
                            "interpolate inputRange must be a compile-time array of finite numbers",
                        );
                        None
                    })
                })
                .collect(),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "interpolate inputRange must be a static array literal or an immutable compile-time constant array",
                );
                None
            }
        }
    }

    pub(super) fn collect_interpolate_outputs(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<Vec<MotionValue>> {
        if self.array_shape_illegal(expression, "interpolate outputRange") {
            return None;
        }
        if let Expression::ArrayExpression(_) = peel_expr(expression) {
            return array_items(expression)?
                .into_iter()
                .map(|item| {
                    static_motion_value(item)
                        .or_else(|| self.fold_to_value(item))
                        .or_else(|| {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                item.span(),
                                "interpolate outputs must be scalars known at compile time (literals, or \
                                 arithmetic over module constants — anything depending on ctx/props/cues \
                                 cannot be an output stop)",
                            );
                            None
                        })
                })
                .collect();
        }
        if let Expression::Identifier(identifier) = peel_expr(expression)
            && let Some(init) = self
                .module_const_inits
                .get(identifier.name.as_str())
                .copied()
        {
            return self.collect_interpolate_outputs(init);
        }
        match self.eval_static(expression) {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| {
                    motion_value_from_json(item).or_else(|| {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            expression.span(),
                            "interpolate outputRange must be a compile-time array of scalars",
                        );
                        None
                    })
                })
                .collect(),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "interpolate outputRange must be a static array literal or an immutable compile-time constant array",
                );
                None
            }
        }
    }

    /// Fail-closed on spread, holes, and circular const aliases. Returns true
    /// when a diagnostic was pushed.
    pub(super) fn array_shape_illegal(&mut self, expression: &Expression<'_>, role: &str) -> bool {
        let mut seen = BTreeSet::new();
        self.array_shape_illegal_inner(expression, role, &mut seen)
    }

    pub(super) fn array_shape_illegal_inner(
        &mut self,
        expression: &Expression<'_>,
        role: &str,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        match peel_expr(expression) {
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    match element {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                spread.span(),
                                format!(
                                    "{role} cannot use spread; the expanded length must be a compile-time-fixed list"
                                ),
                            );
                            return true;
                        }
                        ArrayExpressionElement::Elision(elision) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                elision.span(),
                                format!("{role} cannot contain holes"),
                            );
                            return true;
                        }
                        _ => {}
                    }
                }
                false
            }
            Expression::Identifier(identifier) => {
                let name = identifier.name.as_str();
                if !seen.insert(name.to_string()) {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        identifier.span(),
                        format!("{role} constant `{name}` is circular"),
                    );
                    return true;
                }
                self.module_const_inits
                    .get(name)
                    .copied()
                    .is_some_and(|init| self.array_shape_illegal_inner(init, role, seen))
            }
            _ => {
                if let Some(call) = as_map_call(expression)
                    && let Expression::StaticMemberExpression(member) = &call.callee
                {
                    return self.array_shape_illegal_inner(&member.object, role, seen);
                }
                false
            }
        }
    }

    pub(super) fn collection_depends_on_runtime(&self, expression: &Expression<'_>) -> bool {
        self.shadowed_by_dynamic(expression)
            || referenced_identifiers(expression)
                .iter()
                .any(|name| matches!(name.as_str(), "ctx" | "props" | "signals" | "data"))
    }

    /// Lower a fixed-length collection to frame-time Exprs.
    ///
    /// Admits array literals, immutable compile-time constant arrays, and
    /// compile-time-bounded `.map()` whose callback may capture `T` / other
    /// legal frame values. The expanded length is frozen; no JS array remains.
    pub(super) fn collect_array_elements(
        &mut self,
        expression: &Expression<'_>,
        role: &str,
    ) -> Option<Vec<ExprId>> {
        if self.array_shape_illegal(expression, role) {
            return None;
        }
        if let Some(call) = as_map_call(expression) {
            return self.unroll_array_map(call, role);
        }
        if let Some(items) = array_items(expression) {
            return items
                .into_iter()
                .map(|item| self.lower_expr(item))
                .collect();
        }
        if let Expression::Identifier(identifier) = peel_expr(expression)
            && let Some(init) = self
                .module_const_inits
                .get(identifier.name.as_str())
                .copied()
        {
            return self.collect_array_elements(init, role);
        }
        if self.collection_depends_on_runtime(expression) {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!(
                    "{role} length cannot depend on ctx, props, data, cues, or other frame values"
                ),
            );
            return None;
        }
        match self.eval_static(expression) {
            Some(serde_json::Value::Array(items)) => {
                if items.len() > MAX_EXPR_MAP_UNROLL {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        format!(
                            "{role} expands {} items; the compile-time collection budget is {MAX_EXPR_MAP_UNROLL}",
                            items.len()
                        ),
                    );
                    return None;
                }
                items
                    .into_iter()
                    .map(|item| {
                        let Some(value) = motion_value_from_json(&item) else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                expression.span(),
                                format!(
                                    "{role} element is not a compile-time scalar or typed value"
                                ),
                            );
                            return None;
                        };
                        Some(self.push(Expr::Const { value }, expression.span()))
                    })
                    .collect()
            }
            Some(_) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("{role} must be a fixed-length array"),
                );
                None
            }
            None => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!(
                        "{role} must be a fixed-length array literal, an immutable compile-time constant array, or a compile-time-bounded .map()"
                    ),
                );
                None
            }
        }
    }

    pub(super) fn unroll_array_map(
        &mut self,
        call: &oxc::ast::ast::CallExpression<'_>,
        role: &str,
    ) -> Option<Vec<ExprId>> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        if self.array_shape_illegal(&member.object, role) {
            return None;
        }
        if self.collection_depends_on_runtime(&member.object) {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "{role} .map() length cannot depend on ctx, props, data, cues, or other frame values; the expanded topology is fixed at compile time"
                ),
            );
            return None;
        }
        let Some(serde_json::Value::Array(items)) = self.eval_static(&member.object) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "{role} .map() input must be a compile-time-bounded immutable array; mutable bindings, spreads, and helpers without a static length are illegal"
                ),
            );
            return None;
        };
        if items.len() > MAX_EXPR_MAP_UNROLL
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "{role} .map() expands {} items; the compile-time unroll budget is {MAX_EXPR_MAP_UNROLL}",
                    items.len()
                ),
            );
            return None;
        }
        self.expanded_list_items += items.len();
        if call.arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "array .map() requires one arrow callback",
            );
            return None;
        }
        let Some(callback) = call.arguments[0].as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.arguments[0].span(),
                "array .map() callback cannot spread",
            );
            return None;
        };
        let Expression::ArrowFunctionExpression(arrow) = peel_expr(callback) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                callback.span(),
                "array .map() callback must be an arrow function",
            );
            return None;
        };
        if arrow.r#async || arrow.params.rest.is_some() || arrow.params.items.len() > 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.span(),
                "array .map() callback must be synchronous `(item, index) => value`",
            );
            return None;
        }
        let saved_frame = self.bindings.snapshot_frame();
        let mut points = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            self.bindings.restore_frame(saved_frame.clone());
            if let Some(parameter) = arrow.params.items.first() {
                self.bind_static_pattern(&parameter.pattern, item);
            }
            if let Some(parameter) = arrow.params.items.get(1) {
                self.bind_static_pattern(&parameter.pattern, serde_json::json!(index));
            }
            let Some(expr) = self.lower_map_callback_expr(arrow) else {
                self.bindings.restore_frame(saved_frame);
                return None;
            };
            points.push(expr);
        }
        self.bindings.restore_frame(saved_frame);
        Some(points)
    }

    pub(super) fn lower_map_callback_expr(
        &mut self,
        arrow: &oxc::ast::ast::ArrowFunctionExpression<'_>,
    ) -> Option<ExprId> {
        if let Some(expression) = arrow.get_expression() {
            return self.lower_expr(expression);
        }
        let Some(body) = arrow.body.as_function_body() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.body.span(),
                "array .map() callback needs an expression or block body",
            );
            return None;
        };
        let mut result = None;
        for statement in &body.statements {
            match statement {
                Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                    if result.is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            statement.span(),
                            "array .map() callback cannot execute declarations after return",
                        );
                        continue;
                    }
                    for declarator in &declaration.declarations {
                        let Some(initializer) = &declarator.init else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                declarator.span(),
                                "map callback const needs an initializer",
                            );
                            continue;
                        };
                        let Some(name) = declarator.id.get_identifier_name() else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                declarator.span(),
                                "array .map() callback const currently requires a plain identifier",
                            );
                            continue;
                        };
                        if let Some(value) = self.eval_static(initializer) {
                            self.bind_static(name.to_string(), value);
                        } else if let Some(expr) = self.lower_expr(initializer) {
                            self.bind_dynamic(name.to_string(), expr);
                        }
                    }
                }
                Statement::ReturnStatement(statement) if result.is_none() => {
                    result = statement
                        .argument
                        .as_ref()
                        .and_then(|expression| self.lower_expr(expression));
                }
                other => self.illegal(
                    DiagCode::GrammarForbidden,
                    other.span(),
                    "array .map() callback only allows prepare-time const bindings and one returned expression",
                ),
            }
        }
        if result.is_none() && self.diagnostics.is_empty() {
            self.illegal(
                DiagCode::GrammarForbidden,
                body.span(),
                "array .map() callback must return an expression",
            );
        }
        result
    }

    pub(super) fn lower_interpolate(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if !(3..=4).contains(&arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "interpolate(input, inputRange, outputRange, options?) requires three or four arguments",
            );
            return None;
        }
        let input = self.lower_expr(arguments[0].as_expression()?)?;
        let inputs = self.collect_interpolate_inputs(arguments[1].as_expression()?)?;
        let outputs = self.collect_interpolate_outputs(arguments[2].as_expression()?)?;
        if inputs.len() < 2 || inputs.len() != outputs.len() {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "interpolate ranges must have equal length >= 2",
            );
            return None;
        }
        let mut stops = Vec::new();
        for (input, output) in inputs.into_iter().zip(outputs) {
            stops.push(InterpolateStop { input, output });
        }
        let easing_count = stops.len() - 1;
        let easings = match arguments.get(3) {
            None => vec![MotionEasing::Linear; easing_count],
            Some(options) => self.lower_interpolate_options(options, easing_count)?,
        };
        Some(self.push(
            Expr::Interpolate {
                input,
                stops,
                easings,
                extrapolate_left: Extrapolation::Clamp,
                extrapolate_right: Extrapolation::Clamp,
            },
            span,
        ))
    }

    /// Lower a closed-form spring with unclamped time and mutually exclusive presets and physical
    /// parameters. Require `fps: ctx.fps` without lowering it so elapsed frames use the render
    /// clock and frame rate only changes sampling density.
    pub(super) fn lower_spring(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [argument] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "spring takes exactly one options object",
            );
            return None;
        };
        let Some(Expression::ObjectExpression(object)) = argument.as_expression().map(strip_parens)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                argument.span(),
                "spring takes an options object: spring({ elapsedFrames, fps, preset })",
            );
            return None;
        };

        let mut elapsed = None;
        let mut saw_fps = false;
        let mut preset: Option<(String, Span)> = None;
        let mut bare: Vec<(&'static str, f64, Span)> = Vec::new();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "spring options cannot spread",
                );
                return None;
            };
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "spring option names must be static identifiers",
                );
                return None;
            };
            match name.as_str() {
                "elapsedFrames" => elapsed = self.lower_expr(&property.value),
                "fps" => {
                    // Only accept ctx.fps.
                    let source = self.src(&property.value).trim().to_string();
                    if source != "ctx.fps" {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "spring's `fps` must be exactly `ctx.fps`: the spring is sampled at \
                             the frame rate this render actually uses, so that changing the frame \
                             rate changes only the sampling density, never the physical feel",
                        );
                        return None;
                    }
                    saw_fps = true;
                }
                "preset" => {
                    let Some(MotionValue::Str(value)) = self.fold_to_value(&property.value) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            format!(
                                "spring preset must be one of: {}",
                                valle_motion::spring::PRESETS.join(", ")
                            ),
                        );
                        return None;
                    };
                    preset = Some((value, property.value.span()));
                }
                name @ ("mass" | "stiffness" | "damping") => {
                    let Some(value) = self.fold_to_number(&property.value) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            format!(
                                "spring `{name}` must be a number known at compile time — the \
                                 physical parameters of a spring do not vary per frame"
                            ),
                        );
                        return None;
                    };
                    let name = match name {
                        "mass" => "mass",
                        "stiffness" => "stiffness",
                        _ => "damping",
                    };
                    bare.push((name, value, property.value.span()));
                }
                other => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        format!(
                            "`{other}` is not a spring option; use elapsedFrames, fps, preset, \
                             or mass/stiffness/damping"
                        ),
                    );
                    return None;
                }
            }
        }

        let Some(elapsed_frames) = elapsed else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "spring needs `elapsedFrames` — pass `ctx.<phase>.elapsedFrames`, which is not \
                 clamped at the window edges so the spring keeps converging after the phase ends",
            );
            return None;
        };
        if !saw_fps {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "spring needs `fps: ctx.fps` so the source says in what unit `elapsedFrames` is \
                 counted",
            );
            return None;
        }

        // Reject a preset combined with explicit physical parameters.
        if let Some((_, preset_span)) = &preset
            && !bare.is_empty()
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                *preset_span,
                "spring `preset` and the bare `mass`/`stiffness`/`damping` parameters are \
                 mutually exclusive; keeping both would silently ignore one of them",
            );
            return None;
        }

        let params = if let Some((name, preset_span)) = preset {
            let Some(params) = valle_motion::spring::preset(&name) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    preset_span,
                    format!(
                        "unknown spring preset `{name}`; use one of: {}",
                        valle_motion::spring::PRESETS.join(", ")
                    ),
                );
                return None;
            };
            params
        } else {
            // Default physical parameters: mass 1, stiffness 100, damping 10.
            let mut params = valle_motion::SpringParams {
                mass: 1.0,
                stiffness: 100.0,
                damping: 10.0,
            };
            for (name, value, value_span) in bare {
                let slot = match name {
                    "mass" => &mut params.mass,
                    "stiffness" => &mut params.stiffness,
                    _ => &mut params.damping,
                };
                if !value.is_finite() || (name != "damping" && value <= 0.0) || value < 0.0 {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        value_span,
                        format!("spring `{name}` must be finite and positive"),
                    );
                    return None;
                }
                *slot = value;
            }
            params
        };

        Some(self.push(
            Expr::Spring {
                elapsed_frames,
                mass: params.mass,
                stiffness: params.stiffness,
                damping: params.damping,
            },
            span,
        ))
    }

    pub(super) fn lower_interpolate_options(
        &mut self,
        options: &Argument<'_>,
        easing_count: usize,
    ) -> Option<Vec<MotionEasing>> {
        let expression = options.as_expression()?;
        let Expression::ObjectExpression(object) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "interpolate options must be an object literal",
            );
            return None;
        };
        let mut easings = None;
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "interpolate options spread is illegal; list every option explicitly",
                );
                continue;
            };
            let name = match &property.key {
                PropertyKey::StaticIdentifier(name) => name.name.to_string(),
                PropertyKey::StringLiteral(name) => name.value.to_string(),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.key.span(),
                        "computed interpolate option names are illegal",
                    );
                    continue;
                }
            };
            if name != "easing" {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("unknown interpolate option `{name}`; only `easing` is admitted"),
                );
                continue;
            }
            easings = self.lower_easing(&property.value, easing_count);
        }
        easings.or_else(|| Some(vec![MotionEasing::Linear; easing_count]))
    }

    /// Accept one easing for all intervals or one per interval, with exactly one fewer entries than
    /// stops.
    pub(super) fn lower_easing(
        &mut self,
        value: &Expression<'_>,
        easing_count: usize,
    ) -> Option<Vec<MotionEasing>> {
        if self.array_shape_illegal(value, "interpolate easing array") {
            return None;
        }
        if let Some(items) = array_items(value) {
            if items.len() != easing_count {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    value.span(),
                    format!(
                        "interpolate easing array must have exactly {easing_count} entries (one per segment)"
                    ),
                );
                return None;
            }
            return items
                .into_iter()
                .map(|item| self.easing_name(item))
                .collect();
        }
        if let Expression::Identifier(identifier) = peel_expr(value)
            && let Some(init) = self
                .module_const_inits
                .get(identifier.name.as_str())
                .copied()
        {
            return self.lower_easing(init, easing_count);
        }
        if let Some(serde_json::Value::Array(items)) = self.eval_static(value) {
            if items.len() != easing_count {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    value.span(),
                    format!(
                        "interpolate easing array must have exactly {easing_count} entries (one per segment)"
                    ),
                );
                return None;
            }
            return items
                .iter()
                .map(|item| {
                    let Some(name) = item.as_str() else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            value.span(),
                            "interpolate easing must be a compile-time easing name",
                        );
                        return None;
                    };
                    self.parse_easing_or_diag(name, value.span())
                })
                .collect();
        }
        Some(vec![self.easing_name(value)?; easing_count])
    }

    pub(super) fn easing_name(&mut self, value: &Expression<'_>) -> Option<MotionEasing> {
        if let Expression::StringLiteral(name) = peel_expr(value) {
            return self.parse_easing_or_diag(name.value.as_str(), value.span());
        }
        if let Some(serde_json::Value::String(name)) = self.eval_static(value) {
            return self.parse_easing_or_diag(&name, value.span());
        }
        self.illegal(
            DiagCode::GrammarForbidden,
            value.span(),
            "interpolate easing must be a compile-time easing name (string literal or immutable constant)",
        );
        None
    }

    pub(super) fn parse_easing_or_diag(&mut self, name: &str, span: Span) -> Option<MotionEasing> {
        parse_easing(name).or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "unknown easing `{name}`; use linear/ease/easeIn/easeOut/easeInOut/exp or cubic-bezier(x1,y1,x2,y2)"
                ),
            );
            None
        })
    }
}
