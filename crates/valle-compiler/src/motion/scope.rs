//! Lexical bindings, component/helper expansion, and compile-time list scopes.

use super::*;

struct InstanceAttemptState {
    keys: BTreeSet<String>,
    layout_ids: BTreeSet<String>,
    capabilities: BTreeSet<String>,
    used_font_controls: BTreeSet<String>,
    camera: Option<CameraBinding>,
    source_node_spans_len: usize,
    source_node_stacks_len: usize,
    source_object_spans_len: usize,
    resource_refs_len: usize,
    instance_groups_len: usize,
    diagnostics_len: usize,
    warnings_len: usize,
    expanded_helper_calls: usize,
    expanded_list_items: usize,
}

impl<'s> Compiler<'s> {
    fn snapshot_instance_attempt(&self) -> InstanceAttemptState {
        InstanceAttemptState {
            keys: self.keys.clone(),
            layout_ids: self.layout_ids.clone(),
            capabilities: self.extra_capabilities.clone(),
            used_font_controls: self.used_font_controls.clone(),
            camera: self.camera.clone(),
            source_node_spans_len: self.source_ledger.node_spans.len(),
            source_node_stacks_len: self.source_ledger.node_expansion_stacks.len(),
            source_object_spans_len: self.source_ledger.object_spans.len(),
            resource_refs_len: self.resource_refs.len(),
            instance_groups_len: self.instance_groups.len(),
            diagnostics_len: self.diagnostics.len(),
            warnings_len: self.warnings.len(),
            expanded_helper_calls: self.expanded_helper_calls,
            expanded_list_items: self.expanded_list_items,
        }
    }

