//! OXC-level admission scans and normalized source generation.

use super::*;

#[derive(Default)]
pub(super) struct GeometryReferences {
    pub keys: BTreeSet<String>,
    pub unknown_key: bool,
}

/// Find author references before JSX maps are lowered. An instance row cannot replace a layout
/// node whose box is read elsewhere; nonliteral keys conservatively disable that replacement.
pub(super) fn geometry_references(program: &Program<'_>) -> GeometryReferences {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan(GeometryReferences);
    impl<'a> Visit<'a> for Scan {
        fn visit_call_expression(&mut self, call: &oxc::ast::ast::CallExpression<'a>) {
            if matches!(&call.callee, Expression::Identifier(id) if matches!(id.name.as_str(), "bounds" | "anchor"))
            {
                match call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .map(peel_expr)
                {
                    Some(Expression::StringLiteral(value)) => {
                        self.0.keys.insert(value.value.to_string());
                    }
                    _ => self.0.unknown_key = true,
                }
            }
            walk::walk_call_expression(self, call);
        }
    }

    let mut scan = Scan::default();
    scan.visit_program(program);
    scan.0
}

/// The `**` expressions of a module. QuickJS computes `**` with the host's `pow`, so source text
/// handed to the sandbox spells each one as the deterministic `Math.pow`, which frame-time
/// lowering also uses. Text and CSS containing the same characters are not operators.
#[derive(Default)]
pub(super) struct Exponentiations {
    /// Expression start, end of the base, start of the exponent, expression end.
    spans: Vec<[u32; 4]>,
    /// The first `**=`, which has no deterministic spelling of the same length.
    pub assignment: Option<Span>,
}

impl Exponentiations {
    pub(super) fn scan(program: &Program<'_>) -> Self {
        use oxc::ast_visit::{Visit, walk};

        struct Scan(Exponentiations);
        impl<'a> Visit<'a> for Scan {
            fn visit_binary_expression(&mut self, it: &oxc::ast::ast::BinaryExpression<'a>) {
                if it.operator == BinaryOperator::Exponential {
                    self.0.spans.push([
                        it.span.start,
                        it.left.span().end,
                        it.right.span().start,
                        it.span.end,
                    ]);
                }
                walk::walk_binary_expression(self, it);
            }
            fn visit_assignment_expression(
                &mut self,
                it: &oxc::ast::ast::AssignmentExpression<'a>,
            ) {
                if it.operator == oxc::syntax::operator::AssignmentOperator::Exponential {
                    self.0.assignment.get_or_insert(it.span);
                }
                walk::walk_assignment_expression(self, it);
            }
        }

        let mut scan = Scan(Self::default());
        scan.visit_program(program);
        scan.0
    }

    /// The source text of `span` with every `a ** b` inside it written as `Math.pow(a, b)`.
    pub(super) fn rewrite<'t>(&self, source: &'t str, span: Span) -> std::borrow::Cow<'t, str> {
        let text = &source[span.start as usize..span.end as usize];
        let mut edits = Vec::new();
        for &[start, base_end, exponent_start, end] in &self.spans {
            if span.start <= start && end <= span.end {
                edits.push((start, start, "Math.pow("));
                edits.push((base_end, exponent_start, ", "));
                edits.push((end, end, ")"));
            }
        }
        if edits.is_empty() {
            return text.into();
        }
        // Edits of nested expressions never overlap; at one offset an insertion precedes the
        // replacement that starts there.
        edits.sort_by_key(|&(start, end, _)| (start, end));
        let mut output = String::with_capacity(text.len() + edits.len() * 4);
        let mut at = span.start;
        for (start, end, replacement) in edits {
            output.push_str(&source[at as usize..start as usize]);
            output.push_str(replacement);
            at = end;
        }
        output.push_str(&source[at as usize..span.end as usize]);
        output.into()
    }
}

/// The first async function, `await` or `for await`. Preparation and frames are synchronous, so
/// nothing could wait for them.
pub(super) fn first_async(program: &Program<'_>) -> Option<Span> {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan(Option<Span>);
    impl<'a> Visit<'a> for Scan {
        fn visit_function(
            &mut self,
            it: &oxc::ast::ast::Function<'a>,
            flags: oxc::semantic::ScopeFlags,
        ) {
            if it.r#async {
                self.0.get_or_insert(it.span);
            }
            walk::walk_function(self, it, flags);
        }
        fn visit_arrow_function_expression(
            &mut self,
            it: &oxc::ast::ast::ArrowFunctionExpression<'a>,
        ) {
            if it.r#async {
                self.0.get_or_insert(it.span);
            }
            walk::walk_arrow_function_expression(self, it);
        }
        fn visit_await_expression(&mut self, it: &oxc::ast::ast::AwaitExpression<'a>) {
            self.0.get_or_insert(it.span);
            walk::walk_await_expression(self, it);
        }
        fn visit_for_of_statement(&mut self, it: &oxc::ast::ast::ForOfStatement<'a>) {
            if it.r#await {
                self.0.get_or_insert(it.span);
            }
            walk::walk_for_of_statement(self, it);
        }
    }

    let mut scan = Scan::default();
    scan.visit_program(program);
    scan.0
}

