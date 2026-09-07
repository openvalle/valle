//! Lexical bindings, component/helper expansion, and compile-time list scopes.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn static_bindings(&self) -> Vec<(String, serde_json::Value)> {
        let mut bindings = self
            .bindings
            .statics
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>();
        if let Some(theme) = &self.current_theme {
            bindings.push((THEME_SCOPE_BINDING.to_owned(), theme.clone()));
        }
        bindings
    }

    /// Bind a prepare-time constant and invalidate any dynamic binding with the same name.
    pub(super) fn bind_static(&mut self, name: String, value: serde_json::Value) {
        self.bindings.scalars.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.statics.insert(name, value);
    }

    /// Bind a runtime expression and invalidate static or tuple bindings with the same name.
    /// Otherwise the sandbox could fold a shadowed outer value into a constant.
    pub(super) fn bind_dynamic(&mut self, name: String, expr: ExprId) {
        self.bindings.statics.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.scalars.insert(name, expr);
    }

    /// Bind a fixed-length collection of frame-time expressions. The collection
    /// only exists while lowering author code; every indexed use resolves back to
    /// one scalar ExprId before the SceneArtifact is emitted.
    pub(super) fn bind_dynamic_tuple(&mut self, name: String, values: Vec<ExprId>) {
        self.bindings.scalars.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.tuples.insert(name, values);
    }

    pub(super) fn dynamic_tuple_value(&self, expression: &Expression<'_>) -> Option<Vec<ExprId>> {
        match peel_expr(expression) {
            Expression::Identifier(identifier) => {
                self.bindings.tuples.get(identifier.name.as_str()).cloned()
            }
            Expression::StaticMemberExpression(member) if matches!(&member.object, Expression::Identifier(id) if id.name == "props") => {
                self.bindings
                    .component_props
                    .as_ref()
                    .and_then(|props| props.get(member.property.name.as_str()))
                    .and_then(|value| match value {
                        AuthorValue::DynamicTuple(values) => Some(values.clone()),
                        _ => None,
                    })
            }
            _ => None,
        }
    }

    pub(super) fn is_dynamic_tuple_expression(&self, expression: &Expression<'_>) -> bool {
        matches!(peel_expr(expression), Expression::ArrayExpression(_))
            || matches!(
                peel_expr(expression),
                Expression::CallExpression(call)
                    if matches!(
                        &call.callee,
                        Expression::StaticMemberExpression(member)
                            if member.property.name == "map"
                    )
            )
            || self.dynamic_tuple_value(expression).is_some()
    }

    pub(super) fn lower_dynamic_tuple_expression(
        &mut self,
        expression: &Expression<'_>,
        role: &str,
    ) -> Option<Vec<ExprId>> {
        if let Some(values) = self.dynamic_tuple_value(expression) {
            return Some(values);
        }
        if let Expression::CallExpression(call) = peel_expr(expression)
            && matches!(
                &call.callee,
                Expression::StaticMemberExpression(member) if member.property.name == "map"
            )
        {
            return self.lower_dynamic_tuple_map(call, role);
        }
        let Expression::ArrayExpression(array) = peel_expr(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!("{role} must be a fixed-length array literal or dynamic tuple binding"),
            );
            return None;
        };
        if array.elements.len() > MAX_EXPR_MAP_UNROLL {
            self.illegal(
                DiagCode::GrammarForbidden,
                array.span,
                format!(
                    "{role} expands {} elements; the compile-time tuple budget is {MAX_EXPR_MAP_UNROLL}",
                    array.elements.len()
                ),
            );
            return None;
        }
        array
            .elements
            .iter()
            .map(|element| match element {
                ArrayExpressionElement::SpreadElement(spread) => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        spread.span(),
                        format!("{role} cannot use spread; tuple length must stay explicit"),
                    );
                    None
                }
                ArrayExpressionElement::Elision(elision) => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        elision.span(),
                        format!("{role} cannot contain holes"),
                    );
                    None
                }
                value => value
                    .as_expression()
                    .and_then(|value| self.lower_expr(value)),
            })
            .collect()
    }

    /// Compile-time-unroll a static collection whose callback returns one
    /// frame-time scalar per item. The tuple remains an authoring-only value:
    /// every use is resolved to an existing ExprId before artifact emission.
    pub(super) fn lower_dynamic_tuple_map(
        &mut self,
        call: &oxc::ast::ast::CallExpression<'_>,
        role: &str,
    ) -> Option<Vec<ExprId>> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let Some(serde_json::Value::Array(items)) = self.eval_static(&member.object) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!("{role} map input must be prepare-time static"),
            );
            return None;
        };
        if items.len() > MAX_EXPR_MAP_UNROLL {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "{role} map expands {} elements; the compile-time tuple budget is {MAX_EXPR_MAP_UNROLL}",
                    items.len()
                ),
            );
            return None;
        }
        if call.arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!("{role} map requires one arrow callback"),
            );
            return None;
        }
        let Some(Expression::ArrowFunctionExpression(arrow)) = call.arguments[0].as_expression()
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.arguments[0].span(),
                format!("{role} map callback must be an arrow function"),
            );
            return None;
        };
        if arrow.r#async || arrow.params.rest.is_some() || arrow.params.items.len() > 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.span(),
                format!("{role} map callback must be synchronous `(item, index) => scalar`"),
            );
            return None;
        }

        let saved_frame = self.bindings.snapshot_frame();
        let expected_len = items.len();
        let mut values = Vec::with_capacity(expected_len);
        for (index, item) in items.into_iter().enumerate() {
            self.bindings.restore_frame(saved_frame.clone());
            if let Some(parameter) = arrow.params.items.first() {
                self.bind_static_pattern(&parameter.pattern, item);
            }
            if let Some(parameter) = arrow.params.items.get(1) {
                self.bind_static_pattern(&parameter.pattern, serde_json::json!(index));
            }
            let value = if let Some(expression) = arrow.get_expression() {
                self.lower_expr(expression)
            } else {
                let Some(body) = arrow.body.as_function_body() else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        arrow.body.span(),
                        format!("{role} map callback needs an expression or block body"),
                    );
                    self.bindings.restore_frame(saved_frame);
                    return None;
                };
                let mut result = None;
                for statement in &body.statements {
                    match statement {
                        Statement::VariableDeclaration(declaration)
                            if declaration.kind.is_const() && result.is_none() =>
                        {
                            for declarator in &declaration.declarations {
                                let Some(initializer) = &declarator.init else {
                                    self.illegal(
                                        DiagCode::GrammarForbidden,
                                        declarator.span(),
                                        format!("{role} map callback const needs an initializer"),
                                    );
                                    continue;
                                };
                                let BindingPattern::BindingIdentifier(identifier) = &declarator.id
                                else {
                                    self.illegal(
                                        DiagCode::GrammarForbidden,
                                        declarator.id.span(),
                                        format!(
                                            "{role} map callback scalar const must use a plain identifier"
                                        ),
                                    );
                                    continue;
                                };
                                let name = identifier.name.to_string();
                                if let Some(value) = self.eval_static(initializer) {
                                    self.bind_static(name, value);
                                } else if let Some(expr) = self.lower_expr(initializer) {
                                    self.bind_dynamic(name, expr);
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
                            format!(
                                "{role} map callback only allows const bindings and one returned scalar expression"
                            ),
                        ),
                    }
                }
                result
            };
            if let Some(value) = value {
                values.push(value);
            }
        }
        self.bindings.restore_frame(saved_frame);
        (values.len() == expected_len).then_some(values)
    }

    pub(super) fn lower_author_value(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<AuthorValue> {
        if let Some(value) = self.eval_static(expression) {
            return Some(AuthorValue::Static(value));
        }
        if self.is_dynamic_tuple_expression(expression) {
            return self
                .lower_dynamic_tuple_expression(expression, "component/helper argument")
                .map(AuthorValue::DynamicTuple);
        }
        self.lower_expr(expression).map(AuthorValue::Dynamic)
    }

    /// Bind JSX children in the same namespace as scalar and tuple values.
    pub(super) fn bind_children(&mut self, name: String, children: Vec<PendingNode>) {
        self.bindings.scalars.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.insert(name, children);
    }

    /// Detect names currently bound to runtime expressions; the sandbox cannot safely evaluate them
    /// using older static bindings.
    pub(super) fn shadowed_by_dynamic(&self, expression: &Expression<'_>) -> bool {
        if self.bindings.scalars.is_empty()
            && self.bindings.tuples.is_empty()
            && self.bindings.children.is_empty()
        {
            return false;
        }
        referenced_identifiers(expression).iter().any(|name| {
            self.bindings.scalars.contains_key(name)
                || self.bindings.tuples.contains_key(name)
                || self.bindings.children.contains_key(name)
        })
    }

    /// Evaluate a prepare-time expression or return None for runtime lowering. Preserve explicit
    /// builtin rejections so invalid inputs keep their original diagnostics.
    pub(super) fn eval_static(&mut self, expression: &Expression<'_>) -> Option<serde_json::Value> {
        // Reject static evaluation when runtime bindings shadow sandbox constants.
        if self.shadowed_by_dynamic(expression) {
            return None;
        }
        // Peel `as const` / satisfies / parens so the sandbox sees plain JS.
        let expression = peel_expr(expression);
        match self
            .sandbox
            .eval_json(self.src(expression), &self.static_bindings(), "prepare")
        {
            // Reject typed-value markers embedded in strings, including JSON.stringify results.
            // Object keys identifying legitimate typed values remain allowed.
            Ok(value) if string_leaks_typed_marker(&value) => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    expression.span(),
                    "a typed value's internal representation leaked into a string \
                     (JSON.stringify on a point/rect/path?); keep typed values typed — \
                     a template literal hole accepts one directly",
                );
                None
            }
            Ok(value) => Some(value),
            Err(diagnostic) => {
                if diagnostic.code == DiagCode::BuiltinRejected {
                    self.push_diagnostic(from_motion_diagnostic(
                        self.source,
                        diagnostic,
                        expression.span(),
                    ));
                }
                None
            }
        }
    }

    pub(super) fn bind_local_pattern(
        &mut self,
        pattern: &BindingPattern<'_>,
        initializer: &'s Expression<'s>,
    ) {
        if let Some(span) = reserved_theme_binding(pattern) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "`ThemeProvider` and `useTheme` are reserved compile-time Motion intrinsics and cannot be shadowed",
            );
            return;
        }
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                let name = identifier.name.to_string();
                // Register component-local function declarations as helpers instead of Motion
                // values.
                if let Some(authored) = authored_fn_of(initializer) {
                    self.bindings.local_functions.insert(name, authored);
                    return;
                }
                if let Some(value) = self.eval_static(initializer) {
                    self.bind_static(name, value);
                } else if self.is_dynamic_tuple_expression(initializer) {
                    if let Some(values) =
                        self.lower_dynamic_tuple_expression(initializer, "local dynamic tuple")
                    {
                        self.bind_dynamic_tuple(name, values);
                    }
                } else if let Some(expr) = self.lower_expr(initializer) {
                    self.bind_dynamic(name, expr);
                }
            }
            BindingPattern::ObjectPattern(pattern) => {
                if pattern.rest.is_some() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        pattern.span(),
                        "object rest destructuring is illegal because admitted fields must stay explicit",
                    );
                }
                if matches!(strip_parens(initializer), Expression::Identifier(id) if id.name == "props")
                {
                    self.bind_props_pattern(pattern);
                    return;
                }
                let Some(value) = self.eval_static(initializer) else {
                    self.unsupported(
                        initializer.span(),
                        "object destructuring requires props or prepare-time static data",
                    );
                    return;
                };
                self.bind_static_object_pattern(pattern, &value);
            }
            BindingPattern::ArrayPattern(pattern) => {
                if pattern.rest.is_some() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        pattern.span(),
                        "array rest destructuring is not admitted",
                    );
                }
                let Some(serde_json::Value::Array(values)) = self.eval_static(initializer) else {
                    self.unsupported(
                        initializer.span(),
                        "array destructuring requires prepare-time static data",
                    );
                    return;
                };
                for (binding, value) in pattern.elements.iter().zip(values) {
                    if let Some(binding) = binding {
                        self.bind_static_pattern(binding, value);
                    }
                }
            }
            BindingPattern::AssignmentPattern(_) => self.illegal(
                DiagCode::GrammarForbidden,
                pattern.span(),
                "a top-level local binding cannot itself be an assignment pattern",
            ),
        }
    }

    pub(super) fn bind_props_pattern(&mut self, pattern: &oxc::ast::ast::ObjectPattern<'_>) {
        for property in &pattern.properties {
            let Some(prop_name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    "computed props destructuring is illegal",
                );
                continue;
            };
            let Some(local_name) = binding_local_name(&property.value) else {
                self.unsupported(
                    property.value.span(),
                    "nested dynamic props destructuring is not yet in the scalar expression IR",
                );
                continue;
            };
            let value = self
                .bindings
                .component_props
                .as_ref()
                .and_then(|props| props.get(&prop_name))
                .cloned();
            match value {
                Some(AuthorValue::Dynamic(expr)) => {
                    self.bind_dynamic(local_name, expr);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::Static(value)) => {
                    self.bind_static(local_name, value);
                }
                Some(AuthorValue::Children(children)) => {
                    self.bind_children(local_name, children);
                }
                None if self.bindings.component_props.is_none() => {
                    if !self.controls.props.contains_key(&prop_name) {
                        self.illegal(
                            DiagCode::UnknownProp,
                            property.span(),
                            format!("`props.{prop_name}` is not declared in controls.props"),
                        );
                    } else {
                        let expr = self.push(Expr::Prop { name: prop_name }, property.span());
                        self.bind_dynamic(local_name, expr);
                    }
                }
                None => {
                    if let Some(default) = binding_default(&property.value)
                        && let Some(value) = self.eval_static(default)
                    {
                        self.bind_static(local_name, value);
                    } else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.span(),
                            format!("required component prop `{prop_name}` was not provided"),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn bind_static_object_pattern(
        &mut self,
        pattern: &oxc::ast::ast::ObjectPattern<'_>,
        value: &serde_json::Value,
    ) {
        let Some(object) = value.as_object() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                pattern.span(),
                "object destructuring source is not an object",
            );
            return;
        };
        for property in &pattern.properties {
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "computed destructuring is illegal",
                );
                continue;
            };
            if let Some(value) = object.get(&name).cloned() {
                self.bind_static_pattern(&property.value, value);
            } else if let Some(default) = binding_default(&property.value)
                && let Some(value) = self.eval_static(default)
            {
                self.bind_static_pattern(&property.value, value);
            } else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!("prepare object has no field `{name}` and no default"),
                );
            }
        }
    }

    pub(super) fn bind_static_pattern(
        &mut self,
        pattern: &BindingPattern<'_>,
        value: serde_json::Value,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                // Use mutually exclusive binding updates when static map items shadow outer runtime
                // values.
                self.bind_static(identifier.name.to_string(), value);
            }
            BindingPattern::AssignmentPattern(pattern) => {
                self.bind_static_pattern(&pattern.left, value);
            }
            BindingPattern::ObjectPattern(pattern) => {
                self.bind_static_object_pattern(pattern, &value)
            }
            BindingPattern::ArrayPattern(pattern) => {
                let Some(values) = value.as_array() else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        pattern.span(),
                        "array pattern source is not an array",
                    );
                    return;
                };
                for (binding, value) in pattern.elements.iter().zip(values.iter().cloned()) {
                    if let Some(binding) = binding {
                        self.bind_static_pattern(binding, value);
                    }
                }
            }
        }
    }

    pub(super) fn lower_node_expression(
        &mut self,
        expression: &'s Expression<'s>,
        path: &str,
    ) -> Option<Vec<PendingNode>> {
        let expression = strip_parens(expression);
        match expression {
            Expression::JSXElement(element) => self.lower_jsx(element, path).map(|node| vec![node]),
            Expression::NullLiteral(_) => Some(Vec::new()),
            Expression::Identifier(identifier) => self
                .bindings
                .children
                .get(identifier.name.as_str())
                .cloned()
                .or_else(|| {
                    self.unsupported(
                        expression.span(),
                        format!("`{}` is not a JSX children binding", identifier.name),
                    );
                    None
                }),
            Expression::StaticMemberExpression(member)
                if matches!(&member.object, Expression::Identifier(id) if id.name == "props")
                    && member.property.name == "children" =>
            {
                self.bindings
                    .component_props
                    .as_ref()
                    .and_then(|props| props.get("children"))
                    .and_then(|value| match value {
                        AuthorValue::Children(children) => Some(children.clone()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            expression.span(),
                            "`props.children` is only available inside a called component",
                        );
                        None
                    })
            }
            Expression::ConditionalExpression(conditional) => {
                if let Some(value) = self.eval_static(&conditional.test)
                    && let Some(condition) = value.as_bool()
                {
                    return self.lower_node_expression(
                        if condition {
                            &conditional.consequent
                        } else {
                            &conditional.alternate
                        },
                        path,
                    );
                }
                let condition = self.lower_expr(&conditional.test)?;
                let mut when_true =
                    self.lower_node_expression(&conditional.consequent, &format!("{path}.true"))?;
                let mut when_false =
                    self.lower_node_expression(&conditional.alternate, &format!("{path}.false"))?;
                self.apply_visibility(&mut when_true, condition);
                let not = self.not_expr(condition, conditional.test.span());
                self.apply_visibility(&mut when_false, not);
                when_true.extend(when_false);
                Some(when_true)
            }
            Expression::LogicalExpression(logical) if logical.operator == LogicalOperator::And => {
                if let Some(value) = self.eval_static(&logical.left)
                    && let Some(condition) = value.as_bool()
                {
                    return if condition {
                        self.lower_node_expression(&logical.right, path)
                    } else {
                        Some(Vec::new())
                    };
                }
                let condition = self.lower_expr(&logical.left)?;
                let mut nodes = self.lower_node_expression(&logical.right, path)?;
                self.apply_visibility(&mut nodes, condition);
                Some(nodes)
            }
            Expression::CallExpression(call) if self.is_static_map_call(call) => {
                self.lower_static_map(call, path)
            }
            // Recognize JSX helpers by their registered names rather than inspecting return syntax.
            Expression::CallExpression(call) => {
                let Expression::Identifier(callee) = &call.callee else {
                    self.unsupported(
                        expression.span(),
                        "component children must be JSX, static conditions, fixed-topology conditions, children, a prepare-time array map, or a call to a JSX-returning helper",
                    );
                    return None;
                };
                let name = callee.name.to_string();
                let Some((function, captures_scope)) = self.authored_fn(name.as_str()) else {
                    self.unsupported(
                        expression.span(),
                        format!(
                            "`{name}` is not a declared helper or component; JSX children must be \
                             JSX, static conditions, fixed-topology conditions, children, a \
                             prepare-time array map, or a call to a JSX-returning helper"
                        ),
                    );
                    return None;
                };
                self.lower_node_helper(&name, function, captures_scope, call, path)
            }
            _ => {
                self.unsupported(
                    expression.span(),
                    "component children must be JSX, static conditions, fixed-topology conditions, children, or a prepare-time array map",
                );
                None
            }
        }
    }

    /// Inline JSX helpers at the call site with positional arguments. Reuse normal JSX lowering for
    /// keys, lists, and coordinate-space rules.
    pub(super) fn lower_node_helper(
        &mut self,
        name: &str,
        function: AuthoredFn<'s>,
        captures_scope: bool,
        call: &'s oxc::ast::ast::CallExpression<'s>,
        path: &str,
    ) -> Option<Vec<PendingNode>> {
        if self.helper_stack.iter().any(|entry| entry == name) {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!("recursive helper `{name}` cannot be expanded at compile time"),
            );
            return None;
        }
        self.expanded_helper_calls += 1;
        if self.expanded_helper_calls > MAX_EXPANDED_HELPER_CALLS {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!(
                    "helper expansion exceeded the module budget of {MAX_EXPANDED_HELPER_CALLS} \
                     inline calls; every call site inlines the whole body, so call chains fan \
                     out multiplicatively — precompute the values at prepare time (an array + \
                     `.map`) or flatten the helper chain"
                ),
            );
            return None;
        }
        if function.is_async() || function.is_generator() || function.params().rest.is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                function.span(),
                format!("helper `{name}` cannot be async, a generator, or variadic"),
            );
            return None;
        }
        if call.arguments.len() > function.params().items.len() {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!("helper `{name}` received too many arguments"),
            );
            return None;
        }
        if function.body().is_none() && function.expression_body().is_none() {
            self.illegal(
                DiagCode::ModuleShape,
                function.span(),
                format!("helper `{name}` has no body"),
            );
            return None;
        }

        // Evaluate arguments in the caller's scope before entering the helper scope.
        let mut values = Vec::with_capacity(call.arguments.len());
        for argument in &call.arguments {
            let Some(expression) = argument.as_expression() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    argument.span(),
                    format!("helper `{name}` arguments cannot spread"),
                );
                return None;
            };
            values.push(self.lower_author_value(expression)?);
        }

        // Local helpers capture outer bindings; module helpers use an isolated value scope. Save
        // and restore every binding kind together.
        let saved_bindings = self.bindings.enter_helper_scope(captures_scope);
        self.helper_stack.push(name.to_string());

        for (index, parameter) in function.params().items.iter().enumerate() {
            let Some(local_name) = parameter
                .pattern
                .get_identifier_name()
                .map(|name| name.to_string())
            else {
                // Only plain identifier parameters have supported binding semantics.
                self.unsupported(
                    parameter.span(),
                    format!("helper `{name}` currently requires plain identifier parameters"),
                );
                continue;
            };
            let value = values.get(index).cloned().or_else(|| {
                parameter
                    .initializer
                    .as_deref()
                    .and_then(|default| self.eval_static(default))
                    .map(AuthorValue::Static)
            });
            match value {
                Some(AuthorValue::Static(value)) => {
                    self.bind_static(local_name, value);
                }
                Some(AuthorValue::Dynamic(expr)) => {
                    self.bind_dynamic(local_name, expr);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::Children(_)) => unreachable!("helper arguments are values"),
                None => self.illegal(
                    DiagCode::GrammarForbidden,
                    parameter.span(),
                    format!("helper `{name}` is missing argument `{local_name}`"),
                ),
            }
        }

        let mut nodes = function
            .expression_body()
            .and_then(|expression| self.lower_node_expression(strip_parens(expression), path));
        if let Some(body) = function.body() {
            for statement in &body.statements {
                match statement {
                    Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                        for declarator in &declaration.declarations {
                            let Some(initializer) = &declarator.init else {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    declarator.span(),
                                    "helper const needs an initializer",
                                );
                                continue;
                            };
                            self.bind_local_pattern(&declarator.id, initializer);
                        }
                    }
                    Statement::ReturnStatement(statement) if nodes.is_none() => {
                        nodes = statement.argument.as_ref().and_then(|value| {
                            self.lower_node_expression(strip_parens(value), path)
                        });
                    }
                    _ => self.illegal(
                        DiagCode::GrammarForbidden,
                        statement.span(),
                        format!(
                            "helper `{name}` only allows const bindings and one returned JSX expression"
                        ),
                    ),
                }
            }
        }

        self.helper_stack.pop();
        self.bindings.restore(saved_bindings);
        if nodes.is_none() && self.diagnostics.is_empty() {
            self.illegal(
                DiagCode::GrammarForbidden,
                function.span(),
                format!("helper `{name}` must return JSX"),
            );
        }
        nodes
    }

    pub(super) fn is_static_map_call(&self, call: &oxc::ast::ast::CallExpression<'_>) -> bool {
        matches!(
            &call.callee,
            Expression::StaticMemberExpression(member) if member.property.name == "map"
        )
    }

    pub(super) fn lower_static_map(
        &mut self,
        call: &'s oxc::ast::ast::CallExpression<'s>,
        path: &str,
    ) -> Option<Vec<PendingNode>> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let Some(serde_json::Value::Array(items)) = self.eval_static(&member.object) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                "JSX array map input must be prepare-time static",
            );
            return None;
        };
        if items.len() > MAX_STATIC_MAP_ITEMS
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "prepare-time map expands {} items; the per-map limit is {MAX_STATIC_MAP_ITEMS} and the module budget is {MAX_TOTAL_EXPANDED_LIST_ITEMS}",
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
                "array map requires one arrow callback",
            );
            return None;
        }
        let Some(Expression::ArrowFunctionExpression(arrow)) = call.arguments[0].as_expression()
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.arguments[0].span(),
                "array map callback must be an arrow function",
            );
            return None;
        };
        if arrow.r#async || arrow.params.rest.is_some() || arrow.params.items.len() > 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.span(),
                "array map callback must be synchronous `(item, index) => JSX`",
            );
            return None;
        }
        // Restore the complete lexical binding frame between map iterations and after the map.
        let saved_frame = self.bindings.snapshot_frame();
        let mut nodes = Vec::new();
        self.list_depth += 1;
        for (index, item) in items.into_iter().enumerate() {
            self.bindings.restore_frame(saved_frame.clone());
            if let Some(parameter) = arrow.params.items.first() {
                self.bind_static_pattern(&parameter.pattern, item);
            }
            if let Some(parameter) = arrow.params.items.get(1) {
                self.bind_static_pattern(&parameter.pattern, serde_json::json!(index));
            }
            if let Some(mut item_nodes) =
                self.lower_static_map_callback(arrow, &format!("{path}.{index}"))
            {
                nodes.append(&mut item_nodes);
            }
        }
        self.list_depth -= 1;
        self.bindings.restore_frame(saved_frame);
        Some(nodes)
    }

    pub(super) fn lower_static_map_callback(
        &mut self,
        arrow: &'s oxc::ast::ast::ArrowFunctionExpression<'s>,
        path: &str,
    ) -> Option<Vec<PendingNode>> {
        if let Some(expression) = arrow.get_expression() {
            return self.lower_node_expression(expression, path);
        }
        let Some(body) = arrow.body.as_function_body() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.body.span(),
                "array map callback needs an expression or block body",
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
                            "array map callback cannot execute declarations after return",
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
                        self.bind_local_pattern(&declarator.id, initializer);
                    }
                }
                Statement::ReturnStatement(statement) if result.is_none() => {
                    result = statement
                        .argument
                        .as_ref()
                        .and_then(|expression| self.lower_node_expression(expression, path));
                }
                other => self.illegal(
                    DiagCode::GrammarForbidden,
                    other.span(),
                    "array map callback only allows prepare-time const bindings and one returned JSX expression",
                ),
            }
        }
        if result.is_none() && self.diagnostics.is_empty() {
            self.illegal(
                DiagCode::GrammarForbidden,
                body.span(),
                "array map callback must return JSX",
            );
        }
        result
    }

    /// Resolve local helpers before module helpers. Local helpers capture the component's immutable
    /// bindings; module helpers use module constants and parameters. Parameters shadow captured
    /// names.
    pub(super) fn authored_fn(&self, name: &str) -> Option<(AuthoredFn<'s>, bool)> {
        if let Some(local) = self.bindings.local_functions.get(name) {
            return Some((*local, true));
        }
        self.functions.get(name).map(|f| (*f, false))
    }

    pub(super) fn scoped_key(&self, key: &str) -> String {
        if self.key_prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{key}", self.key_prefix)
        }
    }

    pub(super) fn not_expr(&mut self, input: ExprId, span: Span) -> ExprId {
        let false_value = self.push(
            Expr::Const {
                value: MotionValue::Bool(false),
            },
            span,
        );
        self.push(
            Expr::Compare {
                op: CompareOp::Eq,
                lhs: input,
                rhs: false_value,
            },
            span,
        )
    }

    pub(super) fn and_expr(&mut self, lhs: ExprId, rhs: ExprId, span: Span) -> ExprId {
        let false_value = self.push(
            Expr::Const {
                value: MotionValue::Bool(false),
            },
            span,
        );
        self.push(
            Expr::Select {
                condition: lhs,
                when_true: rhs,
                when_false: false_value,
            },
            span,
        )
    }

    pub(super) fn apply_visibility(&mut self, nodes: &mut [PendingNode], condition: ExprId) {
        for node in nodes {
            node.visibility = Some(match node.visibility {
                Some(existing) => self.and_expr(existing, condition, node.span),
                None => condition,
            });
        }
    }

    pub(super) fn lower_component(
        &mut self,
        element: &'s JSXElement<'s>,
        name: &str,
        function: AuthoredFn<'s>,
        path: &str,
    ) -> Option<PendingNode> {
        if self.component_stack.iter().any(|entry| entry == name) {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.span(),
                format!("recursive component expansion is illegal: {name}"),
            );
            return None;
        }

        let mut props = BTreeMap::new();
        let mut instance_key = None;
        for attribute in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    "component prop spread is illegal; props must remain explicit",
                );
                continue;
            };
            let JSXAttributeName::Identifier(prop_name) = &attribute.name else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.name.span(),
                    "namespaced component props are illegal",
                );
                continue;
            };
            let prop_name = prop_name.name.to_string();
            if prop_name == "key" {
                instance_key = self.attr_static_string(&attribute.value, attribute.span(), "key");
                continue;
            }
            let value = match &attribute.value {
                None => Some(AuthorValue::Static(serde_json::Value::Bool(true))),
                Some(JSXAttributeValue::StringLiteral(value)) => Some(AuthorValue::Static(
                    serde_json::Value::String(value.value.to_string()),
                )),
                Some(JSXAttributeValue::ExpressionContainer(container)) => {
                    let Some(expression) = container.expression.as_expression() else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            container.span(),
                            "component prop expression cannot be empty",
                        );
                        continue;
                    };
                    self.lower_author_value(expression)
                }
                _ => {
                    self.unsupported(
                        attribute.span(),
                        "component props cannot contain JSX values",
                    );
                    None
                }
            };
            if let Some(value) = value
                && props.insert(prop_name.clone(), value).is_some()
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    format!("duplicate component prop `{prop_name}`"),
                );
            }
        }
        if self.list_depth > 0 && instance_key.is_none() {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.opening_element.span(),
                "every component expanded from a data list needs a stable `key`",
            );
        }
        let instance_key = instance_key.unwrap_or_else(|| path.to_string());
        let caller_prefix = self.key_prefix.clone();
        let component_prefix = self.scoped_key(&instance_key);
        self.key_prefix = component_prefix.clone();

        let mut children = Vec::new();
        for (index, child) in element.children.iter().enumerate() {
            match child {
                JSXChild::Element(child) => {
                    if let Some(node) = self.lower_jsx(child, &format!("{path}.children.{index}")) {
                        children.push(node);
                    }
                }
                JSXChild::ExpressionContainer(container)
                    if matches!(container.expression, JSXExpression::EmptyExpression(_)) => {}
                JSXChild::ExpressionContainer(container) => {
                    if let Some(expression) = container.expression.as_expression()
                        && let Some(mut lowered) = self
                            .lower_node_expression(expression, &format!("{path}.children.{index}"))
                    {
                        children.append(&mut lowered);
                    }
                }
                JSXChild::Text(text) if text.value.trim().is_empty() => {}
                _ => self.unsupported(
                    child.span(),
                    "component children must be Motion JSX nodes; wrap text in `<Text>`",
                ),
            }
        }
        self.key_prefix = caller_prefix;
        props.insert("children".into(), AuthorValue::Children(children));

        if function.body().is_none() && function.expression_body().is_none() {
            self.illegal(
                DiagCode::ModuleShape,
                function.span(),
                format!("component `{name}` has no body"),
            );
            return None;
        }
        let params = function.params();
        if params.rest.is_some() || params.items.len() > 3 {
            self.illegal(
                DiagCode::ModuleShape,
                params.span(),
                format!("component `{name}` must use `(ctx, props, signals)` parameters"),
            );
            return None;
        }

        let saved_bindings = self.bindings.enter_isolated_scope();
        let saved_prefix = self.key_prefix.clone();
        self.key_prefix = component_prefix;
        self.bindings.component_props = Some(props);
        self.component_stack.push(name.to_string());

        if let Some(first) = params.items.first()
            && first.pattern.get_identifier_name().map(|id| id.as_str()) != Some("ctx")
        {
            self.illegal(
                DiagCode::ModuleShape,
                first.span(),
                "component first parameter must be `ctx`",
            );
        }
        if let Some(second) = params.items.get(1) {
            match &second.pattern {
                BindingPattern::BindingIdentifier(identifier) if identifier.name == "props" => {}
                BindingPattern::ObjectPattern(pattern) => self.bind_props_pattern(pattern),
                _ => self.illegal(
                    DiagCode::ModuleShape,
                    second.span(),
                    "component second parameter must be `props` or an object destructure",
                ),
            }
        }
        if let Some(third) = params.items.get(2)
            && third.pattern.get_identifier_name().map(|id| id.as_str()) != Some("signals")
        {
            self.illegal(
                DiagCode::ModuleShape,
                third.span(),
                "component third parameter must be `signals`",
            );
        }

        let result = if let Some(expression) = function.expression_body() {
            self.compile_root_expression(expression)
        } else {
            self.compile_body(function.body().expect("component block body checked above"))
        };
        self.component_stack.pop();
        self.bindings.restore(saved_bindings);
        self.key_prefix = saved_prefix;
        result
    }

    pub(super) fn lower_dynamic_helper(
        &mut self,
        name: &str,
        function: AuthoredFn<'s>,
        captures_scope: bool,
        arguments: &[Argument<'_>],
        call_span: Span,
    ) -> Option<ExprId> {
        if self.helper_stack.iter().any(|entry| entry == name) {
            self.illegal(
                DiagCode::GrammarForbidden,
                call_span,
                format!("recursive pure helper `{name}` cannot be bounded at compile time"),
            );
            return None;
        }
        self.expanded_helper_calls += 1;
        if self.expanded_helper_calls > MAX_EXPANDED_HELPER_CALLS {
            self.illegal(
                DiagCode::GrammarForbidden,
                call_span,
                format!(
                    "helper expansion exceeded the module budget of {MAX_EXPANDED_HELPER_CALLS} \
                     inline calls; every call site inlines the whole body, so call chains fan \
                     out multiplicatively — precompute the values at prepare time (an array + \
                     `.map`) or flatten the helper chain"
                ),
            );
            return None;
        }
        if function.is_async() || function.is_generator() || function.params().rest.is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                function.span(),
                format!("pure helper `{name}` cannot be async, a generator, or variadic"),
            );
            return None;
        }
        if arguments.len() > function.params().items.len() {
            self.illegal(
                DiagCode::GrammarForbidden,
                call_span,
                format!("pure helper `{name}` received too many arguments"),
            );
            return None;
        }
        let mut values = Vec::new();
        for argument in arguments {
            let Some(expression) = argument.as_expression() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    argument.span(),
                    "pure helper arguments cannot spread",
                );
                return None;
            };
            values.push(self.lower_author_value(expression)?);
        }

        if function.body().is_none() && function.expression_body().is_none() {
            self.illegal(
                DiagCode::ModuleShape,
                function.span(),
                format!("pure helper `{name}` has no body"),
            );
            return None;
        }
        let saved_bindings = self.bindings.enter_helper_scope(captures_scope);
        self.helper_stack.push(name.to_string());

        for (index, parameter) in function.params().items.iter().enumerate() {
            let Some(local_name) = parameter
                .pattern
                .get_identifier_name()
                .map(|name| name.to_string())
            else {
                self.unsupported(
                    parameter.span(),
                    format!(
                        "dynamic helper `{name}` currently requires scalar identifier parameters"
                    ),
                );
                continue;
            };
            let value = values.get(index).cloned().or_else(|| {
                parameter
                    .initializer
                    .as_deref()
                    .and_then(|default| self.eval_static(default))
                    .map(AuthorValue::Static)
            });
            match value {
                Some(AuthorValue::Static(value)) => {
                    self.bind_static(local_name, value);
                }
                Some(AuthorValue::Dynamic(expr)) => {
                    self.bind_dynamic(local_name, expr);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::Children(_)) => unreachable!("helper arguments are values"),
                None => self.illegal(
                    DiagCode::GrammarForbidden,
                    parameter.span(),
                    format!("pure helper `{name}` is missing argument `{local_name}`"),
                ),
            }
        }

        let mut result = function
            .expression_body()
            .and_then(|expression| self.lower_expr(expression));
        if let Some(body) = function.body() {
            for statement in &body.statements {
                match statement {
                    Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                        for declarator in &declaration.declarations {
                            let Some(initializer) = &declarator.init else {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    declarator.span(),
                                    "pure helper const needs an initializer",
                                );
                                continue;
                            };
                            self.bind_local_pattern(&declarator.id, initializer);
                        }
                    }
                    Statement::ReturnStatement(statement) if result.is_none() => {
                        result = statement
                            .argument
                            .as_ref()
                            .and_then(|value| self.lower_expr(value));
                    }
                    _ => self.illegal(
                        DiagCode::GrammarForbidden,
                        statement.span(),
                        format!(
                            "pure helper `{name}` only allows const bindings and one return expression"
                        ),
                    ),
                }
            }
        }

        self.helper_stack.pop();
        self.bindings.restore(saved_bindings);
        if result.is_none() && self.diagnostics.is_empty() {
            self.illegal(
                DiagCode::GrammarForbidden,
                function.span(),
                format!("pure helper `{name}` must return one scalar expression"),
            );
        }
        result
    }
}