    fn restore_instance_attempt(&mut self, state: InstanceAttemptState) {
        self.keys = state.keys;
        self.layout_ids = state.layout_ids;
        self.extra_capabilities = state.capabilities;
        self.used_font_controls = state.used_font_controls;
        self.camera = state.camera;
        self.source_ledger
            .node_spans
            .truncate(state.source_node_spans_len);
        self.source_ledger
            .node_expansion_stacks
            .truncate(state.source_node_stacks_len);
        self.source_ledger
            .object_spans
            .truncate(state.source_object_spans_len);
        self.resource_refs.truncate(state.resource_refs_len);
        self.instance_groups.truncate(state.instance_groups_len);
        self.source_ledger
            .instance_exprs
            .truncate(state.instance_groups_len);
        self.diagnostics.truncate(state.diagnostics_len);
        self.warnings.truncate(state.warnings_len);
        self.expanded_helper_calls = state.expanded_helper_calls;
        self.expanded_list_items = state.expanded_list_items;
    }

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
        self.bindings.objects.remove(&name);
        self.bindings.instance_objects.remove(&name);
        self.bindings.scalars.remove(&name);
        self.bindings.paints.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.statics.insert(name, value);
    }

    /// Bind a runtime expression and invalidate static or tuple bindings with the same name.
    /// Otherwise the sandbox could fold a shadowed outer value into a constant.
    pub(super) fn bind_dynamic(&mut self, name: String, expr: ExprId) {
        self.bindings.objects.remove(&name);
        self.bindings.instance_objects.remove(&name);
        self.bindings.paints.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.scalars.insert(name, expr);
    }

    /// Bind a fixed-length collection of frame-time expressions. The collection
    /// only exists while lowering author code; every indexed use resolves back to
    /// one scalar ExprId before the SceneArtifact is emitted.
    pub(super) fn bind_dynamic_tuple(&mut self, name: String, values: Vec<ExprId>) {
        self.bindings.objects.remove(&name);
        self.bindings.instance_objects.remove(&name);
        self.bindings.scalars.remove(&name);
        self.bindings.paints.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.tuples.insert(name, values);
    }

    pub(super) fn bind_paint(&mut self, name: String, paint: PaintValue) {
        self.bindings.objects.remove(&name);
        self.bindings.instance_objects.remove(&name);
        self.bindings.scalars.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.paints.insert(name, paint);
    }

    pub(super) fn bind_instance_object(
        &mut self,
        name: String,
        fields: BTreeMap<String, (u32, valle_motion::expr::ExprType)>,
    ) {
        self.bindings.objects.remove(&name);
        self.bindings.scalars.remove(&name);
        self.bindings.paints.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.remove(&name);
        self.bindings.instance_objects.insert(name, fields);
    }

    pub(super) fn instance_object_fields(
        &self,
        expression: &Expression<'_>,
    ) -> Option<BTreeMap<String, (u32, valle_motion::expr::ExprType)>> {
        match peel_expr(expression) {
            Expression::Identifier(id) => self
                .instance_scope
                .as_ref()
                .filter(|scope| scope.parameter == id.name.as_str())
                .map(|scope| scope.fields.clone())
                .or_else(|| {
                    self.bindings
                        .instance_objects
                        .get(id.name.as_str())
                        .cloned()
                }),
            Expression::StaticMemberExpression(member) if matches!(&member.object, Expression::Identifier(id) if id.name == "props") => {
                self.bindings
                    .component_props
                    .as_ref()
                    .and_then(|props| props.get(member.property.name.as_str()))
                    .and_then(|value| match value {
                        AuthorValue::InstanceObject(fields) => Some(fields.clone()),
                        _ => None,
                    })
            }
            _ => None,
        }
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
        if let Some(paint) = self.bound_paint(expression) {
            return Some(AuthorValue::Paint(paint));
        }
        if let Some(fields) = self.instance_object_fields(expression) {
            return Some(AuthorValue::InstanceObject(fields));
        }
        if let Some(value) = self.eval_static(expression) {
            return Some(AuthorValue::Static(value));
        }
        if let Expression::CallExpression(call) = peel_expr(expression)
            && let Expression::Identifier(callee) = &call.callee
            && matches!(
                callee.name.as_str(),
                "linearGradient" | "radialGradient" | "conicGradient"
            )
        {
            return self
                .lower_gradient_call(call, expression.span())
                .map(AuthorValue::Paint);
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
        self.bindings.objects.remove(&name);
        self.bindings.instance_objects.remove(&name);
        self.bindings.scalars.remove(&name);
        self.bindings.paints.remove(&name);
        self.bindings.tuples.remove(&name);
        self.bindings.statics.remove(&name);
        self.bindings.children.insert(name, children);
    }

    /// Detect names currently bound to runtime expressions; the sandbox cannot safely evaluate them
    /// using older static bindings.
    pub(super) fn shadowed_by_dynamic(&self, expression: &Expression<'_>) -> bool {
        if self
            .instance_scope
            .as_ref()
            .is_some_and(|scope| referenced_identifiers(expression).contains(&scope.parameter))
        {
            return true;
        }
        if self.bindings.scalars.is_empty()
            && self.bindings.paints.is_empty()
            && self.bindings.tuples.is_empty()
            && self.bindings.instance_objects.is_empty()
            && self.bindings.children.is_empty()
            && self.bindings.objects.is_empty()
        {
            return false;
        }
        referenced_identifiers(expression).iter().any(|name| {
            self.bindings.scalars.contains_key(name)
                || self.bindings.paints.contains_key(name)
                || self.bindings.tuples.contains_key(name)
                || self.bindings.instance_objects.contains_key(name)
                || self.bindings.children.contains_key(name)
                || (self.bindings.objects.contains_key(name)
                    && !self.bindings.statics.contains_key(name))
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
                    self.bindings.objects.remove(&name);
                    self.bindings.paints.remove(&name);
                    self.bindings.local_functions.insert(name, authored);
                    return;
                }
                if let Some(fields) = self.instance_object_fields(initializer) {
                    self.bind_instance_object(name, fields);
                    return;
                }
                let object = self.capture_authored_object(initializer);
                if let Some(value) = self.eval_static(initializer) {
                    self.bind_static(name.clone(), value);
                    if let Some(object) = object {
                        self.bindings.objects.insert(name, object);
                    }
                } else if let Some(paint) = self.bound_paint(initializer) {
                    self.bind_paint(name, paint);
                } else if let Expression::CallExpression(call) = peel_expr(initializer)
                    && let Expression::Identifier(callee) = &call.callee
                    && matches!(
                        callee.name.as_str(),
                        "linearGradient" | "radialGradient" | "conicGradient"
                    )
                {
                    if let Some(paint) = self.lower_gradient_call(call, initializer.span()) {
                        self.bind_paint(name, paint);
                    }
                } else if let Some(object) = object {
                    self.bindings.scalars.remove(&name);
                    self.bindings.paints.remove(&name);
                    self.bindings.tuples.remove(&name);
                    self.bindings.statics.remove(&name);
                    self.bindings.children.remove(&name);
                    self.bindings.objects.insert(name, object);
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
                if let Some(fields) = self.instance_object_fields(initializer) {
                    self.bind_instance_object_pattern(pattern, &fields);
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
                    "nested props destructuring is not supported; bind one scalar field at a time",
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
                Some(AuthorValue::Paint(paint)) => {
                    self.bind_paint(local_name, paint);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::InstanceObject(fields)) => {
                    self.bind_instance_object(local_name, fields);
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

    fn bind_instance_object_pattern(
        &mut self,
        pattern: &oxc::ast::ast::ObjectPattern<'_>,
        fields: &BTreeMap<String, (u32, valle_motion::expr::ExprType)>,
    ) {
        for property in &pattern.properties {
            let Some(field) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "computed instance fields are illegal",
                );
                continue;
            };
            let Some(name) = binding_local_name(&property.value) else {
                self.unsupported(
                    property.span(),
                    "nested instance field destructuring is unsupported",
                );
                continue;
            };
            let Some((column, value_type)) = fields.get(&field).copied() else {
                self.illegal(
                    DiagCode::UnknownIdentifier,
                    property.span(),
                    format!("unknown instance field `{field}`"),
                );
                continue;
            };
            let expr = self.push(Expr::InstanceField { column, value_type }, property.span());
            self.bind_dynamic(name, expr);
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
                Some(AuthorValue::Paint(paint)) => {
                    self.bind_paint(local_name, paint);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::InstanceObject(fields)) => {
                    self.bind_instance_object(local_name, fields);
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
        let mut fallback_reason = None;
        if items.len() <= valle_motion::MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE
            && let Some(nodes) = self.try_lower_instance_map(
                &items,
                &member.object,
                arrow,
                path,
                &mut fallback_reason,
            )
        {
            return Some(nodes);
        }
        if items.len() > MAX_STATIC_MAP_ITEMS
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "prepare-time map expands {} items; the per-map limit is {MAX_STATIC_MAP_ITEMS} and the module budget is {MAX_TOTAL_EXPANDED_LIST_ITEMS}; homogeneous absolute View or translated solid Path instance groups can use up to {} rows",
                    items.len(), valle_motion::MAX_GEOMETRY_BATCH_INSTANCES_PER_NODE,
                ),
            );
            return None;
        }
        // Small authored lists are ordinary composition, not a performance problem.
        if items.len() >= 64 {
            self.warn(
                DiagCode::InstanceFallback,
                call.span(),
                format!(
                    "static JSX map expands {} rows: {}",
                    items.len(),
                    fallback_reason.unwrap_or_else(|| self.instance_map_fallback_reason(arrow)),
                ),
            );
        }
        self.expanded_list_items += items.len();
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

    fn instance_map_fallback_reason(
        &self,
        arrow: &oxc::ast::ast::ArrowFunctionExpression<'s>,
    ) -> String {
        let Some(Expression::JSXElement(element)) = arrow.get_expression().map(peel_expr) else {
            return "callback does not return one JSX element".into();
        };
        let tag = match &element.opening_element.name {
            JSXElementName::Identifier(id) => id.name.as_str(),
            JSXElementName::IdentifierReference(id) => id.name.as_str(),
            _ => return "template component name is dynamic".into(),
        };
        if self.authored_fn(tag).is_some() {
            return "component instance needs a direct item key, supported row fields, and a fixed in-flow View/Group/Text tree".into();
        }
        if !matches!(tag, "View" | "Group" | "Text" | "Circle" | "Path") {
            return format!("{tag} has no instance template renderer");
        }
        if !element.children.is_empty() && tag != "Text" {
            return "template has children and needs per-instance layout".into();
        }
        let mut class_name = None;
        let mut style_properties = BTreeSet::new();
        let mut attributes = BTreeSet::new();
        for attribute in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                return "spread attributes prevent a fixed template".into();
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                return "namespaced attributes prevent a fixed template".into();
            };
            let name = name.name.as_str();
            attributes.insert(name);
            if name == "className" {
                class_name = match &attribute.value {
                    Some(JSXAttributeValue::StringLiteral(value)) => Some(value.value.as_str()),
                    _ => None,
                };
            }
            if name == "style" {
                let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value
                else {
                    return "style must be a fixed object literal".into();
                };
                let Some(Expression::ObjectExpression(object)) =
                    container.expression.as_expression().map(peel_expr)
                else {
                    return "style must be a fixed object literal".into();
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return "style spread prevents a fixed template".into();
                    };
                    let name = match &property.key {
                        PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                        PropertyKey::StringLiteral(id) => id.value.as_str(),
                        _ => return "computed style keys prevent a fixed template".into(),
                    };
                    style_properties.insert(name);
                }
            }
        }
        if tag == "View"
            && class_name.is_some_and(|name| {
                name != "absolute" && name.split_whitespace().any(|class| class == "absolute")
            })
        {
            return "View needs exact className=\"absolute\" for absolute batching".into();
        }
        if !attributes.contains("key") {
            return "template leaf needs a key from the array item to preserve row identity".into();
        }
        let supported_style = |name: &str| match tag {
            "View" | "Group" => matches!(
                name,
                "left"
                    | "top"
                    | "width"
                    | "height"
                    | "backgroundColor"
                    | "opacity"
                    | "transform"
                    | "minWidth"
                    | "minHeight"
                    | "maxWidth"
                    | "maxHeight"
                    | "marginTop"
                    | "marginRight"
                    | "marginBottom"
                    | "marginLeft"
                    | "paddingTop"
                    | "paddingRight"
                    | "paddingBottom"
                    | "paddingLeft"
                    | "borderRadius"
            ),
            "Text" => matches!(
                name,
                "width"
                    | "height"
                    | "fontSize"
                    | "fontWeight"
                    | "letterSpacing"
                    | "lineHeight"
                    | "color"
                    | "opacity"
                    | "textAlign"
                    | "whiteSpace"
            ),
            "Circle" => name == "opacity",
            "Path" => matches!(
                name,
                "translate" | "opacity" | "rotate" | "scale" | "transform" | "transformOrigin"
            ),
            _ => false,
        };
        if let Some(name) = style_properties.iter().find(|name| !supported_style(name)) {
            return format!("{tag} style.{name} has no pixel-equivalent instance lowering");
        }
        if tag == "View" && class_name == Some("absolute") {
            if let Some(name) = style_properties.iter().find(|name| {
                matches!(
                    **name,
                    "minWidth"
                        | "minHeight"
                        | "maxWidth"
                        | "maxHeight"
                        | "marginTop"
                        | "marginRight"
                        | "marginBottom"
                        | "marginLeft"
                        | "paddingTop"
                        | "paddingRight"
                        | "paddingBottom"
                        | "paddingLeft"
                        | "borderRadius"
                )
            }) {
                return format!(
                    "View style.{name} needs the per-row layout template instead of the absolute batch template"
                );
            }
        }
        if tag == "View" && class_name != Some("absolute") {
            if let Some(name) = style_properties
                .iter()
                .find(|name| matches!(**name, "left" | "top" | "transform"))
            {
                return format!("View style.{name} needs the exact absolute batch template");
            }
        }
        if tag == "View"
            && class_name == Some("absolute")
            && let Some(name) = ["left", "top", "width", "height", "backgroundColor"]
                .into_iter()
                .find(|name| !style_properties.contains(name))
        {
            return format!("View style.{name} is required by the absolute box template");
        }
        if tag == "Path" {
            if let Some(name) = attributes.iter().find(|name| {
                !matches!(
                    **name,
                    "key" | "d" | "fill" | "stroke" | "strokeWidth" | "style" | "visible"
                )
            }) {
                return format!("Path {name} has no shared-geometry instance lowering");
            }
            if !attributes.contains("d") || !attributes.contains("fill") {
                return "Path needs a shared static d and solid fill".into();
            }
            if !style_properties.contains("translate") {
                return "Path needs a pixel translate style".into();
            }
            if (style_properties.contains("rotate")
                || style_properties.contains("scale")
                || style_properties.contains("transform"))
                && !style_properties.contains("transformOrigin")
            {
                return "Path rotation/scale/skew needs transformOrigin: point(0, 0)".into();
            }
        }
        if tag == "Circle" {
            if let Some(name) = attributes.iter().find(|name| {
                !matches!(
                    **name,
                    "key" | "cx" | "cy" | "r" | "fill" | "style" | "visible"
                )
            }) {
                return format!("Circle {name} has no full-arc instance lowering");
            }
        }
        if self.geometry_references.unknown_key {
            return "dynamic bounds()/anchor() references need per-row layout".into();
        }
        "row fields, value types, or geometry references do not meet the instance template contract"
            .into()
    }

    /// Compile a homogeneous template once, keeping row values in columns. Flow roots may
    /// be View, Group, or plain Text; each row still receives its own layout.
    fn try_lower_instance_map(
        &mut self,
        items: &[serde_json::Value],
        source: &'s Expression<'s>,
        arrow: &'s oxc::ast::ast::ArrowFunctionExpression<'s>,
        path: &str,
        fallback_reason: &mut Option<String>,
    ) -> Option<Vec<PendingNode>> {
        let index_formulas = self.prepared_index_formulas(source, items.len());
        let parameter = arrow
            .params
            .items
            .first()?
            .pattern
            .get_identifier_name()?
            .to_string();
        let element = match arrow.get_expression().map(peel_expr)? {
            Expression::JSXElement(element) => element,
            _ => return None,
        };
        let tag = match &element.opening_element.name {
            JSXElementName::Identifier(id) => id.name.as_str(),
            JSXElementName::IdentifierReference(id) => id.name.as_str(),
            _ => return None,
        };
        let is_component = self.authored_fn(tag).is_some();
        let is_view = tag == "View" && !is_component;
        let is_group = tag == "Group" && !is_component;
        let is_text = tag == "Text" && !is_component;
        let is_circle = tag == "Circle" && !is_component;
        let is_path = tag == "Path" && !is_component;
        if !(is_view || is_group || is_text || is_circle || is_path || is_component)
            || !element.children.is_empty() && !(is_view || is_group || is_text || is_component)
        {
            return None;
        }
        // Exact Circle arcs currently emit one path per row, bounded by the ordinary map budget.
        if is_circle && items.len() > MAX_STATIC_MAP_ITEMS {
            return None;
        }
        let mut key_field = None;
        let mut has_absolute_class = false;
        let mut has_style = false;
        let mut has_circle_radius = false;
        let mut has_circle_fill = false;
        let mut has_path_data = false;
        let mut has_path_fill = false;
        let mut has_path_stroke = false;
        let mut has_path_stroke_width = false;
        let mut path_fill_source = None;
        let mut path_stroke_source = None;
        let mut style_names = BTreeSet::new();
        for attribute in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                return None;
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                return None;
            };
            if is_component && name.name != "key" {
                continue;
            }
            match name.name.as_str() {
                "key" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value
                    else {
                        return None;
                    };
                    let Some(Expression::StaticMemberExpression(member)) =
                        container.expression.as_expression().map(peel_expr)
                    else {
                        return None;
                    };
                    if !matches!(&member.object, Expression::Identifier(id) if id.name.as_str() == parameter)
                    {
                        return None;
                    }
                    key_field = Some(member.property.name.to_string());
                }
                "className" => {
                    if !(is_view || is_group || is_text) {
                        return None;
                    }
                    match &attribute.value {
                        Some(JSXAttributeValue::StringLiteral(value)) => {
                            has_absolute_class = value.value == "absolute";
                            if !has_absolute_class
                                && value
                                    .value
                                    .split_whitespace()
                                    .any(|class| class == "absolute")
                            {
                                return None;
                            }
                        }
                        Some(JSXAttributeValue::ExpressionContainer(_)) => {}
                        _ => return None,
                    }
                }
                "style" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value
                    else {
                        return None;
                    };
                    let Some(Expression::ObjectExpression(object)) =
                        container.expression.as_expression().map(peel_expr)
                    else {
                        return None;
                    };
                    has_style = true;
                    for property in &object.properties {
                        let ObjectPropertyKind::ObjectProperty(property) = property else {
                            return None;
                        };
                        let name = match &property.key {
                            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                            PropertyKey::StringLiteral(id) => id.value.as_str(),
                            _ => return None,
                        };
                        if !(if is_circle {
                            name == "opacity"
                        } else if is_path {
                            matches!(
                                name,
                                "translate"
                                    | "opacity"
                                    | "rotate"
                                    | "scale"
                                    | "transform"
                                    | "transformOrigin"
                            )
                        } else if is_text {
                            matches!(
                                name,
                                "width"
                                    | "height"
                                    | "fontSize"
                                    | "fontWeight"
                                    | "letterSpacing"
                                    | "lineHeight"
                                    | "color"
                                    | "opacity"
                                    | "textAlign"
                                    | "whiteSpace"
                            )
                        } else {
                            matches!(
                                name,
                                "left"
                                    | "top"
                                    | "width"
                                    | "height"
                                    | "backgroundColor"
                                    | "opacity"
                                    | "transform"
                                    | "minWidth"
                                    | "minHeight"
                                    | "maxWidth"
                                    | "maxHeight"
                                    | "marginTop"
                                    | "marginRight"
                                    | "marginBottom"
                                    | "marginLeft"
                                    | "paddingTop"
                                    | "paddingRight"
                                    | "paddingBottom"
                                    | "paddingLeft"
                                    | "borderRadius"
                            )
                        }) {
                            return None;
                        }
                        style_names.insert(name.to_owned());
                        if name == "transform"
                            && !matches!(peel_expr(&property.value), Expression::TemplateLiteral(template)
                                if template.quasis.len() == 2
                                    && template.expressions.len() == 1
                                    && template.quasis[0].value.raw == (if is_path { "skewX(" } else { "rotate(" })
                                    && template.quasis[1].value.raw == "deg)")
                            && !(is_path
                                && matches!(
                                    peel_expr(&property.value),
                                    Expression::StringLiteral(_)
                                ))
                        {
                            return None;
                        }
                    }
                }
                "visible" => {
                    if !matches!(
                        &attribute.value,
                        Some(JSXAttributeValue::ExpressionContainer(_))
                    ) {
                        return None;
                    }
                }
                "cx" | "cy" | "r" | "fill" if is_circle => {
                    if attribute.value.is_none() {
                        return None;
                    }
                    has_circle_radius |= name.name == "r";
                    has_circle_fill |= name.name == "fill";
                }
                "d" | "fill" | "stroke" | "strokeWidth" | "strokeLinecap" | "strokeLineCap"
                | "strokeCap" | "strokeLinejoin" | "strokeLineJoin" | "strokeJoin"
                | "strokeDasharray" | "strokeDash" | "strokeDashoffset" | "strokeDashOffset"
                | "strokeMiterlimit" | "strokeMiterLimit"
                    if is_path =>
                {
                    let value = attribute.value.as_ref()?;
                    has_path_data |= name.name == "d";
                    has_path_fill |= name.name == "fill";
                    has_path_stroke |= name.name == "stroke";
                    has_path_stroke_width |= name.name == "strokeWidth";
                    if matches!(name.name.as_str(), "fill" | "stroke") {
                        let span = value.span();
                        let authored = self.source.get(span.start as usize..span.end as usize)?;
                        if name.name == "fill" {
                            path_fill_source = Some(authored);
                        } else {
                            path_stroke_source = Some(authored);
                        }
                    }
                }
                _ => return None,
            }
        }
        let key_field = key_field?;
        let layout_view = is_component || is_group || is_text || is_view && !has_absolute_class;
        if !element.children.is_empty() && !layout_view {
            return None;
        }
        if layout_view
            && style_names
                .iter()
                .any(|name| matches!(name.as_str(), "left" | "top" | "transform"))
        {
            return None;
        }
        if is_view
            && !layout_view
            && style_names.iter().any(|name| {
                !matches!(
                    name.as_str(),
                    "left"
                        | "top"
                        | "width"
                        | "height"
                        | "backgroundColor"
                        | "opacity"
                        | "transform"
                )
            })
        {
            return None;
        }
        if is_path && has_path_stroke_width && !has_path_stroke {
            *fallback_reason = Some("Path strokeWidth needs a stroke".into());
            return None;
        }
        if items.is_empty()
            || is_view
                && (!layout_view && (!has_style || !has_absolute_class)
                    || !["left", "top", "width", "height", "backgroundColor"]
                        .iter()
                        .all(|name| style_names.contains(*name))
                        && !layout_view)
            || is_circle && (!has_circle_radius || !has_circle_fill)
            || is_path
                && (!has_path_data
                    || !has_path_fill
                    || !has_style
                    || !style_names.contains("translate"))
        {
            return None;
        }
        if self.geometry_references.unknown_key
            || items.iter().any(|item| {
                item.as_object()
                    .and_then(|object| object.get(&key_field))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|key| {
                        self.geometry_references.keys.contains(key)
                            || self
                                .geometry_references
                                .keys
                                .contains(&self.scoped_key(key))
                    })
            })
        {
            if !layout_view {
                *fallback_reason =
                    Some("bounds()/anchor() reads an instance row's layout box".into());
                return None;
            }
        }
        let first = items.first()?.as_object()?;
        #[cfg(not(target_family = "wasm"))]
        let instance_data_started = std::time::Instant::now();
        // Only scalar or typed geometry fields can become instance columns. Prepare-time
        // objects may carry metadata that the template never reads; those fields must not
        // force an otherwise homogeneous component back to per-row expansion.
        let mut columns = first
            .iter()
            .filter_map(|(name, value)| {
                motion_value_from_json(value)
                    .filter(MotionValue::is_finite)
                    .map(|value| (name.clone(), vec![value]))
            })
            .collect::<Vec<_>>();
        columns.sort_by(|left, right| left.0.cmp(&right.0));
        let mut keys = Vec::with_capacity(items.len());
        let mut seen_keys = BTreeSet::new();
        for (index, item) in items.iter().enumerate() {
            let object = item.as_object()?;
            let key = object.get(&key_field)?.as_str()?;
            let scoped_key = self.scoped_key(key);
            if !seen_keys.insert(scoped_key.clone()) {
                *fallback_reason = Some("duplicate instance row key".into());
                return None;
            }
            keys.push(scoped_key);
            if index > 0 {
                for (name, values) in &mut columns {
                    if values.len() != index {
                        continue;
                    }
                    let value = object
                        .get(name)
                        .and_then(motion_value_from_json)
                        .filter(MotionValue::is_finite)
                        .filter(|value| {
                            valle_motion::expr::ExprType::of_value(&values[0])
                                == valle_motion::expr::ExprType::of_value(value)
                        });
                    if let Some(value) = value {
                        values.push(value);
                    }
                }
            }
        }
        columns.retain(|(_, values)| values.len() == items.len());
        let fields = columns
            .iter()
            .enumerate()
            .map(|(index, (name, values))| {
                (
                    name.clone(),
                    (
                        index as u32,
                        valle_motion::expr::ExprType::of_value(&values[0]),
                    ),
                )
            })
            .collect();
        #[cfg(not(target_family = "wasm"))]
        let instance_data_elapsed = instance_data_started.elapsed();
        let template_key = self.scoped_key(&format!("{path}.__template__"));
        let saved = self.snapshot_instance_attempt();
        // Lower in the outer arena so captured values retain their identity, then extract
        // only the template's reachable closure before rolling back temporary expressions.
        let prefix_len = self.expr_arena.values.len();
        let saved_bindings = self.bindings.clone();
        let saved_scope = self.instance_scope.replace(InstanceCompileScope {
            parameter,
            key_field,
            template_key: template_key.clone(),
            fields,
        });
        let index_binding = arrow
            .params
            .items
            .get(1)
            .and_then(|parameter| parameter.pattern.get_identifier_name())
            .map(|name| name.to_string());
        if let Some(name) = index_binding {
            let id = self.push(Expr::InstanceIndex, arrow.span());
            self.bindings.scalars.insert(name, id);
        }
        self.list_depth += 1;
        #[cfg(not(target_family = "wasm"))]
        let template_started = std::time::Instant::now();
        let template = self.lower_jsx(element, path);
        #[cfg(not(target_family = "wasm"))]
        let template_elapsed = template_started.elapsed();
        self.list_depth -= 1;
        let mut template = template;
        let group_arena = template
            .as_mut()
            .and_then(|template| instance_capture::extract(&self.expr_arena, template));
        self.expr_arena.truncate(prefix_len);
        self.instance_scope = saved_scope;
        self.bindings = saved_bindings;
        let Some(mut template) = template else {
            self.restore_instance_attempt(saved);
            *fallback_reason =
                Some("template could not be lowered with instance row values".into());
            return None;
        };
        if self.diagnostics.len() > saved.diagnostics_len {
            self.restore_instance_attempt(saved);
            *fallback_reason =
                Some("template needs per-row lowering for its expressions or topology".into());
            return None;
        }
        let Some(group_arena) = group_arena else {
            self.restore_instance_attempt(saved);
            *fallback_reason = Some("template contains unsupported node fields".into());
            return None;
        };
        if is_path
            && has_path_stroke
            && path_fill_source == path_stroke_source
            && let NodeKind::Path {
                fill: Some(fill),
                stroke: Some(stroke),
                ..
            } = &mut template.kind
        {
            // The author wrote the same pure paint expression twice. Reuse the fill binding
            // so the artifact itself proves the batch's single color paints both passes.
            stroke.paint = fill.clone();
        }
        let Some(root_key_suffix) = template.key.strip_prefix(&template_key) else {
            self.restore_instance_attempt(saved);
            *fallback_reason = Some("component root key is outside the instance key scope".into());
            return None;
        };
        if !root_key_suffix.is_empty()
            && !root_key_suffix.starts_with('/')
            && !root_key_suffix.starts_with('.')
        {
            self.restore_instance_attempt(saved);
            *fallback_reason = Some("component root key is outside the instance key scope".into());
            return None;
        }
        for key in &mut keys {
            key.push_str(root_key_suffix);
        }
        let text_root_supported = match &template.kind {
            NodeKind::Text {
                text,
                per_unit: None,
                path: None,
            } if template.children.is_empty() => match text {
                TextValue::Static { .. } => true,
                TextValue::Expr { expr } => {
                    group_arena.types.get(expr.0 as usize).copied().flatten()
                        == Some(valle_motion::expr::ExprType::String)
                }
            },
            _ => false,
        };
        let layout_root_supported = !layout_view
            || (matches!(template.kind, NodeKind::Box | NodeKind::Group) || text_root_supported)
                && template.space.is_none()
                && template.semantic.is_none()
                && !template.is_mask_source
                && (text_root_supported
                    || template.styles.iter().all(|style| {
                        matches!(
                            style.property.as_str(),
                            "width"
                                | "height"
                                | "min-width"
                                | "min-height"
                                | "max-width"
                                | "max-height"
                                | "margin-top"
                                | "margin-right"
                                | "margin-bottom"
                                | "margin-left"
                                | "padding-top"
                                | "padding-right"
                                | "padding-bottom"
                                | "padding-left"
                                | "border-radius"
                                | "background-color"
                                | "opacity"
                        )
                    }));
        let template_types_supported = template.styles.iter().all(|style| {
            if text_root_supported {
                return pending_instance_style_supported(style, &group_arena.types, true);
            }
            use valle_motion::expr::ExprType;
            match &style.value {
                StyleValue::Static { value } => match style.property.as_str() {
                    "left" | "top" | "width" | "height" | "min-width" | "min-height"
                    | "max-width" | "max-height" | "margin-top" | "margin-right"
                    | "margin-bottom" | "margin-left" | "padding-top" | "padding-right"
                    | "padding-bottom" | "padding-left" | "border-radius" => {
                        matches!(value,
                        MotionValue::Number(number) if number.is_finite())
                            || matches!(value, MotionValue::Length(length)
                            if length.unit == valle_motion::value::LengthUnit::Px
                                && length.value.is_finite())
                    }
                    "opacity" => matches!(value, MotionValue::Number(number)
                        if number.is_finite() && (0.0..=1.0).contains(number)),
                    "background-color" => {
                        matches!(value, MotionValue::Color(_) | MotionValue::Str(_))
                    }
                    "transform" if is_path => matches!(value, MotionValue::Str(value)
                        if valle_motion::artifact::path_instance_skew_x(value).is_some()),
                    "transform" => matches!(value, MotionValue::Str(_)),
                    "rotate" => match value {
                        MotionValue::Angle(angle) => (angle.as_degrees() as f32).is_finite(),
                        MotionValue::Str(value) => valle_motion::value::Angle::parse(value)
                            .is_some_and(|angle| (angle.as_degrees() as f32).is_finite()),
                        _ => false,
                    },
                    "scale" => match value {
                        MotionValue::Number(value) => *value != 0.0 && (*value as f32).is_finite(),
                        MotionValue::Point(point) => {
                            point.x != 0.0
                                && point.y != 0.0
                                && (point.x as f32).is_finite()
                                && (point.y as f32).is_finite()
                        }
                        _ => false,
                    },
                    "transform-origin" => {
                        matches!(value,
                        MotionValue::Point(point) if point.x == 0.0 && point.y == 0.0)
                            || matches!(value, MotionValue::Length2(lengths)
                            if lengths.x.unit == valle_motion::value::LengthUnit::Px
                                && lengths.y.unit == valle_motion::value::LengthUnit::Px
                                && lengths.x.value == 0.0 && lengths.y.value == 0.0)
                    }
                    "translate" => match value {
                        MotionValue::Point(point) => point.x.is_finite() && point.y.is_finite(),
                        MotionValue::Vec2(vector) => vector.x.is_finite() && vector.y.is_finite(),
                        MotionValue::Length2(lengths) => {
                            lengths.x.unit == valle_motion::value::LengthUnit::Px
                                && lengths.y.unit == valle_motion::value::LengthUnit::Px
                                && lengths.x.value.is_finite()
                                && lengths.y.value.is_finite()
                        }
                        _ => false,
                    },
                    _ => false,
                },
                StyleValue::Expr { expr } => match (
                    style.property.as_str(),
                    group_arena.types.get(expr.0 as usize).copied().flatten(),
                ) {
                    (
                        "left" | "top" | "width" | "height" | "opacity" | "min-width"
                        | "min-height" | "max-width" | "max-height" | "margin-top" | "margin-right"
                        | "margin-bottom" | "margin-left" | "padding-top" | "padding-right"
                        | "padding-bottom" | "padding-left" | "border-radius",
                        Some(ExprType::Number),
                    )
                    | ("background-color", Some(ExprType::Color | ExprType::String))
                    | ("transform", Some(ExprType::String))
                    | ("rotate", Some(ExprType::Angle | ExprType::String))
                    | ("scale", Some(ExprType::Number | ExprType::Point))
                    | ("translate", Some(ExprType::Point | ExprType::Vec2)) => true,
                    _ => false,
                },
            }
        });
        let template_inputs_supported = group_arena.values.iter().all(|expr| {
            !matches!(expr, Expr::NodeBounds { .. } | Expr::Project3D { .. })
                && !matches!(expr, Expr::Context { input } if input.is_unit())
        });
        let template_visibility_supported = template.visibility.is_none_or(|id| {
            group_arena.types.get(id.0 as usize).copied().flatten()
                == Some(valle_motion::expr::ExprType::Bool)
        });
        let template_children_supported = if layout_view {
            pending_instance_children_supported(
                &template_key,
                &template.children,
                &group_arena.types,
            )
        } else {
            template.children.is_empty()
        };
        if !layout_root_supported
            || !template_types_supported
            || !template_inputs_supported
            || !template_visibility_supported
            || !template_children_supported
        {
            self.restore_instance_attempt(saved);
            *fallback_reason = Some(if !layout_root_supported {
                "layout instance root must be a fixed View/Group or plain Text with supported styles"
                    .into()
            } else if !template_types_supported {
                format!("{tag} has a style value type unsupported by its instance renderer")
            } else if !template_inputs_supported {
                "template reads per-node layout, 3D, or text-unit context".into()
            } else if !template_children_supported {
                "template children need a fixed Box/Text tree with supported inline styles and row-scoped keys".into()
            } else {
                "visible must produce a boolean for every row".into()
            });
            return None;
        }
        // The key already has its own column. Keep only item fields read by the compiled
        // template, and rewrite their compact column ids after expression lowering.
        let mut exprs = group_arena.values;
        let used_columns = exprs
            .iter()
            .filter_map(|expr| match expr {
                Expr::InstanceField { column, .. } => Some(*column),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let remap = used_columns
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new as u32))
            .collect::<BTreeMap<_, _>>();
        #[cfg(not(target_family = "wasm"))]
        let column_pack_started = std::time::Instant::now();
        let columns = columns
            .into_iter()
            .enumerate()
            .filter_map(|(old, (name, values))| {
                if !used_columns.contains(&(old as u32)) {
                    return None;
                }
                let mut values =
                    InstanceColumnValues::from_values(values).expect("validated instance column");
                if let (InstanceColumnValues::Numbers(numbers), Some(expression)) =
                    (&values, index_formulas.get(&name))
                    && numbers.iter().enumerate().all(|(index, value)| {
                        expression.evaluate(index).to_bits() == value.to_bits()
                    })
                {
                    values = InstanceColumnValues::Formula {
                        rows: numbers.len() as u32,
                        expression: expression.clone(),
                    };
                }
                Some(InstanceColumn { name, values })
            })
            .collect();
        #[cfg(not(target_family = "wasm"))]
        metrics::record_instance_data(column_pack_started.elapsed());
        for expr in &mut exprs {
            if let Expr::InstanceField { column, .. } = expr {
                *column = remap[column];
            }
        }
        let template_children = template
            .children
            .into_iter()
            .map(pending_instance_template_node)
            .collect();
        let instance_group = InstanceGroup {
            template: SceneNode {
                key: template.key,
                kind: template.kind,
                space: template.space,
                class_names: template.class_names,
                class_conditions: template.class_conditions,
                styles: template.styles,
                visibility: template.visibility,
                children: ChildRange::EMPTY,
                semantic: template.semantic,
            },
            template_key_prefix: template_key,
            template_children,
            keys: InstanceKeys::from_values(keys),
            columns,
            exprs,
        };
        if is_circle && instance_group.circle_template_parameters().is_none() {
            self.restore_instance_attempt(saved);
            *fallback_reason =
                Some("Circle needs a full solid arc without stroke or trimming".into());
            return None;
        }
        if is_path && instance_group.static_path_template().is_none() {
            self.restore_instance_attempt(saved);
            *fallback_reason = Some(if has_path_stroke {
                "Path needs static geometry, pixel translate, and same-color non-negative stroke with default cap/join".into()
            } else {
                "Path needs one nonempty static geometry, solid fill, and pixel translate".into()
            });
            return None;
        }
        #[cfg(not(target_family = "wasm"))]
        {
            metrics::record_instance_data(instance_data_elapsed);
            metrics::record_template_compile(template_elapsed);
        }
        let group = self.instance_groups.len() as u32;
        self.source_ledger
            .instance_exprs
            .push((group_arena.spans, group_arena.expansion_stacks));
        self.instance_groups.push(instance_group);
        self.extra_capabilities
            .insert(GEOMETRY_BATCH_CAPABILITY.to_owned());
        let zero = || StyleValue::Static {
            value: MotionValue::Number(0.0),
        };
        let full = || StyleValue::Static {
            value: MotionValue::Length(valle_motion::value::Length::parse("100%").unwrap()),
        };
        Some(vec![PendingNode {
            span: element.span(),
            expansion_stack: self.component_stack.clone(),
            key: self.scoped_key(&format!("{path}.__instances__")),
            kind: if layout_view {
                NodeKind::InstanceLayout { group }
            } else {
                NodeKind::InstanceBatch { group }
            },
            space: None,
            class_names: if layout_view {
                Vec::new()
            } else {
                vec!["absolute".into()]
            },
            class_conditions: BTreeMap::new(),
            styles: if layout_view {
                Vec::new()
            } else {
                [
                    StyleBinding {
                        property: "left".into(),
                        value: zero(),
                    },
                    StyleBinding {
                        property: "top".into(),
                        value: zero(),
                    },
                    StyleBinding {
                        property: "width".into(),
                        value: full(),
                    },
                    StyleBinding {
                        property: "height".into(),
                        value: full(),
                    },
                ]
                .into()
            },
            visibility: None,
            semantic: None,
            children: Vec::new(),
            is_mask_source: false,
        }])
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
        let mut scoped_instance_key = None;
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
                if let Some(scope) = self.instance_scope.as_ref()
                    && matches!(&attribute.value,
                        Some(JSXAttributeValue::ExpressionContainer(container))
                        if matches!(container.expression.as_expression().map(peel_expr),
                            Some(Expression::StaticMemberExpression(member))
                            if matches!(&member.object, Expression::Identifier(id)
                                if id.name.as_str() == scope.parameter)
                                && member.property.name.as_str() == scope.key_field))
                {
                    scoped_instance_key = Some(scope.template_key.clone());
                } else {
                    instance_key =
                        self.attr_static_string(&attribute.value, attribute.span(), "key");
                }
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
        let instance_key = instance_key.unwrap_or_else(|| path.to_string());
        let caller_prefix = self.key_prefix.clone();
        let component_prefix =
            scoped_instance_key.unwrap_or_else(|| self.scoped_key(&instance_key));
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
        if params.rest.is_some() || params.items.len() > 2 {
            self.illegal(
                DiagCode::ModuleShape,
                params.span(),
                format!("component `{name}` must use `(ctx, props)` parameters"),
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
                Some(AuthorValue::Paint(paint)) => {
                    self.bind_paint(local_name, paint);
                }
                Some(AuthorValue::DynamicTuple(values)) => {
                    self.bind_dynamic_tuple(local_name, values);
                }
                Some(AuthorValue::InstanceObject(fields)) => {
                    self.bind_instance_object(local_name, fields);
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

fn pending_instance_children_supported(
    scope_key: &str,
    children: &[PendingNode],
    types: &[Option<valle_motion::expr::ExprType>],
) -> bool {
    fn check(
        scope_key: &str,
        node: &PendingNode,
        types: &[Option<valle_motion::expr::ExprType>],
        depth: usize,
        count: &mut usize,
    ) -> bool {
        *count += 1;
        if depth > 32
            || *count > 1024
            || !node
                .key
                .strip_prefix(scope_key)
                .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with('/'))
            || node.space.is_some()
            || node.class_conditions.iter().any(|(class, id)| {
                !node.class_names.contains(class)
                    || types.get(id.0 as usize).copied().flatten()
                        != Some(valle_motion::expr::ExprType::Bool)
            })
            || node.semantic.is_some()
            || node.is_mask_source
            || node.visibility.is_some_and(|id| {
                types.get(id.0 as usize).copied().flatten()
                    != Some(valle_motion::expr::ExprType::Bool)
            })
        {
            return false;
        }
        let is_text = match &node.kind {
            NodeKind::Box | NodeKind::Group => false,
            NodeKind::Text {
                text,
                per_unit: None,
                path: None,
            } if node.children.is_empty() => {
                if let TextValue::Expr { expr } = text
                    && types.get(expr.0 as usize).copied().flatten()
                        != Some(valle_motion::expr::ExprType::String)
                {
                    return false;
                }
                true
            }
            _ => return false,
        };
        node.styles
            .iter()
            .all(|style| pending_instance_style_supported(style, types, is_text))
            && node
                .children
                .iter()
                .all(|child| check(scope_key, child, types, depth + 1, count))
    }
    let mut count = 0;
    children
        .iter()
        .all(|child| check(scope_key, child, types, 1, &mut count))
}

fn pending_instance_style_supported(
    style: &StyleBinding,
    types: &[Option<valle_motion::expr::ExprType>],
    is_text: bool,
) -> bool {
    use valle_motion::expr::ExprType;
    let property = style.property.as_str();
    let value_type = match &style.value {
        StyleValue::Static { value } if value.is_finite() => ExprType::of_value(value),
        StyleValue::Static { .. } => return false,
        StyleValue::Expr { expr } => match types.get(expr.0 as usize).copied().flatten() {
            Some(value_type) => value_type,
            None => return false,
        },
    };
    let pixel_length = value_type == ExprType::Number
        || matches!(&style.value,
        StyleValue::Static { value: MotionValue::Length(length) }
            if length.unit == valle_motion::value::LengthUnit::Px && length.value.is_finite());
    match property {
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height"
        | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" | "border-radius" | "gap"
        | "left" | "top" | "right" | "bottom" => pixel_length,
        "font-size" | "letter-spacing" | "line-height" if is_text => pixel_length,
        "background-color" if !is_text => matches!(value_type, ExprType::Color | ExprType::String),
        "color" if is_text => matches!(value_type, ExprType::Color | ExprType::String),
        "opacity" => value_type == ExprType::Number,
        "font-weight" if is_text => matches!(value_type, ExprType::Number | ExprType::String),
        "text-align" | "white-space" if is_text => value_type == ExprType::String,
        "display" | "position" | "flex-direction" | "justify-content" | "align-items"
        | "flex-wrap"
            if !is_text =>
        {
            value_type == ExprType::String
        }
        _ => false,
    }
}

fn pending_instance_template_node(node: PendingNode) -> InstanceTemplateNode {
    InstanceTemplateNode {
        node: SceneNode {
            key: node.key,
            kind: node.kind,
            space: node.space,
            class_names: node.class_names,
            class_conditions: node.class_conditions,
            styles: node.styles,
            visibility: node.visibility,
            children: ChildRange::EMPTY,
            semantic: node.semantic,
        },
        children: node
            .children
            .into_iter()
            .map(pending_instance_template_node)
            .collect(),
    }
}