/// Object evaluation would silently keep the last duplicate property before `defineSequence`
/// can inspect it. Sequence labels are semantic addresses, so catch that lossy case on the AST.
pub(super) fn scan_sequence_labels(program: &Program<'_>) -> Option<(Span, String)> {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan {
        hit: Option<(Span, String)>,
    }
    impl<'a> Visit<'a> for Scan {
        fn visit_call_expression(&mut self, call: &oxc::ast::ast::CallExpression<'a>) {
            if self.hit.is_none()
                && matches!(&call.callee, Expression::Identifier(id) if id.name == "defineSequence")
                && let Some(expression) = call.arguments.first().and_then(Argument::as_expression)
                && let Expression::ObjectExpression(object) = strip_parens(expression)
            {
                let mut labels = BTreeSet::new();
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            let Some(label) = static_property_name(&property.key) else {
                                self.hit = Some((
                                    property.key.span(),
                                    "defineSequence labels must be static names".into(),
                                ));
                                break;
                            };
                            if !labels.insert(label.clone()) {
                                self.hit = Some((
                                    property.key.span(),
                                    format!("defineSequence has duplicate stage label `{label}`"),
                                ));
                                break;
                            }
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.hit = Some((
                                spread.span(),
                                "defineSequence cannot spread stages; labels and order must stay explicit"
                                    .into(),
                            ));
                            break;
                        }
                    }
                }
            }
            walk::walk_call_expression(self, call);
        }
    }

    let mut scan = Scan::default();
    scan.visit_program(program);
    scan.hit
}

/// Detect internal typed-value markers in serialized string values; typed object keys remain valid.
pub(super) fn string_leaks_typed_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains("__valleType"),
        serde_json::Value::Array(items) => items.iter().any(string_leaks_typed_marker),
        serde_json::Value::Object(fields) => fields.values().any(string_leaks_typed_marker),
        _ => false,
    }
}

/// Collect free identifier references, excluding property names and bindings introduced by nested
/// scopes. Track scopes during traversal so inner parameters do not hide outer references elsewhere
/// in the expression.
pub(super) fn referenced_identifiers(expression: &Expression<'_>) -> BTreeSet<String> {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan {
        free: BTreeSet<String>,
        scopes: Vec<BTreeSet<String>>,
    }
    impl<'a> Visit<'a> for Scan {
        fn visit_arrow_function_expression(
            &mut self,
            it: &oxc::ast::ast::ArrowFunctionExpression<'a>,
        ) {
            self.scopes.push(BTreeSet::new());
            walk::walk_arrow_function_expression(self, it);
            self.scopes.pop();
        }
        fn visit_function(
            &mut self,
            it: &oxc::ast::ast::Function<'a>,
            flags: oxc::semantic::ScopeFlags,
        ) {
            self.scopes.push(BTreeSet::new());
            walk::walk_function(self, it, flags);
            self.scopes.pop();
        }
        fn visit_binding_identifier(&mut self, it: &oxc::ast::ast::BindingIdentifier<'a>) {
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(it.name.to_string());
            }
            walk::walk_binding_identifier(self, it);
        }
        fn visit_identifier_reference(&mut self, it: &oxc::ast::ast::IdentifierReference<'a>) {
            let name = it.name.as_str();
            if !self.scopes.iter().any(|scope| scope.contains(name)) {
                self.free.insert(name.to_string());
            }
            walk::walk_identifier_reference(self, it);
        }
    }

    let mut scan = Scan::default();
    scan.visit_expression(expression);
    scan.free
}

/// Detect JSX in both variable and function declarations before passing code to QuickJS.
pub(super) fn contains_jsx(statement: &Statement<'_>) -> bool {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan {
        found: bool,
    }
    impl<'a> Visit<'a> for Scan {
        fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
            self.found = true;
            walk::walk_jsx_element(self, it);
        }
        fn visit_jsx_fragment(&mut self, it: &oxc::ast::ast::JSXFragment<'a>) {
            self.found = true;
            walk::walk_jsx_fragment(self, it);
        }
    }

    let mut scan = Scan::default();
    scan.visit_statement(statement);
    scan.found
}

/// Reprint the parsed AST without comments or source formatting before hashing it.
/// Declaration and JSX attribute order remain unchanged because they are semantic.
pub(super) fn normalized_ast(program: &Program<'_>) -> String {
    Codegen::new()
        .with_options(CodegenOptions {
            minify: false,
            comments: CommentOptions {
                normal: false,
                jsdoc: false,
                annotation: false,
                legal: oxc::codegen::LegalComment::None,
            },
            ..CodegenOptions::default()
        })
        .build(program)
        .code
}
