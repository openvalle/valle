//! A deliberately small, side-effect-free subset of map callbacks. The runtime additionally
//! checks captured values and row data before tracing one callback for the whole table.

use oxc::ast::ast::{
    ArrowFunctionBody, BindingPattern, CallExpression, Expression, ObjectPropertyKind,
    PropertyKind, Statement, VariableDeclarationKind,
};

use super::{bare, key};

pub(super) struct Map {
    pub captures: Vec<String>,
    pub fields: Vec<String>,
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
            let Statement::VariableDeclaration(d) = s else {
                return None;
            };
            if d.kind != VariableDeclarationKind::Const {
                return None;
            }
            for v in &d.declarations {
                let BindingPattern::BindingIdentifier(id) = &v.id else {
                    return None;
                };
                if !check.value(v.init.as_ref()?) {
                    return None;
                }
                check.bind(id.name.as_str())?;
            }
        }
        let Statement::ReturnStatement(r) = last else {
            return None;
        };
        check.element(r.argument.as_ref()?)
    };
    (valid && !(check.direct && !check.fields.is_empty())).then_some(Map {
        captures: check.captures,
        fields: check.fields,
    })
}

#[derive(Default)]
struct Pure {
    locals: Vec<String>,
    captures: Vec<String>,
    record: Option<String>,
    direct: bool,
    fields: Vec<String>,
}

impl Pure {
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
        matches!(&c.callee, Expression::Identifier(i) if i.name == "__h")
            && c.arguments.len() == 2
            && matches!(c.arguments[0].as_expression(), Some(Expression::Identifier(i)) if i.name == "View")
            && c.arguments[1]
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
            Expression::StaticMemberExpression(m) if !m.optional => {
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
