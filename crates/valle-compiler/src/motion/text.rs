//! Text, rich-span, and prepare-time theme lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_text_value(&mut self, expression: &'s Expression<'s>) -> Option<TextValue> {
        self.eval_static(expression)
            .and_then(|value| match value {
                serde_json::Value::String(value) => Some(TextValue::Static { value }),
                serde_json::Value::Number(value) => Some(TextValue::Static {
                    value: value.to_string(),
                }),
                // Represent authored boolean expressions as a single Text node.
                serde_json::Value::Bool(value) => Some(TextValue::Static {
                    value: value.to_string(),
                }),
                _ => None,
            })
            .or_else(|| {
                self.lower_expr(expression).map(|expr| {
                    // Wrap dynamic scalar text in a one-part Template to reuse deterministic
                    // formatting. Template admission rejects non-scalar values.
                    let expr = self.push(
                        Expr::Template {
                            parts: vec![valle_motion::TemplatePart::Expr { expr }],
                        },
                        expression.span(),
                    );
                    TextValue::Expr { expr }
                })
            })
    }

    /// JSX text is not ordinary Rust/JS whitespace. A single authored line keeps its explicit
    /// leading/trailing spaces (those spaces often separate adjacent `<Span>` runs), while source
    /// formatting around line breaks is indentation and must not leak into the paragraph. This is
    /// the same shape used by the established JSX transforms: trim indentation at newline
    /// boundaries and join non-empty physical lines with one space.
    pub(super) fn jsx_text_value(value: &str) -> Option<String> {
        let lines = value.replace("\r\n", "\n").replace('\r', "\n");
        let lines = lines.split('\n').collect::<Vec<_>>();
        let last_non_empty = lines.iter().rposition(|line| !line.trim().is_empty())?;
        let mut out = String::new();
        for (index, line) in lines.iter().enumerate() {
            let mut line = line.replace('\t', " ");
            if index != 0 {
                line = line.trim_start().to_owned();
            }
            if index != lines.len() - 1 {
                line = line.trim_end().to_owned();
            }
            if line.is_empty() {
                continue;
            }
            out.push_str(&line);
            if index != last_non_empty && !line.ends_with(' ') {
                out.push(' ');
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// Lower one authored `<Span>` into one or more existing Text leaves. The leaves remain
    /// adjacent children of the enclosing paragraph Group, so Takumi shapes and wraps the whole
    /// inline context once while retaining each run's style/source identity.
    pub(super) fn lower_span_runs(
        &mut self,
        element: &'s JSXElement<'s>,
        path: &str,
    ) -> Vec<(TextValue, Vec<StyleBinding>, Span)> {
        let mut styles = Vec::new();
        let (attributes, _) = self.jsx_attributes(element);
        for attribute in attributes {
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.name.span(),
                    "namespaced Span attributes are illegal",
                );
                continue;
            };
            if name.name != "style" {
                self.unsupported(
                    attribute.span(),
                    format!(
                        "Span attribute `{}` is not admitted; the first version accepts only style",
                        name.name
                    ),
                );
                continue;
            }
            let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    "Span style must be a fixed-shape object expression",
                );
                continue;
            };
            let Some(expression) = container.expression.as_expression() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    container.span(),
                    "Span style must be a fixed-shape object expression",
                );
                continue;
            };

            const ALLOWED: &[&str] = &[
                "color",
                "font-family",
                "font-size",
                "font-weight",
                "font-variation-settings",
                "font-style",
                "letter-spacing",
                "opacity",
            ];
            let object = self.resolve_style(expression);
            for entry in &object.properties {
                let name = &entry.name;
                if !name.starts_with("--") && !ALLOWED.contains(&camel_to_kebab(name).as_str()) {
                    self.unsupported(
                        entry.property.span(),
                        format!("Span style `{name}` is not admitted; allowed: color, fontFamily, fontSize, fontWeight, fontVariationSettings, fontStyle, letterSpacing, opacity and static custom properties"),
                    );
                }
            }
            let mut lowered = self.lower_style(&object, path, None);
            lowered.retain(|style| {
                style.property.starts_with("--") || ALLOWED.contains(&style.property.as_str())
            });
            styles.extend(lowered);
        }

        let mut runs = Vec::new();
        for child in &element.children {
            let value = match child {
                JSXChild::Text(value) => Self::jsx_text_value(value.value.as_str())
                    .map(|value| TextValue::Static { value }),
                JSXChild::ExpressionContainer(container)
                    if matches!(container.expression, JSXExpression::EmptyExpression(_)) =>
                {
                    None
                }
                JSXChild::ExpressionContainer(container) => container
                    .expression
                    .as_expression()
                    .and_then(|expression| self.lower_text_value(expression)),
                _ => {
                    self.unsupported(
                        child.span(),
                        "Span children must be static text or string/number expressions; nested elements are not admitted",
                    );
                    None
                }
            };
            if let Some(value) = value {
                runs.push((value, styles.clone(), child.span()));
            }
        }
        if runs.is_empty() {
            runs.push((
                TextValue::Static {
                    value: String::new(),
                },
                styles,
                element.span(),
            ));
        }
        runs
    }

    /// Lower the compile-time-only theme intrinsic. A provider is deliberately transparent: it
    /// contributes neither a key nor a node and therefore cannot perturb layout, locate paths, or
    /// renderer wire versions. Several children take its place among the parent's children, like
    /// a fragment's.
    pub(super) fn lower_theme_provider(
        &mut self,
        element: &'s JSXElement<'s>,
        path: &str,
    ) -> Option<PendingNode> {
        let mut value_expression = None;
        for attribute in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    "ThemeProvider does not admit prop spread; pass one explicit `value` object",
                );
                continue;
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.name.span(),
                    "ThemeProvider only admits the `value` attribute",
                );
                continue;
            };
            if name.name != "value" {
                self.illegal(
                    DiagCode::UnknownProp,
                    attribute.span(),
                    format!(
                        "ThemeProvider has no `{}` prop; it is a transparent compile-time scope",
                        name.name
                    ),
                );
                continue;
            }
            let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    "ThemeProvider `value` must be a prepare-time object expression",
                );
                continue;
            };
            let Some(expression) = container.expression.as_expression() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    container.span(),
                    "ThemeProvider `value` cannot be empty",
                );
                continue;
            };
            if value_expression.replace(expression).is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    "ThemeProvider has more than one `value` attribute",
                );
            }
        }

        let Some(value_expression) = value_expression else {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.opening_element.span(),
                "ThemeProvider requires a prepare-time `value` object",
            );
            return None;
        };
        let Some(overlay) = self.eval_static(value_expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                value_expression.span(),
                "ThemeProvider `value` must be prepare-time static; controls.props and ctx values cannot change theme scope",
            );
            return None;
        };
        if !overlay.is_object() {
            self.illegal(
                DiagCode::GrammarForbidden,
                value_expression.span(),
                "ThemeProvider `value` must be an object of typed tokens",
            );
            return None;
        }
        if let Err(message) = validate_theme(&overlay) {
            self.illegal(DiagCode::GrammarForbidden, value_expression.span(), message);
            return None;
        }

        let mut merged = self
            .current_theme
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        if let Err(message) = merge_theme(&mut merged, overlay, "theme") {
            self.illegal(DiagCode::GrammarForbidden, value_expression.span(), message);
            return None;
        }
        if let Err(message) = validate_theme(&merged) {
            self.illegal(DiagCode::GrammarForbidden, value_expression.span(), message);
            return None;
        }

        let saved_theme = self.current_theme.replace(merged);
        let diagnostics_before_children = self.diagnostics.len();
        let mut children = self.lower_child_nodes(
            &element.children,
            &format!("{path}.children."),
            "ThemeProvider children must be Motion JSX nodes; wrap text in `<Text>`",
        );
        self.current_theme = saved_theme;

        match children.len() {
            0 => {
                if self.diagnostics.len() == diagnostics_before_children {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        "ThemeProvider must contain a Motion JSX node",
                    );
                }
                None
            }
            1 => children.pop(),
            // A theme scope adds no box: its children take its place, like a fragment's.
            _ => Some(self.fragment_node(element.span(), path, children)),
        }
    }
}
