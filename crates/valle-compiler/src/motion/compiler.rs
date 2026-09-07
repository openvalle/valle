//! Compiler construction, root orchestration, output arenas, and diagnostics.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn new(
        source: &'s str,
        program: &'s Program<'s>,
        resources: &[ResourceRef],
        measure: Option<&MeasureEnv>,
        shader_registry: Option<&ShaderRegistryEnv>,
        prepare_data: Option<&PrepareDataBinding>,
    ) -> Result<Self, Vec<CompilerDiagnostic>> {
        if let Some(span) = module_reserved_theme_binding(program) {
            return Err(vec![diagnostic_at(
                source,
                DiagCode::GrammarForbidden,
                span,
                "`ThemeProvider` and `useTheme` are reserved compile-time Motion intrinsics; do not declare or import replacements",
            )]);
        }
        if let Some(diagnostic) = scan_forbidden(source, "module") {
            let span = forbidden_span(source, &diagnostic.message);
            return Err(vec![from_motion_diagnostic(source, diagnostic, span)]);
        }
        if let Some(span) = scan_exponentiation(program) {
            return Err(vec![diagnostic_at(
                source,
                DiagCode::GrammarForbidden,
                span,
                "`**` is implementation-approximated in ECMAScript and cannot be intercepted by \
                 Valle's deterministic math bridge. Use `Math.pow(base, exponent)` instead",
            )]);
        }
        if let Some((span, message)) = scan_sequence_labels(program) {
            return Err(vec![diagnostic_at(
                source,
                DiagCode::GrammarForbidden,
                span,
                message,
            )]);
        }
        // Reject missing measurement fonts before creating the sandbox to provide an actionable
        // diagnostic.
        if measure.is_none() && crate::motion_sandbox::mentions_measure(source) {
            return Err(vec![from_motion_diagnostic(
                source,
                MotionDiagnostic::new(
                    DiagCode::StaticEvalFailed,
                    "module",
                    "this source measures text at compile time, which requires a font bundle; \
                     pass `--font <path>` so measurement uses the same fonts as rendering",
                ),
                Span::new(0, source.len() as u32),
            )]);
        }

        let mut prelude = String::from(STATIC_HELPERS);
        let mut module_const_inits = BTreeMap::new();
        for statement in &program.body {
            record_module_const_inits(statement, &mut module_const_inits);
            let is_prepare_declaration = match statement {
                // Keep JSX declarations out of QuickJS; node helpers are expanded by the lowerer.
                Statement::VariableDeclaration(_) => !contains_jsx(statement),
                Statement::FunctionDeclaration(function) => {
                    function
                        .id
                        .as_ref()
                        .is_some_and(|id| !is_component_name(id.name.as_str()))
                        && !contains_jsx(statement)
                }
                _ => false,
            };
            if is_prepare_declaration {
                // Strip TypeScript-only wrappers before evaluating module constants in QuickJS.
                if let Statement::VariableDeclaration(declaration) = statement
                    && let Some(text) = prelude_peeled_const_text(source, declaration)
                {
                    prelude.push_str(&text);
                    prelude.push('\n');
                    continue;
                }
                prelude.push_str(
                    &source[statement.span().start as usize..statement.span().end as usize],
                );
                prelude.push('\n');
            }
        }
        let sandbox = Sandbox::new(&prelude, measure).map_err(|diagnostic| {
            vec![from_motion_diagnostic(
                source,
                diagnostic,
                Span::new(0, source.len() as u32),
            )]
        })?;
        // Register both function declarations and arrow-function helpers.
        let mut functions: BTreeMap<String, AuthoredFn<'s>> = BTreeMap::new();
        for statement in &program.body {
            match statement {
                Statement::FunctionDeclaration(function) => {
                    if let Some(id) = function.id.as_ref() {
                        functions
                            .insert(id.name.to_string(), AuthoredFn::Declared(function.as_ref()));
                    }
                }
                Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                    for declarator in &declaration.declarations {
                        if let Some(name) = declarator.id.get_identifier_name()
                            && let Some(initializer) = &declarator.init
                            && let Some(authored) = authored_fn_of(initializer)
                        {
                            functions.insert(name.to_string(), authored);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            source,
            sandbox,
            camera: None,
            extra_capabilities: BTreeSet::new(),
            has_measure: measure.is_some(),
            shader_registry: shader_registry.cloned(),
            component: None,
            controls: default_controls(),
            controls_span: None,
            prepare_data: prepare_data.cloned(),
            default_function: None,
            functions,
            expr_arena: ExprArena::default(),
            source_ledger: SourceLedger::default(),
            resource_refs: resources.to_vec(),
            used_font_controls: BTreeSet::new(),
            bindings: Bindings::default(),
            module_const_inits,
            current_theme: None,
            key_prefix: String::new(),
            component_stack: Vec::new(),
            helper_stack: Vec::new(),
            list_depth: 0,
            expanded_list_items: 0,
            expanded_helper_calls: 0,
            keys: BTreeSet::new(),
            layout_ids: BTreeSet::new(),
            diagnostics: Vec::new(),
        })
    }

    pub(super) fn compile(
        &mut self,
        program: &'s Program<'s>,
    ) -> Result<SceneArtifact, Vec<CompilerDiagnostic>> {
        for statement in &program.body {
            match statement {
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                        for declarator in &declaration.declarations {
                            let Some(name) = declarator.id.get_identifier_name() else {
                                self.illegal(DiagCode::ModuleShape, declarator.span(), "exported controls use simple identifiers");
                                continue;
                            };
                            let Some(initializer) = &declarator.init else {
                                self.illegal(DiagCode::ModuleShape, declarator.span(), "an exported const needs an initializer");
                                continue;
                            };
                            match name.as_str() {
                                "component" => match initializer {
                                    Expression::StringLiteral(value) => self.component = Some(value.value.to_string()),
                                    _ => self.illegal(DiagCode::ModuleShape, initializer.span(), "component must be a string literal"),
                                },
                                "controls" => self.compile_controls(initializer),
                                other => self.unsupported(initializer.span(), format!("export `{other}` is a valid future authoring surface but the current module contract only admits `component` and `controls`")),
                            }
                        }
                    }
                    _ => self.illegal(DiagCode::ModuleShape, export.span(), "named exports must be `export const component` or `export const controls`"),
                },
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        self.default_function = Some(function.as_ref());
                        if self.component.is_none() {
                            self.component = function.id.as_ref().map(|id| id.name.to_string());
                        }
                        self.validate_root_params(&function.params);
                    }
                    _ => self.illegal(DiagCode::ModuleShape, export.span(), "default export must be a named function declaration"),
                },
                Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {}
                Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {}
                other => self.illegal(DiagCode::GrammarForbidden, other.span(), "module scope only allows static const/function declarations and the component exports"),
            }
        }

        self.validate_prepare_data();

        let Some(component) = self.component.clone() else {
            self.illegal(
                DiagCode::ModuleShape,
                Span::new(0, 0),
                "give the default export a function name or export `component`",
            );
            return Err(std::mem::take(&mut self.diagnostics));
        };
        let Some(function) = self.default_function else {
            self.illegal(
                DiagCode::ModuleShape,
                Span::new(0, 0),
                "missing default component function",
            );
            return Err(std::mem::take(&mut self.diagnostics));
        };
        let Some(body) = function.body.as_deref() else {
            self.illegal(
                DiagCode::ModuleShape,
                function.span(),
                "default component needs a function body",
            );
            return Err(std::mem::take(&mut self.diagnostics));
        };

        self.bind_root_params(&function.params);
        let authored_root = self.compile_body(body);
        if !self.diagnostics.is_empty() {
            return Err(std::mem::take(&mut self.diagnostics));
        }
        let Some(authored_root) = authored_root else {
            self.illegal(
                DiagCode::ModuleShape,
                body.span(),
                "component must return one JSX element",
            );
            return Err(std::mem::take(&mut self.diagnostics));
        };
        let authored_root_span = authored_root.span;
        let authored_root_stack = authored_root.expansion_stack.clone();
        let mut root = if matches!(authored_root.kind, NodeKind::Group) {
            authored_root
        } else {
            PendingNode {
                space: None,
                span: authored_root_span,
                expansion_stack: authored_root_stack,
                key: "__scene_root__".into(),
                kind: NodeKind::Group,
                class_names: Vec::new(),
                styles: Vec::new(),
                visibility: None,
                semantic: None,
                children: vec![authored_root],
                is_mask_source: false,
            }
        };
        self.normalize_glass_scopes(&mut root, None);
        if !self.diagnostics.is_empty() {
            return Err(std::mem::take(&mut self.diagnostics));
        }
        let mut nodes = Vec::new();
        let mut node_children = Vec::new();
        let root = emit_node(
            root,
            &mut nodes,
            &mut node_children,
            &mut self.source_ledger.node_spans,
            &mut self.source_ledger.node_expansion_stacks,
        );
        let resource_refs = std::mem::take(&mut self.resource_refs)
            .into_iter()
            .filter(|resource| {
                self.controls
                    .assets
                    .get(&resource.control)
                    .is_none_or(|control| {
                        control.kind != AssetKind::Font
                            || self.used_font_controls.contains(&resource.control)
                    })
            })
            .collect();
        // G4.7: artifacts that use Motion Glass declare the capability; the registry lists it,
        // and admission fail-closed rules (kernel/ABI digests) apply at execution.
        if nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Glass(_) | NodeKind::GlassField(_)))
        {
            self.extra_capabilities
                .insert(valle_motion::glass::MOTION_GLASS_CAPABILITY.to_string());
        }
        let artifact = SceneArtifact {
            camera: self.camera.clone(),
            format_version: ARTIFACT_FORMAT_VERSION,
            capability_set: CapabilitySet::new(
                BASE_CAPABILITIES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .chain(self.extra_capabilities.iter().cloned()),
            ),
            component,
            controls: self.controls.clone(),
            resource_refs,
            exprs: std::mem::take(&mut self.expr_arena.values),
            nodes,
            node_children,
            root,
        };
        let validation = match self.shader_registry.as_ref() {
            Some(registry) => artifact.validate_with_shaders(registry),
            None => artifact.validate(),
        };
        if let Err(errors) = validation {
            for error in errors {
                let span = validation_span(
                    &error.path,
                    &self.expr_arena.spans,
                    &self.source_ledger.node_spans,
                    self.controls_span,
                );
                self.illegal(DiagCode::ArtifactInvalid, span, error.to_string());
            }
            return Err(std::mem::take(&mut self.diagnostics));
        }
        Ok(artifact)
    }

    /// Normalize the semantic `<GlassField>` scope after JSX expansion, when the complete parent
    /// chain is available. Field membership is derived data: authors declare `fieldId` exactly
    /// once on `<GlassField>`, and every nearest descendant `<Glass>` inherits it through
    /// non-painting layout/transform wrappers. Independent defaults are expanded only after this
    /// classification, so field-only inheritance can never depend on a duplicated member prop.
    fn normalize_glass_scopes(
        &mut self,
        node: &mut PendingNode,
        inherited: Option<valle_motion::glass::GlassFieldId>,
    ) {
        let active = match &node.kind {
            NodeKind::GlassField(field) => {
                if let Some(parent) = inherited.as_ref() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        node.span,
                        format!(
                            "<GlassField fieldId=\"{}\"> is structurally nested inside field '{}'; nested fields must be placed inside a member foreground",
                            field.field_id.as_str(),
                            parent.as_str()
                        ),
                    );
                }
                Some(field.field_id.clone())
            }
            _ => inherited,
        };

        if let NodeKind::Glass(glass) = &mut node.kind {
            if glass.field_id.is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    node.span,
                    "<Glass> must not declare fieldId; membership is inherited from the nearest <GlassField>",
                );
            }
            match active.as_ref() {
                Some(field_id) => {
                    if glass.material.is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            node.span,
                            "a GlassField member inherits material and cannot override it",
                        );
                    }
                    if glass.environment.is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            node.span,
                            "a GlassField member inherits environment and cannot override it",
                        );
                    }
                    if glass.motion.character.is_some() || glass.motion.settle.is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            node.span,
                            "a GlassField member inherits motion.character/settle; only intensity and drive may be overridden",
                        );
                    }
                    glass.field_id = Some(field_id.clone());
                }
                None => {
                    glass.field_id = None;
                    glass
                        .material
                        .get_or_insert_with(valle_motion::glass::GlassMaterialBinding::defaults);
                    glass
                        .environment
                        .get_or_insert_with(valle_motion::glass::GlassEnvironmentBinding::defaults);
                    glass
                        .motion
                        .character
                        .get_or_insert(valle_motion::glass::GlassCharacter::DEFAULT);
                    glass
                        .motion
                        .settle
                        .get_or_insert(valle_motion::glass::DEFAULT_SETTLE_SECONDS);
                }
            }
        }

        // A Glass node consumes the active field for its own material shell. Its children are
        // foreground content and therefore begin a fresh material scope; this is what makes a
        // nested independent Glass/GlassField unambiguous.
        let child_scope = if matches!(&node.kind, NodeKind::Glass(_)) {
            None
        } else {
            active
        };
        for child in &mut node.children {
            self.normalize_glass_scopes(child, child_scope.clone());
        }
    }

    pub(super) fn compile_controls(&mut self, expression: &Expression<'_>) {
        let span = expression.span();
        self.controls_span = Some(span);
        match self
            .sandbox
            .eval_json(self.src(expression), &[], "controls")
        {
            Ok(value) => match controls_from_json(&value) {
                Ok(controls) => {
                    // Scene camera expressions are supported; reject camera controls that require
                    // unsupported target selection or fit behavior.
                    if !controls.camera.values.is_empty() {
                        self.unsupported(
                            named_control_span(self.source, span, "camera"),
                            "camera *controls* (target / fit knobs) are not implemented yet; \
                             the scene-level camera itself is available as `<Scene camera={...}>` \
                             with center / zoom / rotation",
                        );
                    }
                    self.controls = controls;
                }
                Err(message) => self.illegal(DiagCode::ModuleShape, span, message),
            },
            Err(diagnostic) => {
                self.diagnostics
                    .push(from_motion_diagnostic(self.source, diagnostic, span))
            }
        }
    }

    pub(super) fn validate_prepare_data(&mut self) {
        let Some(binding) = self.prepare_data.as_ref() else {
            if !self.controls.data.is_empty() {
                self.illegal(
                    DiagCode::ModuleShape,
                    self.controls_span.unwrap_or(Span::new(0, 0)),
                    "controls.data declares structured input but no data binding was supplied",
                );
            }
            return;
        };
        if binding.source.trim().is_empty() {
            self.illegal(
                DiagCode::ModuleShape,
                self.controls_span.unwrap_or(Span::new(0, 0)),
                "prepare data binding source must be non-empty so it can enter the fingerprint",
            );
            return;
        }
        let source_path = binding.source.clone();
        let value = binding.value.clone();
        if let Err(errors) = self.controls.validate_data(&value) {
            for error in errors {
                self.push_diagnostic(CompilerDiagnostic {
                    class: DiagClass::Illegal,
                    code: DiagCode::ModuleShape,
                    span: SourceSpan {
                        start: 0,
                        end: 0,
                        line: 1,
                        column: 1,
                    },
                    source_path: Some(source_path.clone()),
                    message: error.to_string(),
                });
            }
        }
    }

    pub(super) fn validate_root_params(&mut self, params: &FormalParameters<'_>) {
        if params.rest.is_some() || params.items.len() > 4 {
            self.illegal(
                DiagCode::ModuleShape,
                params.span(),
                "component parameters must be `(ctx, props, signals, data)`; props and data may be destructured in their positions",
            );
            return;
        }
        if let Some(first) = params.items.first()
            && first
                .pattern
                .get_identifier_name()
                .map(|name| name.as_str())
                != Some("ctx")
        {
            self.illegal(
                DiagCode::ModuleShape,
                first.span(),
                "the first component parameter must be `ctx`",
            );
        }
        if let Some(second) = params.items.get(1)
            && !matches!(second.pattern, BindingPattern::ObjectPattern(_))
            && second
                .pattern
                .get_identifier_name()
                .map(|name| name.as_str())
                != Some("props")
        {
            self.illegal(
                DiagCode::ModuleShape,
                second.span(),
                "the second component parameter must be `props` or an object destructure",
            );
        }
        if let Some(third) = params.items.get(2)
            && third
                .pattern
                .get_identifier_name()
                .map(|name| name.as_str())
                != Some("signals")
        {
            self.illegal(
                DiagCode::ModuleShape,
                third.span(),
                "the third component parameter must be `signals`",
            );
        }
        if let Some(fourth) = params.items.get(3)
            && !matches!(
                fourth.pattern,
                BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
            )
            && fourth
                .pattern
                .get_identifier_name()
                .map(|name| name.as_str())
                != Some("data")
        {
            self.illegal(
                DiagCode::ModuleShape,
                fourth.span(),
                "the fourth component parameter must be `data` or a data destructure",
            );
        }
    }

    pub(super) fn bind_root_params(&mut self, params: &FormalParameters<'_>) {
        if let Some(second) = params.items.get(1)
            && let BindingPattern::ObjectPattern(pattern) = &second.pattern
        {
            if pattern.rest.is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    pattern.span(),
                    "props rest destructuring is not deterministic because it hides the admitted schema",
                );
            }
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
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.value.span(),
                        "nested props destructuring is not supported; bind one scalar field at a time",
                    );
                    continue;
                };
                if !self.controls.props.contains_key(&prop_name) {
                    self.illegal(
                        DiagCode::UnknownProp,
                        property.span(),
                        format!("`props.{prop_name}` is not declared in controls.props"),
                    );
                    continue;
                }
                let expr = self.push(Expr::Prop { name: prop_name }, property.span());
                self.bind_dynamic(local_name, expr);
            }
        }
        if let Some(fourth) = params.items.get(3) {
            let value = self
                .prepare_data
                .as_ref()
                .map_or_else(|| serde_json::json!({}), |binding| binding.value.clone());
            self.bind_static_pattern(&fourth.pattern, value);
        }
    }

    pub(super) fn compile_body(&mut self, body: &'s FunctionBody<'s>) -> Option<PendingNode> {
        let mut root = None;
        for statement in &body.statements {
            match statement {
                Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                    for declarator in &declaration.declarations {
                        let Some(initializer) = &declarator.init else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                declarator.span(),
                                "const local needs an initializer",
                            );
                            continue;
                        };
                        self.bind_local_pattern(&declarator.id, initializer);
                    }
                }
                Statement::ReturnStatement(statement) => {
                    if root.is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            statement.span(),
                            "component has more than one return",
                        );
                        continue;
                    }
                    root = statement
                        .argument
                        .as_ref()
                        .and_then(|expression| self.compile_root_expression(expression));
                }
                Statement::ForOfStatement(statement) => {
                    self.lower_prepare_for_of(statement);
                }
                other => {
                    let message = self.control_flow_advice(other);
                    self.illegal(DiagCode::GrammarForbidden, other.span(), message);
                }
            }
        }
        root
    }

    pub(super) fn lower_prepare_for_of(
        &mut self,
        statement: &'s oxc::ast::ast::ForOfStatement<'s>,
    ) {
        if statement.r#await {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.span(),
                "prepare for...of cannot be async",
            );
            return;
        }
        let Some(serde_json::Value::Array(items)) = self.eval_static(&statement.right) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.right.span(),
                "for...of input must be a prepare-time static array; frame values cannot choose topology",
            );
            return;
        };
        if items.len() > MAX_STATIC_MAP_ITEMS
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.right.span(),
                format!(
                    "prepare-time for...of expands {} items; the per-loop limit is {MAX_STATIC_MAP_ITEMS} and the module budget is {MAX_TOTAL_EXPANDED_LIST_ITEMS}",
                    items.len()
                ),
            );
            return;
        }
        let oxc::ast::ast::ForStatementLeft::VariableDeclaration(declaration) = &statement.left
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.left.span(),
                "prepare for...of left side must be one `const` binding",
            );
            return;
        };
        if !declaration.kind.is_const() || declaration.declarations.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                declaration.span(),
                "prepare for...of left side must be one `const` binding",
            );
            return;
        }
        let declarator = &declaration.declarations[0];
        if declarator.init.is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                declarator.span(),
                "prepare for...of binding cannot have an initializer",
            );
            return;
        }
        let Statement::BlockStatement(body) = &statement.body else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.body.span(),
                "prepare for...of body must be a block",
            );
            return;
        };

        self.expanded_list_items += items.len();
        let saved_frame = self.bindings.snapshot_frame();
        let mut accumulated_children = saved_frame.children.clone();
        let mut accumulators = BTreeSet::new();
        self.list_depth += 1;
        for item in items {
            self.bindings.restore_frame(saved_frame.clone());
            self.bindings.children = accumulated_children;
            self.bind_static_pattern(&declarator.id, item);
            self.lower_prepare_for_of_body(body, &mut accumulators);
            accumulated_children = self.bindings.children.clone();
        }
        self.list_depth -= 1;
        self.bindings.restore_frame(saved_frame);
        for name in accumulators {
            self.bindings.statics.remove(&name);
            self.bindings.scalars.remove(&name);
            self.bindings.tuples.remove(&name);
            if let Some(children) = accumulated_children.remove(&name) {
                self.bindings.children.insert(name, children);
            }
        }
    }

    pub(super) fn lower_prepare_for_of_body(
        &mut self,
        body: &'s oxc::ast::ast::BlockStatement<'s>,
        accumulators: &mut BTreeSet<String>,
    ) {
        for statement in &body.body {
            match statement {
                Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                    for declarator in &declaration.declarations {
                        let Some(initializer) = &declarator.init else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                declarator.span(),
                                "for...of const needs an initializer",
                            );
                            continue;
                        };
                        self.bind_local_pattern(&declarator.id, initializer);
                    }
                }
                Statement::ExpressionStatement(statement) => {
                    self.lower_prepare_push(&statement.expression, accumulators);
                }
                other => self.illegal(
                    DiagCode::GrammarForbidden,
                    other.span(),
                    "prepare for...of only allows const bindings and `children.push(JSX)`",
                ),
            }
        }
    }

    pub(super) fn lower_prepare_push(
        &mut self,
        expression: &'s Expression<'s>,
        accumulators: &mut BTreeSet<String>,
    ) {
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "prepare loop expression must be `children.push(JSX)`",
            );
            return;
        };
        let Expression::StaticMemberExpression(member) = &call.callee else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "prepare loop expression must be `children.push(JSX)`",
            );
            return;
        };
        let Expression::Identifier(target) = &member.object else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                "prepare loop accumulator must be a local identifier",
            );
            return;
        };
        if member.property.name != "push" || call.arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "prepare loop only admits one `children.push(JSX)` argument",
            );
            return;
        }
        let name = target.name.to_string();
        let initialized_empty = matches!(
            self.bindings.statics.get(&name),
            Some(serde_json::Value::Array(items)) if items.is_empty()
        );
        if !initialized_empty && !self.bindings.children.contains_key(&name) {
            self.illegal(
                DiagCode::GrammarForbidden,
                target.span(),
                format!("for...of accumulator `{name}` must be initialized as `const {name} = []`"),
            );
            return;
        }
        self.bindings.statics.remove(&name);
        self.bindings.children.entry(name.clone()).or_default();
        let Some(argument) = call.arguments[0].as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.arguments[0].span(),
                "prepare loop push argument cannot spread",
            );
            return;
        };
        if let Some(mut nodes) = self.lower_node_expression(argument, &format!("for.{name}")) {
            self.bindings
                .children
                .get_mut(&name)
                .expect("accumulator installed")
                .append(&mut nodes);
            accumulators.insert(name);
        }
    }

    pub(super) fn compile_root_expression(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<PendingNode> {
        self.lower_node_expression(strip_parens(expression), "root")
            .and_then(|mut nodes| {
                if nodes.len() == 1 {
                    nodes.pop()
                } else if nodes.is_empty() {
                    None
                } else {
                    Some(PendingNode {
                        space: None,
                        span: expression.span(),
                        expansion_stack: self.component_stack.clone(),
                        key: self.scoped_key("__component_group__"),
                        kind: NodeKind::Group,
                        class_names: Vec::new(),
                        styles: Vec::new(),
                        visibility: None,
                        semantic: None,
                        children: nodes,
                        is_mask_source: false,
                    })
                }
            })
    }

    /// Explain both alternatives for rejected statement-level control flow: conditional values or
    /// visibility preserve runtime topology, while constant ternaries can fold during preparation.
    /// Do not guess whether an unlowered condition depends on the frame context.
    pub(super) fn control_flow_advice(&self, statement: &Statement<'_>) -> String {
        let kind = match statement {
            Statement::IfStatement(_) => Some("if"),
            Statement::WhileStatement(_) | Statement::DoWhileStatement(_) => Some("while"),
            Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_) => Some("for"),
            Statement::SwitchStatement(_) => Some("switch"),
            _ => None,
        };
        match kind {
            Some(kind @ ("if" | "switch")) => format!(
                "`{kind}` cannot pick the scene's topology: the frame-time IR is fixed-topology \
                 data, so a branch that depends on ctx has nothing to switch on. Put the branch \
                 on the value — `cond ? a : b` — or on visibility — `{{cond && <View/>}}`. \
                 Both keep one topology"
            ),
            Some(kind) => format!(
                "`{kind}` loops cannot run at frame time. Build the list at prepare time and \
                 expand it with `.map`, which produces one node per item and one fixed topology"
            ),
            None => "component body only allows `const` bindings and one final return; effects, \
                     loops and mutable state are illegal"
                .to_string(),
        }
    }

    pub(super) fn src(&self, expression: &Expression<'_>) -> &str {
        &self.source[expression.span().start as usize..expression.span().end as usize]
    }

    /// Try constant folding without mutating the expression arena. Return None when the expression
    /// depends on runtime data or cannot be evaluated.
    pub(super) fn fold_to_value(&mut self, expression: &Expression<'_>) -> Option<MotionValue> {
        let mark = self.expr_arena.values.len();
        let diagnostics = self.diagnostics.len();
        let lowered = self.lower_expr_raw(expression);
        let folded = lowered.and_then(|id| {
            // Skip folding subtrees known to read runtime inputs.
            if self
                .expr_arena
                .reads_runtime
                .get(id.0 as usize)
                .copied()
                .unwrap_or(true)
            {
                return None;
            }
            valle_motion::fold_constants(&self.expr_arena.values, &self.controls)
                .get(id.0 as usize)
                .cloned()
                .flatten()
        });
        self.expr_arena.values.truncate(mark);
        self.expr_arena.spans.truncate(mark);
        self.expr_arena.expansion_stacks.truncate(mark);
        self.expr_arena.reads_runtime.truncate(mark);
        // Discard speculative diagnostics; the caller supplies context-specific errors.
        //
        // Preserve explicit builtin rejections so invalid arguments retain their actionable
        // diagnostics.
        let rescued: Vec<CompilerDiagnostic> = self.diagnostics[diagnostics..]
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagCode::BuiltinRejected)
            .cloned()
            .collect();
        self.diagnostics.truncate(diagnostics);
        for diagnostic in rescued {
            self.push_diagnostic(diagnostic);
        }
        folded
    }

    pub(super) fn fold_to_number(&mut self, expression: &Expression<'_>) -> Option<f64> {
        match self.fold_to_value(expression)? {
            MotionValue::Number(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    pub(super) fn push(&mut self, expression: Expr, span: Span) -> ExprId {
        let id = ExprId(self.expr_arena.values.len() as u32);
        // Children precede parents in the arena; treat invalid indices as runtime-dependent.
        let flag = valle_motion::expr_reads_runtime_inputs(&expression)
            || expression.children().iter().any(|child| {
                self.expr_arena
                    .reads_runtime
                    .get(child.0 as usize)
                    .copied()
                    .unwrap_or(true)
            });
        self.expr_arena.reads_runtime.push(flag);
        self.expr_arena.values.push(expression);
        self.expr_arena.spans.push(span);
        self.expr_arena
            .expansion_stacks
            .push(self.component_stack.clone());
        id
    }

    /// Deduplicate diagnostics by code, source position, and message.
    pub(super) fn push_diagnostic(&mut self, diagnostic: CompilerDiagnostic) {
        // Compare line and column rather than byte spans, which may differ for equivalent
        // expression wrappers.
        let duplicate = self.diagnostics.iter().any(|existing| {
            existing.code == diagnostic.code
                && existing.span.line == diagnostic.span.line
                && existing.span.column == diagnostic.span.column
                && existing.message == diagnostic.message
        });
        if !duplicate {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn illegal(&mut self, code: DiagCode, span: Span, message: impl Into<String>) {
        debug_assert_eq!(code.class(), DiagClass::Illegal);
        self.push_diagnostic(diagnostic_at(self.source, code, span, message));
    }

    pub(super) fn unsupported(&mut self, span: Span, message: impl Into<String>) {
        self.push_diagnostic(diagnostic_at(
            self.source,
            DiagCode::UnsupportedSyntax,
            span,
            message,
        ));
    }
}
