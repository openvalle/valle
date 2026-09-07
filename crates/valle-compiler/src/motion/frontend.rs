//! OXC-level admission scans and normalized source generation.

use super::*;

/// Reject exponentiation through the AST because host-dependent results break deterministic
/// artifacts. Text and CSS containing the same characters must remain valid.
pub(super) fn scan_exponentiation(program: &Program<'_>) -> Option<Span> {
    use oxc::ast_visit::{Visit, walk};

    #[derive(Default)]
    struct Scan {
        hit: Option<Span>,
    }
    impl<'a> Visit<'a> for Scan {
        fn visit_binary_expression(&mut self, it: &oxc::ast::ast::BinaryExpression<'a>) {
            if it.operator == BinaryOperator::Exponential && self.hit.is_none() {
                self.hit = Some(it.span);
            }
            walk::walk_binary_expression(self, it);
        }
        fn visit_assignment_expression(&mut self, it: &oxc::ast::ast::AssignmentExpression<'a>) {
            if it.operator == oxc::syntax::operator::AssignmentOperator::Exponential
                && self.hit.is_none()
            {
                self.hit = Some(it.span);
            }
            walk::walk_assignment_expression(self, it);
        }
    }

    let mut scan = Scan::default();
    scan.visit_program(program);
    scan.hit
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
