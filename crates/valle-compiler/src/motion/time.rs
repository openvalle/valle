//! `useTime()`: the time of the frame being drawn. Components take only `props`, as in React, and
//! read the time through this hook. A name bound to it, or to an object in it, keeps the path
//! under it; every read lowers where it is written, so a `TimeScope` around the read applies.

use super::*;

/// Whether `call` is `useTime()`.
pub(super) fn is_use_time(call: &oxc::ast::ast::CallExpression<'_>) -> bool {
    matches!(&call.callee, Expression::Identifier(callee) if callee.name == "useTime")
        && call.arguments.is_empty()
}

/// Whether `path` names an object in the time rather than one of its numbers.
pub(super) fn is_time_object(path: &[String]) -> bool {
    match path {
        [] => true,
        [name] => matches!(
            name.as_str(),
            "fps" | "host" | "unit" | "viewport" | "instance"
        ),
        _ => false,
    }
}

impl<'s> Compiler<'s> {
    /// The path under the time that `expression` reads: `useTime()`, a name or component prop
    /// bound to the time, or a static member chain from any of them.
    pub(super) fn time_path(&self, expression: &Expression<'_>) -> Option<Vec<String>> {
        let mut segments = Vec::new();
        let mut current = peel_expr(expression);
        let mut path = loop {
            match current {
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.to_string());
                    current = peel_expr(&member.object);
                }
                Expression::Identifier(identifier) => {
                    if let Some(path) = self.bindings.times.get(identifier.name.as_str()) {
                        break path.clone();
                    }
                    if identifier.name != "props" {
                        return None;
                    }
                    let name = segments.pop()?;
                    match self.bindings.component_props.as_ref()?.get(&name)? {
                        AuthorValue::Time(path) => break path.clone(),
                        _ => return None,
                    }
                }
                Expression::CallExpression(call) if is_use_time(call) => break Vec::new(),
                _ => return None,
            }
        };
        path.extend(segments.into_iter().rev());
        Some(path)
    }

    /// Whether `expression` reads the frame rate, as the frame conversions of `spring`, `wiggle`
    /// and sequences require.
    pub(super) fn is_time_fps(&self, expression: &Expression<'_>) -> bool {
        self.time_path(expression)
            .is_some_and(|path| matches!(path.as_slice(), [fps] if fps == "fps"))
    }

    /// Lower one number of the time, at the place it is read.
    pub(super) fn lower_time_path(&mut self, path: &[String], span: Span) -> Option<ExprId> {
        if self.instance_scope.is_some() {
            match path {
                [scope, field] if scope == "instance" && field == "index" => {
                    return Some(self.push(Expr::InstanceIndex, span));
                }
                [scope, field] if scope == "instance" && field == "count" => {
                    return Some(self.push(Expr::InstanceCount, span));
                }
                _ => {}
            }
        }
        if is_time_object(path) {
            self.illegal(
                DiagCode::UnknownContextPath,
                span,
                if path.is_empty() {
                    "`useTime()` holds several numbers; read one, such as `useTime().progress`"
                        .to_owned()
                } else {
                    format!(
                        "`{}` holds several numbers in `useTime()`; read one of them",
                        path.join(".")
                    )
                },
            );
            return None;
        }
        let input = context_input(path).or_else(|| {
            self.illegal(
                DiagCode::UnknownContextPath,
                span,
                format!("`{}` is not a number in `useTime()`", path.join(".")),
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
        Some(self.scoped_context_expr(input, span))
    }

    /// Bind `name` to the time, or to an object in it.
    pub(super) fn bind_time(&mut self, name: String, path: Vec<String>) {
        self.bindings.unbind(&name);
        self.bindings.times.insert(name, path);
    }

    /// Bind what an initializer reads from the time: a name keeps an object of the time, and a
    /// number is read once, where it is bound, like any other local.
    pub(super) fn bind_time_value(
        &mut self,
        pattern: &BindingPattern<'_>,
        path: Vec<String>,
        span: Span,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                let name = identifier.name.to_string();
                if is_time_object(&path) {
                    self.bind_time(name, path);
                } else if let Some(expr) = self.lower_time_path(&path, span) {
                    self.bind_dynamic(name, expr);
                }
            }
            // Every field of the time exists, so a default never applies.
            BindingPattern::AssignmentPattern(assignment) => {
                self.bind_time_value(&assignment.left, path, span);
            }
            BindingPattern::ObjectPattern(object) => {
                if let Some(rest) = &object.rest {
                    self.unsupported(
                        rest.span(),
                        "bind the fields of `useTime()` that you read; it has no rest",
                    );
                }
                for property in &object.properties {
                    let Some(field) = self.destructured_name(property) else {
                        continue;
                    };
                    let mut field_path = path.clone();
                    field_path.push(field);
                    self.bind_time_value(&property.value, field_path, property.span());
                }
            }
            BindingPattern::ArrayPattern(array) => self.unsupported(
                array.span(),
                "`useTime()` is an object; destructure it with `{ progress, seconds }`",
            ),
        }
    }
}
