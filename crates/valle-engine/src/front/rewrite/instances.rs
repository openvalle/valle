//! A deliberately small, side-effect-free subset of map callbacks. The runtime additionally
//! checks captured values and row data before tracing one callback for the whole table.

use oxc::ast::ast::{
    ArrowFunctionBody, AssignmentTarget, BindingPattern, CallExpression, Expression,
    ForStatementLeft, Function, FunctionType, ObjectPropertyKind, PropertyKind, Statement,
    VariableDeclarationKind,
};

use super::{bare, key};

pub(super) struct Map {
    pub captures: Vec<String>,
    pub fields: Vec<String>,
    pub arrays: Vec<String>,
    pub functions: Vec<String>,
    pub tag: String,
}

impl Map {
    pub fn environment(&self) -> String {
        format!(
            "[Math, {}, [{}], [{}], [{}]]",
            self.tag,
            self.captures.join(","),
            self.arrays.join(","),
            self.functions.join(",")
        )
    }
}

/// A helper with scalar parameters and a pure return expression. Its actual captured values
/// and identity are checked at the map call; a replacement function is not trusted by name.
pub(super) fn helper(f: &Function<'_>) -> Option<Map> {
    if f.r#type != FunctionType::FunctionDeclaration
        || f.r#async
        || f.generator
        || f.params.rest.is_some()
    {
        return None;
    }
    f.id.as_ref()?;
    let mut check = Pure::default();
    for p in &f.params.items {
        let BindingPattern::BindingIdentifier(id) = &p.pattern else {
            return None;
        };
        if p.initializer.is_some() {
            return None;
        }
        check.bind(id.name.as_str())?;
    }
    let [Statement::ReturnStatement(r)] = &f.body.as_ref()?.statements[..] else {
        return None;
    };
    if !check.value(r.argument.as_ref()?) {
        return None;
    }
    Some(check.finish())
}

pub(super) fn captures(call: &CallExpression<'_>) -> Option<Map> {
    if call.arguments.len() != 1 {
        return None;
    }
    let Expression::ArrowFunctionExpression(f) = bare(call.arguments[0].as_expression()?) else {
        return None;
    };
    if f.r#async || f.params.rest.is_some() || f.params.items.len() > 2 {
        return None;
    }
    let mut check = Pure::default();
    for param in &f.params.items {
        if param.initializer.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(id) = &param.pattern else {
            return None;
        };
        check.bind(id.name.as_str())?;
    }
    // Only the first parameter can be a record. Member reads on captured objects or local
    // aliases could invoke getters and therefore must retain ordinary map execution.
    check.record = check.locals.first().cloned();
    let valid = if let Some(e) = f.body.as_expression() {
        check.element(e)
    } else {
        let ArrowFunctionBody::FunctionBody(body) = &f.body else {
            return None;
        };
        let (last, rest) = body.statements.split_last()?;
        for s in rest {
            if !check.statement(s) {
                return None;
            }
        }
        let Statement::ReturnStatement(r) = last else {
            return None;
        };
        check.element(r.argument.as_ref()?)
    };
    if check.tag.as_deref() == Some("View")
        && (!check.functions.is_empty() || !check.arrays.is_empty() || !check.mutable.is_empty())
    {
        return None;
    }
    (valid && !(check.direct && !check.fields.is_empty())).then(|| check.finish())
}

#[derive(Default)]
struct Pure {
    locals: Vec<String>,
    captures: Vec<String>,
    record: Option<String>,
    direct: bool,
    fields: Vec<String>,
    arrays: Vec<String>,
    functions: Vec<String>,
    mutable: Vec<String>,
    tag: Option<String>,
}

impl Pure {
    fn finish(self) -> Map {
        Map {
            captures: self.captures,
            fields: self.fields,
            arrays: self.arrays,
            functions: self.functions,
            tag: self.tag.unwrap_or_else(|| "null".into()),
        }
    }

