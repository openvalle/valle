//! Compile finite class choices to candidate utilities and bool selectors. Independent
//! template holes separated by whitespace stay independent; no complete-style Cartesian
//! product is built and no class string is parsed while rendering a frame.

use super::*;
use valle_motion::TemplatePart;

type ClassChoices = Vec<(String, Option<ExprId>)>;
const MAX_TOKEN_CHOICES: usize = 64;

impl Compiler<'_> {
    pub(super) fn lower_classes(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        path: &str,
        names: &mut Vec<String>,
        conditions: &mut BTreeMap<String, ExprId>,
    ) {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => {
                self.add_class_names(&value.value, None, span, path, names, conditions);
            }
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                if let Some(expression) = container.expression.as_expression()
                    && let Some(expr) = self.lower_expr(expression)
                {
                    self.class_expression(expr, None, span, path, names, conditions, 0, &mut 4096);
                }
            }
            _ => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "className requires a string or finite conditional/template of utility strings",
            ),
        }
    }

    fn class_guard(&mut self, a: Option<ExprId>, b: Option<ExprId>, span: Span) -> Option<ExprId> {
        match (a, b) {
            (Some(a), Some(b)) if a != b => Some(self.and_expr(a, b, span)),
            (Some(a), _) | (_, Some(a)) => Some(a),
            _ => None,
        }
    }

    pub(super) fn add_class_names(
        &mut self,
        value: &str,
        guard: Option<ExprId>,
        span: Span,
        path: &str,
        names: &mut Vec<String>,
        conditions: &mut BTreeMap<String, ExprId>,
    ) {
        for class in value.split_whitespace() {
            if let Err(error) = valle_motion::tailwind::validate_tailwind_class(class) {
                let mut message = error.message(class);
                if guard.is_some() {
                    message.push_str(
                        "; candidate from a conditional className (all branches are validated)",
                    );
                }
                let mut diagnostic = diagnostic_at(self.source, error.code(), span, message);
                diagnostic.node_path = Some(path.into());
                diagnostic.utility = Some(class.into());
                diagnostic.style = error.style_issue().cloned();
                self.push_diagnostic(diagnostic);
                continue;
            }
            let existed = names.iter().any(|name| name == class);
            if !existed {
                names.push(class.into());
            }
            match (conditions.get(class).copied(), guard) {
                (_, None) => {
                    conditions.remove(class);
                }
                (None, Some(guard)) if !existed => {
                    conditions.insert(class.into(), guard);
                }
                (Some(a), Some(b)) if a != b => {
                    let yes = self.push(
                        Expr::Const {
                            value: MotionValue::Bool(true),
                        },
                        span,
                    );
                    let combined = self.push(
                        Expr::Select {
                            condition: a,
                            when_true: yes,
                            when_false: b,
                        },
                        span,
                    );
                    conditions.insert(class.into(), combined);
                }
                _ => {}
            }
        }
    }

    fn class_expression(
        &mut self,
        expr: ExprId,
        guard: Option<ExprId>,
        span: Span,
        path: &str,
        names: &mut Vec<String>,
        conditions: &mut BTreeMap<String, ExprId>,
        depth: usize,
        budget: &mut usize,
    ) {
        if depth >= 64 {
            self.unsupported(span, "className conditional nesting exceeds 64 levels");
            return;
        }
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        if *budget == 0 {
            self.unsupported(span, "className exceeds the finite branch expansion budget");
            return;
        }
        match self.expr_arena.values.get(expr.0 as usize).cloned() {
            Some(Expr::Const { value: MotionValue::Str(value) | MotionValue::Enum(value) }) => {
                let origin = self.expr_arena.spans.get(expr.0 as usize).copied().unwrap_or(span);
                self.add_class_names(&value, guard, origin, path, names, conditions);
            }
            Some(Expr::Select { condition, when_true, when_false }) => {
                if when_true == when_false {
                    self.class_expression(when_true, guard, span, path, names, conditions, depth + 1, budget);
                    return;
                }
                let yes = self.class_guard(guard, Some(condition), span);
                let no = self.not_expr(condition, span);
                let no = self.class_guard(guard, Some(no), span);
                self.class_expression(when_true, yes, span, path, names, conditions, depth + 1, budget);
                self.class_expression(when_false, no, span, path, names, conditions, depth + 1, budget);
            }
            Some(Expr::Template { parts }) => {
                let mut groups = Vec::new();
                let mut group = Vec::new();
                for part in parts {
                    match part {
                        TemplatePart::Text { value } => {
                            let mut text = String::new();
                            for character in value.chars() {
                                if character.is_whitespace() {
                                    if !text.is_empty() {
                                        group.push(TemplatePart::Text { value: std::mem::take(&mut text) });
                                    }
                                    if !group.is_empty() { groups.push(std::mem::take(&mut group)); }
                                } else {
                                    text.push(character);
                                }
                            }
                            if !text.is_empty() { group.push(TemplatePart::Text { value: text }); }
                        }
                        part => group.push(part),
                    }
                }
                if !group.is_empty() { groups.push(group); }
                for group in groups {
                    if let Some(choices) = self.class_parts(&group, span, depth + 1) {
                        for (value, condition) in choices {
                            let condition = self.class_guard(guard, condition, span);
                            self.add_class_names(&value, condition, span, path, names, conditions);
                        }
                    }
                }
            }
            _ => self.unsupported(span,
                "className choices must be finite strings; use a ternary between complete utilities instead of arbitrary runtime strings"),
        }
    }

    fn class_parts(
        &mut self,
        parts: &[TemplatePart],
        span: Span,
        depth: usize,
    ) -> Option<ClassChoices> {
        let mut choices = vec![(String::new(), None)];
        for part in parts {
            let next = match part {
                TemplatePart::Text { value } => vec![(value.clone(), None)],
                TemplatePart::Expr { expr } => self.class_fragment(*expr, span, depth + 1)?,
            };
            if choices.len().saturating_mul(next.len()) > MAX_TOKEN_CHOICES {
                self.unsupported(span, "one class token exceeds 64 finite choices; split independent utilities with whitespace");
                return None;
            }
            let mut combined = Vec::new();
            for (left, a) in &choices {
                for (right, b) in &next {
                    combined.push((format!("{left}{right}"), self.class_guard(*a, *b, span)));
                }
            }
            choices = combined;
        }
        Some(choices)
    }

    fn class_fragment(&mut self, expr: ExprId, span: Span, depth: usize) -> Option<ClassChoices> {
        if depth >= 64 {
            self.unsupported(span, "className template nesting exceeds 64 levels");
            return None;
        }
        match self.expr_arena.values.get(expr.0 as usize).cloned() {
            Some(Expr::Const {
                value: MotionValue::Str(value) | MotionValue::Enum(value),
            }) => Some(vec![(value, None)]),
            Some(Expr::Select {
                condition,
                when_true,
                when_false,
            }) => {
                if when_true == when_false {
                    return self.class_fragment(when_true, span, depth + 1);
                }
                let yes = self.class_fragment(when_true, span, depth + 1)?;
                let no = self.class_fragment(when_false, span, depth + 1)?;
                if yes.len() + no.len() > MAX_TOKEN_CHOICES {
                    self.unsupported(span, "one class token exceeds 64 finite choices");
                    return None;
                }
                let inverse = self.not_expr(condition, span);
                let mut choices = Vec::new();
                for (text, guard) in yes {
                    choices.push((text, self.class_guard(guard, Some(condition), span)));
                }
                for (text, guard) in no {
                    choices.push((text, self.class_guard(guard, Some(inverse), span)));
                }
                Some(choices)
            }
            Some(Expr::Template { parts }) => self.class_parts(&parts, span, depth + 1),
            _ => {
                self.unsupported(
                    span,
                    "className template holes require finite string choices",
                );
                None
            }
        }
    }
}
