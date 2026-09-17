//! Resolve fixed-shape author objects before lowering any CSS. Runtime values remain
//! ordinary expression bindings; neither object allocation nor property enumeration
//! is part of frame evaluation.

use super::*;
use oxc::ast::ast::{ObjectProperty, PropertyKind};

#[derive(Clone)]
pub(super) struct StyleScope<'s> {
    bindings: Bindings<'s>,
    theme: Option<serde_json::Value>,
}

#[derive(Clone)]
pub(super) struct AuthoredObject<'s> {
    expression: &'s Expression<'s>,
    scope: Arc<StyleScope<'s>>,
}

pub(super) struct StyleProperty<'s> {
    pub name: String,
    pub property: &'s ObjectProperty<'s>,
    pub scope: Arc<StyleScope<'s>>,
}

pub(super) struct ResolvedStyle<'s> {
    pub span: Span,
    pub properties: Vec<StyleProperty<'s>>,
}

impl ResolvedStyle<'_> {
    pub fn contains(&self, name: &str) -> bool {
        self.properties
            .iter()
            .any(|property| camel_to_kebab(&property.name) == name)
    }
}

impl<'s> Compiler<'s> {
    pub(super) fn style_scope(&self) -> Arc<StyleScope<'s>> {
        Arc::new(StyleScope {
            bindings: self.bindings.clone(),
            theme: self.current_theme.clone(),
        })
    }

    pub(super) fn restore_style_scope(&mut self, scope: &StyleScope<'s>) {
        self.bindings = scope.bindings.clone();
        self.current_theme = scope.theme.clone();
    }

    /// Retain syntax even for static objects: JSON maps discard JS insertion order,
    /// and re-evaluating an initializer in a caller's scope would change shadowing.
    pub(super) fn capture_authored_object(
        &self,
        expression: &'s Expression<'s>,
    ) -> Option<AuthoredObject<'s>> {
        if !self.is_authored_object(expression, true, 0) {
            return None;
        }
        Some(AuthoredObject {
            expression,
            scope: self.style_scope(),
        })
    }

    fn is_authored_object(&self, expression: &Expression<'_>, local: bool, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match peel_expr(expression) {
            Expression::ObjectExpression(_) => true,
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if local && self.bindings.objects.contains_key(name) {
                    return true;
                }
                if local && self.has_local_style_name(name) {
                    return false;
                }
                self.module_const_inits
                    .get(name)
                    .is_some_and(|init| self.is_authored_object(init, false, depth + 1))
            }
            _ => false,
        }
    }

    fn has_local_style_name(&self, name: &str) -> bool {
        self.bindings.scalars.contains_key(name)
            || self.bindings.statics.contains_key(name)
            || self.bindings.tuples.contains_key(name)
            || self.bindings.children.contains_key(name)
            || self.bindings.local_functions.contains_key(name)
    }

    pub(super) fn resolve_style(&mut self, expression: &'s Expression<'s>) -> ResolvedStyle<'s> {
        let scope = self.style_scope();
        let mut properties = Vec::new();
        self.collect_style_properties(expression, scope.clone(), &mut properties, 0);
        self.restore_style_scope(&scope);
        ResolvedStyle {
            span: expression.span(),
            properties,
        }
    }

    fn collect_style_properties(
        &mut self,
        expression: &'s Expression<'s>,
        scope: Arc<StyleScope<'s>>,
        out: &mut Vec<StyleProperty<'s>>,
        depth: usize,
    ) {
        if depth > 64 {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style object references/spreads exceed the 64-level expansion limit",
            );
            return;
        }
        self.restore_style_scope(&scope);
        match peel_expr(expression) {
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if let Some(object) = self.bindings.objects.get(name).cloned() {
                    self.collect_style_properties(object.expression, object.scope, out, depth + 1);
                    return;
                }
                if !self.has_local_style_name(name)
                    && let Some(init) = self.module_const_inits.get(name).copied()
                {
                    let module = Arc::new(StyleScope {
                        bindings: Bindings::default(),
                        theme: None,
                    });
                    self.collect_style_properties(init, module, out, depth + 1);
                    return;
                }
                self.unsupported(
                    expression.span(),
                    format!("style `{name}` must refer to an immutable object with a statically known property set"),
                );
            }
            Expression::ObjectExpression(object) => {
                for item in &object.properties {
                    self.restore_style_scope(&scope);
                    let property = match item {
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.collect_style_properties(&spread.argument, scope.clone(), out, depth + 1);
                            continue;
                        }
                        ObjectPropertyKind::ObjectProperty(property) => property,
                    };
                    if property.method || property.kind != PropertyKind::Init {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.span(),
                            "style objects cannot contain getters, setters, or methods",
                        );
                        continue;
                    }
                    let name = if property.computed {
                        property.key.as_expression()
                            .and_then(|key| self.eval_static(key))
                            .and_then(|key| key.as_str().map(str::to_owned))
                    } else {
                        static_property_name(&property.key)
                    };
                    let Some(name) = name else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            "computed style names must be prepare-time strings",
                        );
                        continue;
                    };
                    if name == "__proto__" {
                        self.illegal(DiagCode::GrammarForbidden, property.span(), "style prototype changes are illegal");
                        continue;
                    }
                    let entry = StyleProperty { name, property, scope: scope.clone() };
                    // JS replacement changes the value but retains the first key's
                    // insertion position, including replacements through spreads.
                    if let Some(index) = out.iter().position(|old| old.name == entry.name) {
                        out[index] = entry;
                    } else {
                        out.push(entry);
                    }
                }
            }
            _ => self.unsupported(
                expression.span(),
                "style and spreads require an immutable object literal/reference with a fixed property set",
            ),
        }
    }
}
