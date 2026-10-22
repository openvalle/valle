//! Fixed collections, compile-time map expansion, interpolate, spring, and easing.

use super::*;
use valle_motion::expr::ExprType;

impl<'s> Compiler<'s> {
    /// A shared dynamic shift of every input stop is exactly equivalent to shifting the input.
    /// This admits per-unit staggered ranges without making InterpolateStop frame-dependent.
    fn shifted_interpolate_inputs<'a>(
        &mut self,
        expression: &'a Expression<'a>,
    ) -> Option<(Vec<f64>, &'a Expression<'a>)> {
        let items = array_items(expression)?;
        let mut inputs = Vec::with_capacity(items.len());
        let mut shift = None;
        let mut shift_source = None::<String>;
        for item in items {
            if self.fold_to_number(item).is_some() {
                return None;
            }
            let (base, candidate) = match peel_expr(item) {
                Expression::BinaryExpression(binary)
                    if binary.operator == BinaryOperator::Addition =>
                {
                    if let Some(base) = self.fold_to_number(&binary.left) {
                        (base, &binary.right)
                    } else if let Some(base) = self.fold_to_number(&binary.right) {
                        (base, &binary.left)
                    } else {
                        (0.0, item)
                    }
                }
                _ => (0.0, item),
            };
            let candidate = peel_expr(candidate);
            let spelling = self.src(candidate).trim();
            if !base.is_finite() || spelling.is_empty() {
                return None;
            }
            if let Some(expected) = &shift_source {
                if spelling != expected {
                    return None;
                }
            } else {
                shift_source = Some(spelling.to_owned());
                shift = Some(candidate);
            }
            inputs.push(base);
        }
        Some((inputs, shift?))
    }

    pub(super) fn collect_interpolate_inputs(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<Vec<f64>> {
        if self.array_shape_illegal(expression, "interpolate inputRange") {
            return None;
        }
        if let Expression::ArrayExpression(_) = peel_expr(expression)
            && !spreads_array(expression)
        {
            return array_items(expression)?
                .into_iter()
                .map(|item| {
                    static_number(item)
                        .or_else(|| self.fold_to_number(item))
                        .or_else(|| {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                item.span(),
                                "interpolate input stops must be finite numbers known at compile time; \
                                 for per-unit stagger use a shared shift on every stop, such as \
                                 [unit.index * 0.1, 1 + unit.index * 0.1]",
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
        if let Expression::ArrayExpression(_) = peel_expr(expression)
            && !spreads_array(expression)
        {
            return array_items(expression)?
                .into_iter()
                .map(|item| {
                    static_motion_value(item)
                        // A string computed at compile time, such as a template literal of
                        // constants, reads as the string literal it equals.
                        .or_else(|| match self.fold_to_value(item)? {
                            MotionValue::Str(text) => Some(motion_value_from_string(&text)),
                            value => Some(value),
                        })
                        .or_else(|| {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                item.span(),
                                "interpolate outputs must be scalars known at compile time (literals, or \
                                 arithmetic over module constants — anything depending on ctx/props \
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
                        // A fixed tuple or a prepare-time array is written out in place.
                        ArrayExpressionElement::SpreadElement(spread)
                            if self.dynamic_tuple_value(&spread.argument).is_some()
                                || matches!(
                                    self.eval_static(&spread.argument),
                                    Some(serde_json::Value::Array(_))
                                ) => {}
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                spread.span(),
                                format!(
                                    "{role} can spread a fixed tuple or a prepare-time array; the expanded length must be a compile-time-fixed list"
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
                .any(|name| matches!(name.as_str(), "useCurrentTime" | "useComposition" | "props"))
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
        if spreads_array(expression) {
            return self.lower_dynamic_tuple_expression(expression, role);
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
                    "{role} length cannot depend on frame values: the clock, `fps`, \
                     `clipDuration`, a template's `width` and `height`, or `props`; bind the \
                     duration, or a scene's size, by name first: `const {{ duration }} = \
                     useComposition()`"
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
                    "{role} .map() length cannot depend on frame values: the clock, `fps`, \
                     `clipDuration`, a template's `width` and `height`, or `props`, because the \
                     expanded topology is fixed at compile time; bind the duration, or a \
                     scene's size, by name first: `const {{ duration }} = useComposition()`"
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
        let time = self.lower_expr(arguments[0].as_expression()?)?;
        let range = arguments[1].as_expression()?;
        let (input, inputs) = if let Some((inputs, shift)) = self.shifted_interpolate_inputs(range)
        {
            let offset = self.lower_expr(shift)?;
            if self.expr_arena.types.get(offset.0 as usize) != Some(&Some(ExprType::Number)) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    shift.span(),
                    "interpolate shared input-stop shift must be numeric",
                );
                return None;
            }
            self.check_time_pair(time, offset, span);
            (
                self.push(
                    Expr::Sub {
                        lhs: time,
                        rhs: offset,
                    },
                    span,
                ),
                inputs,
            )
        } else {
            (time, self.collect_interpolate_inputs(range)?)
        };
        self.check_interpolate_times(time, input, &inputs, span);
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
        let (easings, color_space, extrapolate_left, extrapolate_right) = match arguments.get(3) {
            None => (
                vec![Easing::Linear; easing_count],
                valle_draw::program::GradientInterpolation::Srgb,
                Extrapolation::Clamp,
                Extrapolation::Clamp,
            ),
            Some(options) => self.lower_interpolate_options(options, easing_count)?,
        };
        if color_space != valle_draw::program::GradientInterpolation::Srgb
            && !matches!(stops[0].output, MotionValue::Color(_))
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "interpolate colorSpace requires color outputs",
            );
            return None;
        }
        let clock = self.reads_clock(input);
        Some(self.push(
            Expr::Interpolate {
                input,
                stops,
                easings,
                color_space,
                extrapolate_left,
                extrapolate_right,
                clock,
            },
            span,
        ))
    }

    /// Lower a closed-form spring of unclamped time in seconds, `spring(t, { preset })`. The
    /// spring is described by a preset, by `duration` and `bounce`, or by its physical
    /// parameters.
    pub(super) fn lower_spring(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        builtin: &str,
    ) -> Option<ExprId> {
        let new_form = format!(
            "{builtin} takes the time in seconds and its options: \
             `{builtin}(seconds - 0.4, {{ preset: \"gentle\" }})`"
        );
        let (time, options) = match arguments {
            [time] => (time, None),
            [time, options] => (time, Some(options)),
            _ => {
                self.illegal(DiagCode::GrammarForbidden, span, new_form);
                return None;
            }
        };
        let Some(time) = time.as_expression() else {
            self.illegal(DiagCode::GrammarForbidden, time.span(), new_form);
            return None;
        };
        if matches!(strip_parens(time), Expression::ObjectExpression(_)) {
            self.illegal(
                DiagCode::GrammarForbidden,
                time.span(),
                format!("{new_form}; `elapsedFrames` and `fps` are gone, the time is in seconds"),
            );
            return None;
        }
        let object = match options.map(|options| options.as_expression().map(strip_parens)) {
            None => None,
            Some(Some(Expression::ObjectExpression(object))) => Some(object),
            Some(_) => {
                self.illegal(DiagCode::GrammarForbidden, span, new_form);
                return None;
            }
        };
        let time = self.lower_expr(time)?;

        let mut preset: Option<(String, Span)> = None;
        let mut bare: Vec<(&'static str, f64, Span)> = Vec::new();
        let mut duration: Option<(f64, Span)> = None;
        let mut bounce: Option<(f64, Span)> = None;
        let mut initial_velocity = 0.0;
        let mut seen = BTreeSet::new();
        for property in object.iter().flat_map(|object| &object.properties) {
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
            if !seen.insert(name.clone()) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!("{builtin} option `{name}` is duplicated"),
                );
                return None;
            }
            match name.as_str() {
                "elapsedFrames" | "fps" => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        format!(
                            "{new_form}; `elapsedFrames` and `fps` are gone, the time is in \
                             seconds"
                        ),
                    );
                    return None;
                }
                "initialVelocity" => {
                    let Some(value) = self
                        .fold_to_number(&property.value)
                        .filter(|v| v.is_finite())
                    else {
                        self.illegal(DiagCode::GrammarForbidden, property.value.span(),
                            "spring initialVelocity must be a finite number known at compile time, in normalized distance per second");
                        return None;
                    };
                    initial_velocity = value;
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
                "duration" => {
                    let Some(value) = self
                        .fold_to_number(&property.value)
                        .filter(|value| value.is_finite() && *value > 0.0)
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "spring `duration` must be a positive number of seconds known at \
                             compile time",
                        );
                        return None;
                    };
                    duration = Some((value, property.value.span()));
                }
                "bounce" => {
                    let Some(value) = self
                        .fold_to_number(&property.value)
                        .filter(|value| *value > -1.0 && *value < 1.0)
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "spring `bounce` must be a number above -1 and below 1 known at \
                             compile time: 0 settles without overshooting, higher values bounce \
                             more and lower ones settle more slowly",
                        );
                        return None;
                    };
                    bounce = Some((value, property.value.span()));
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
                            "`{other}` is not a spring option; use initialVelocity, and \
                             preset, duration/bounce or mass/stiffness/damping"
                        ),
                    );
                    return None;
                }
            }
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
        let swing = duration.or(bounce).map(|(_, span)| span);
        if let Some(swing_span) = swing
            && (preset.is_some() || !bare.is_empty())
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                swing_span,
                "spring `duration`/`bounce`, `preset` and `mass`/`stiffness`/`damping` are three \
                 ways to describe one spring; use one of them",
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
        } else if swing.is_some() {
            // SwiftUI's spring(duration:bounce:): the undamped spring swings once per `duration`,
            // and `bounce` sets the damping ratio, 1 - bounce, or 1 / (1 + bounce) below 0.
            let duration = duration.map_or(0.5, |(value, _)| value);
            let bounce = bounce.map_or(0.0, |(value, _)| value);
            let ratio = if bounce < 0.0 {
                1.0 / (1.0 + bounce)
            } else {
                1.0 - bounce
            };
            let frequency = std::f64::consts::TAU / duration;
            let stiffness = frequency * frequency;
            valle_motion::SpringParams {
                mass: 1.0,
                stiffness,
                damping: 2.0 * ratio * valle_draw::math::sqrt(stiffness),
                initial_velocity: 0.0,
            }
        } else {
            // Default physical parameters: mass 1, stiffness 100, damping 10.
            let mut params = valle_motion::SpringParams {
                mass: 1.0,
                stiffness: 100.0,
                damping: 10.0,
                initial_velocity: 0.0,
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
                time,
                mass: params.mass,
                stiffness: params.stiffness,
                damping: params.damping,
                initial_velocity,
                output: if builtin == "springVelocity" {
                    valle_motion::spring::SpringOutput::Velocity
                } else {
                    valle_motion::spring::SpringOutput::Position
                },
            },
            span,
        ))
    }

    pub(super) fn lower_interpolate_options(
        &mut self,
        options: &Argument<'_>,
        easing_count: usize,
    ) -> Option<(
        Vec<Easing>,
        valle_draw::program::GradientInterpolation,
        Extrapolation,
        Extrapolation,
    )> {
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
        let mut color_space = None;
        let mut hue = None;
        let mut extrapolate_left = Extrapolation::Clamp;
        let mut extrapolate_right = Extrapolation::Clamp;
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
            match name.as_str() {
                "easing" => easings = self.lower_easing(&property.value, easing_count),
                "extrapolateLeft" => {
                    extrapolate_left = self.lower_extrapolation(&property.value)?
                }
                "extrapolateRight" => {
                    extrapolate_right = self.lower_extrapolation(&property.value)?;
                }
                "colorSpace" => {
                    let Expression::StringLiteral(value) = strip_parens(&property.value) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "interpolate colorSpace must be a literal string",
                        );
                        return None;
                    };
                    color_space = Some(match value.value.as_str() {
                        "srgb" => valle_draw::program::GradientInterpolation::Srgb,
                        "linear" => valle_draw::program::GradientInterpolation::LinearSrgb,
                        "oklab" => valle_draw::program::GradientInterpolation::Oklab,
                        "oklch" => valle_draw::program::GradientInterpolation::Oklch {
                            hue: valle_draw::program::HueDirection::Shorter,
                        },
                        _ => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                property.value.span(),
                                "interpolate colorSpace must be srgb, linear, oklab, or oklch",
                            );
                            return None;
                        }
                    });
                }
                "hue" => {
                    let Expression::StringLiteral(value) = strip_parens(&property.value) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "interpolate hue must be a literal string",
                        );
                        return None;
                    };
                    hue = Some(match value.value.as_str() {
                        "shorter" => valle_draw::program::HueDirection::Shorter,
                        "longer" => valle_draw::program::HueDirection::Longer,
                        "increasing" => valle_draw::program::HueDirection::Increasing,
                        "decreasing" => valle_draw::program::HueDirection::Decreasing,
                        _ => {
                            self.illegal(DiagCode::GrammarForbidden, property.value.span(),
                                "interpolate hue must be shorter, longer, increasing, or decreasing");
                            return None;
                        }
                    });
                }
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.key.span(),
                        format!("unknown interpolate option `{name}`"),
                    );
                    return None;
                }
            }
        }
        let color_space = match (color_space, hue) {
            (Some(valle_draw::program::GradientInterpolation::Oklch { .. }), hue) => {
                valle_draw::program::GradientInterpolation::Oklch {
                    hue: hue.unwrap_or(valle_draw::program::HueDirection::Shorter),
                }
            }
            (_, Some(_)) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "interpolate hue requires colorSpace: \"oklch\"",
                );
                return None;
            }
            (space, None) => space.unwrap_or(valle_draw::program::GradientInterpolation::Srgb),
        };
        Some((
            easings.unwrap_or_else(|| vec![Easing::Linear; easing_count]),
            color_space,
            extrapolate_left,
            extrapolate_right,
        ))
    }

    /// `"clamp"` holds the first and last outputs outside the input range; `"extend"` continues
    /// the first and last segments as straight lines, without their easing.
    fn lower_extrapolation(&mut self, value: &Expression<'_>) -> Option<Extrapolation> {
        let Expression::StringLiteral(text) = strip_parens(value) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                value.span(),
                "interpolate extrapolateLeft and extrapolateRight must be a literal string",
            );
            return None;
        };
        match text.value.as_str() {
            "clamp" => Some(Extrapolation::Clamp),
            "extend" => Some(Extrapolation::Extend),
            other => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    value.span(),
                    format!(
                        "interpolate extrapolation `{other}` is not available; use \"clamp\", \
                         the default, or \"extend\""
                    ),
                );
                None
            }
        }
    }

    /// Accept one easing for all intervals or one per interval, with exactly one fewer entries than
    /// stops.
    pub(super) fn lower_easing(
        &mut self,
        value: &Expression<'_>,
        easing_count: usize,
    ) -> Option<Vec<Easing>> {
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

    pub(super) fn easing_name(&mut self, value: &Expression<'_>) -> Option<Easing> {
        if let Expression::StringLiteral(name) = peel_expr(value) {
            return self.parse_easing_or_diag(name.value.as_str(), value.span());
        }
        if let Some(serde_json::Value::String(name)) = self.eval_static(value) {
            return self.parse_easing_or_diag(&name, value.span());
        }
        let message = if let Some(easing) = self.remotion_easing(value) {
            format!("Motion has no `Easing` object; write this curve as \"{easing}\"")
        } else if names_easing_object(value) {
            format!(
                "Motion has no `Easing` object; write the easing as a string: {}",
                valle_timeline::easing::EASING_SPELLINGS
            )
        } else {
            "interpolate easing must be a compile-time easing name (string literal or immutable \
             constant)"
                .to_owned()
        };
        self.illegal(DiagCode::GrammarForbidden, value.span(), message);
        None
    }

    /// The curve a Remotion `Easing` expression draws, where one of the spellings draws exactly
    /// it. Remotion's `ease` is CSS's ease-in, and its `bounce` bounces at the end.
    fn remotion_easing(&mut self, expression: &Expression<'_>) -> Option<Easing> {
        match peel_expr(expression) {
            Expression::StaticMemberExpression(member) if is_easing_object(&member.object) => {
                Some(match member.property.name.as_str() {
                    "linear" => Easing::Linear,
                    "ease" => Easing::EaseIn,
                    "quad" => Easing::EaseInQuad,
                    "cubic" => Easing::EaseInCubic,
                    "sin" => Easing::EaseInSine,
                    "circle" => Easing::EaseInCirc,
                    "exp" => Easing::EaseInExpo,
                    "bounce" => Easing::EaseOutBounce,
                    _ => return None,
                })
            }
            Expression::CallExpression(call) => {
                let Expression::StaticMemberExpression(member) = peel_expr(&call.callee) else {
                    return None;
                };
                if !is_easing_object(&member.object) {
                    return None;
                }
                let arguments = call
                    .arguments
                    .iter()
                    .map(Argument::as_expression)
                    .collect::<Option<Vec<_>>>()?;
                match (member.property.name.as_str(), arguments.as_slice()) {
                    ("in", [curve]) => self.remotion_easing(curve),
                    // Time reversed: easeOut(t) = 1 - easeIn(1 - t).
                    ("out", [curve]) => Some(match self.remotion_easing(curve)? {
                        Easing::Linear => Easing::Linear,
                        Easing::EaseIn => Easing::EaseOut,
                        Easing::EaseInQuad => Easing::EaseOutQuad,
                        Easing::EaseInCubic => Easing::EaseOutCubic,
                        Easing::EaseInQuart => Easing::EaseOutQuart,
                        Easing::EaseInQuint => Easing::EaseOutQuint,
                        Easing::EaseInSine => Easing::EaseOutSine,
                        Easing::EaseInCirc => Easing::EaseOutCirc,
                        Easing::EaseInExpo => Easing::EaseOutExpo,
                        Easing::EaseInBack => Easing::EaseOutBack,
                        Easing::EaseOutBounce => Easing::EaseInBounce,
                        Easing::CubicBezier {
                            p: [x1, y1, x2, y2],
                        } => Easing::CubicBezier {
                            p: [1.0 - x2, 1.0 - y2, 1.0 - x1, 1.0 - y1],
                        },
                        _ => return None,
                    }),
                    // Each half is the curve scaled into it, which is easings.net's in-out form
                    // for these families.
                    ("inOut", [curve]) => Some(match self.remotion_easing(curve)? {
                        Easing::Linear => Easing::Linear,
                        Easing::EaseInQuad => Easing::EaseInOutQuad,
                        Easing::EaseInCubic => Easing::EaseInOutCubic,
                        Easing::EaseInQuart => Easing::EaseInOutQuart,
                        Easing::EaseInQuint => Easing::EaseInOutQuint,
                        Easing::EaseInSine => Easing::EaseInOutSine,
                        Easing::EaseInCirc => Easing::EaseInOutCirc,
                        Easing::EaseInExpo => Easing::EaseInOutExpo,
                        _ => return None,
                    }),
                    ("poly", [exponent]) => {
                        let exponent = self.fold_to_number(exponent)?;
                        [
                            (2.0, Easing::EaseInQuad),
                            (3.0, Easing::EaseInCubic),
                            (4.0, Easing::EaseInQuart),
                            (5.0, Easing::EaseInQuint),
                        ]
                        .into_iter()
                        .find(|(power, _)| *power == exponent)
                        .map(|(_, easing)| easing)
                    }
                    ("back", []) => Some(Easing::EaseInBack),
                    ("back", [overshoot]) => {
                        (self.fold_to_number(overshoot)? == 1.70158).then_some(Easing::EaseInBack)
                    }
                    ("bezier", [x1, y1, x2, y2]) => {
                        let easing = Easing::CubicBezier {
                            p: [
                                self.fold_to_number(x1)?,
                                self.fold_to_number(y1)?,
                                self.fold_to_number(x2)?,
                                self.fold_to_number(y2)?,
                            ],
                        };
                        easing.is_valid().then_some(easing)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn parse_easing_or_diag(&mut self, name: &str, span: Span) -> Option<Easing> {
        Easing::parse(name).or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "invalid easing `{name}`; use {}",
                    valle_timeline::easing::EASING_SPELLINGS
                ),
            );
            None
        })
    }
}

fn is_easing_object(expression: &Expression<'_>) -> bool {
    matches!(peel_expr(expression), Expression::Identifier(identifier) if identifier.name == "Easing")
}

/// `Easing.name` or `Easing.name(...)`, Remotion's easing object.
fn names_easing_object(expression: &Expression<'_>) -> bool {
    match peel_expr(expression) {
        Expression::StaticMemberExpression(member) => is_easing_object(&member.object),
        Expression::CallExpression(call) => matches!(
            peel_expr(&call.callee),
            Expression::StaticMemberExpression(member) if is_easing_object(&member.object)
        ),
        _ => false,
    }
}