    fn statement(&mut self, s: &Statement<'_>) -> bool {
        match s {
            Statement::VariableDeclaration(d)
                if matches!(
                    d.kind,
                    VariableDeclarationKind::Const | VariableDeclarationKind::Let
                ) =>
            {
                for v in &d.declarations {
                    let BindingPattern::BindingIdentifier(id) = &v.id else {
                        return false;
                    };
                    if !v.init.as_ref().is_some_and(|e| self.value(e))
                        || self.bind(id.name.as_str()).is_none()
                    {
                        return false;
                    }
                    if d.kind == VariableDeclarationKind::Let {
                        self.mutable.push(id.name.to_string());
                    }
                }
                true
            }
            Statement::ExpressionStatement(s) => {
                if let Expression::AssignmentExpression(a) = bare(&s.expression) {
                    (a.operator.as_str() == "=" || a.operator.to_binary_operator().is_some())
                        && matches!(&a.left, AssignmentTarget::AssignmentTargetIdentifier(i) if self.mutable.iter().any(|s| s == i.name.as_str()))
                        && self.value(&a.right)
                } else {
                    self.value(&s.expression)
                }
            }
            Statement::ForOfStatement(f) if !f.r#await => {
                let Expression::Identifier(array) = bare(&f.right) else {
                    return false;
                };
                if self.locals.iter().any(|s| s == array.name.as_str()) {
                    return false;
                }
                let ForStatementLeft::VariableDeclaration(d) = &f.left else {
                    return false;
                };
                if d.kind != VariableDeclarationKind::Const || d.declarations.len() != 1 {
                    return false;
                }
                let BindingPattern::BindingIdentifier(id) = &d.declarations[0].id else {
                    return false;
                };
                if !self.arrays.iter().any(|s| s == array.name.as_str()) {
                    self.arrays.push(array.name.to_string());
                }
                let locals = self.locals.len();
                let mutable = self.mutable.len();
                if self.bind(id.name.as_str()).is_none() {
                    return false;
                }
                let valid = match &f.body {
                    Statement::BlockStatement(b) => b.body.iter().all(|s| self.statement(s)),
                    s => self.statement(s),
                };
                self.locals.truncate(locals);
                self.mutable.truncate(mutable);
                valid
            }
            _ => false,
        }
    }

    fn bind(&mut self, name: &str) -> Option<()> {
        if matches!(name, "Math" | "View") || self.locals.iter().any(|s| s == name) {
            return None;
        }
        self.locals.push(name.into());
        Some(())
    }

    fn element(&mut self, e: &Expression<'_>) -> bool {
        let Expression::CallExpression(c) = bare(e) else {
            return false;
        };
        if !matches!(&c.callee, Expression::Identifier(i) if i.name == "__h")
            || c.arguments.len() != 2
        {
            return false;
        }
        self.tag = match c.arguments[0].as_expression() {
            Some(Expression::Identifier(i)) if i.name == "View" => Some("View".into()),
            Some(Expression::StringLiteral(s))
                if matches!(
                    s.value.as_str(),
                    Some("path" | "polyline" | "polygon" | "circle" | "ellipse" | "line" | "rect")
                ) =>
            {
                Some(super::json(s.value.as_str().unwrap()))
            }
            _ => return false,
        };
        c.arguments[1]
            .as_expression()
            .is_some_and(|e| self.object(e, true))
    }

    fn object(&mut self, e: &Expression<'_>, props: bool) -> bool {
        let Expression::ObjectExpression(o) = bare(e) else {
            return false;
        };
        o.properties.iter().all(|p| {
            let ObjectPropertyKind::ObjectProperty(p) = p else {
                return false;
            };
            let Ok(Some(name)) = key(p) else {
                return false;
            };
            p.kind == PropertyKind::Init
                && !p.method
                && !p.computed
                && name != "__proto__"
                && if props && name == "style" {
                    self.object(&p.value, false)
                } else {
                    self.value(&p.value)
                }
        })
    }

    fn value(&mut self, e: &Expression<'_>) -> bool {
        match bare(e) {
            Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_) => true,
            Expression::Identifier(i) => {
                let name = i.name.as_str();
                self.direct |= self.record.as_deref() == Some(name);
                if !self.locals.iter().any(|s| s == name)
                    && !self.captures.iter().any(|s| s == name)
                {
                    self.captures.push(name.into());
                }
                true
            }
            Expression::BinaryExpression(b) => {
                !matches!(b.operator.as_str(), "in" | "instanceof")
                    && self.value(&b.left)
                    && self.value(&b.right)
            }
            Expression::UnaryExpression(u) => {
                matches!(u.operator.as_str(), "+" | "-" | "!" | "~") && self.value(&u.argument)
            }
            Expression::LogicalExpression(l) => self.value(&l.left) && self.value(&l.right),
            Expression::ConditionalExpression(c) => {
                self.value(&c.test) && self.value(&c.consequent) && self.value(&c.alternate)
            }
            Expression::TemplateLiteral(t) => t.expressions.iter().all(|e| self.value(e)),
            Expression::ArrayExpression(a) => a
                .elements
                .iter()
                .all(|e| e.as_expression().is_some_and(|e| self.value(e))),
            Expression::ObjectExpression(_) => self.object(e, false),
            Expression::StaticMemberExpression(m) if !m.optional => {
                if matches!(&m.object, Expression::Identifier(i) if i.name == "Math")
                    && matches!(
                        m.property.name.as_str(),
                        "E" | "PI" | "LN2" | "LN10" | "LOG2E" | "LOG10E" | "SQRT1_2" | "SQRT2"
                    )
                {
                    return true;
                }
                if !matches!(&m.object, Expression::Identifier(i) if self.record.as_deref() == Some(i.name.as_str()))
                {
                    return false;
                }
                let name = m.property.name.as_str();
                if !self.fields.iter().any(|s| s == name) {
                    self.fields.push(name.into());
                }
                true
            }
            Expression::CallExpression(c) if !c.optional => {
                if let Expression::Identifier(i) = &c.callee {
                    if self.locals.iter().any(|s| s == i.name.as_str()) {
                        return false;
                    }
                    if !self.functions.iter().any(|s| s == i.name.as_str()) {
                        self.functions.push(i.name.to_string());
                    }
                    return c
                        .arguments
                        .iter()
                        .all(|a| a.as_expression().is_some_and(|e| self.value(e)));
                }
                let Expression::StaticMemberExpression(m) = &c.callee else {
                    return false;
                };
                !m.optional
                    && matches!(&m.object, Expression::Identifier(i) if i.name == "Math")
                    && crate::js::MathFn::parse(m.property.name.as_str()).is_some()
                    && c.arguments
                        .iter()
                        .all(|a| a.as_expression().is_some_and(|e| self.value(e)))
            }
            _ => false,
        }
    }
}
