//! Recognize closed prepare-time numeric functions of an Array.from row index.
//! Every candidate is checked against all authored values before entering the artifact.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn prepared_index_formulas(
        &mut self,
        source: &'s Expression<'s>,
        rows: usize,
    ) -> BTreeMap<String, IndexFormula> {
        self.prepared_index_formulas_inner(source, rows, &mut BTreeSet::new())
            .unwrap_or_default()
    }

    fn prepared_index_formulas_inner(
        &mut self,
        source: &'s Expression<'s>,
        rows: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<BTreeMap<String, IndexFormula>> {
        match peel_expr(source) {
            Expression::Identifier(id) => {
                let name = id.name.to_string();
                if !seen.insert(name.clone()) {
                    return None;
                }
                let initializer = self.module_const_inits.get(&name).copied()?;
                self.prepared_index_formulas_inner(initializer, rows, seen)
            }
            Expression::CallExpression(call) => {
                let Expression::StaticMemberExpression(member) = &call.callee else {
                    return None;
                };
                if !matches!(&member.object, Expression::Identifier(id) if id.name == "Array")
                    || member.property.name != "from"
                    || call.arguments.len() != 2
                {
                    return None;
                }
                let length = call.arguments[0]
                    .as_expression()
                    .and_then(|argument| self.eval_static(argument))?
                    .get("length")?
                    .as_u64()? as usize;
                if length != rows {
                    return None;
                }
                let Expression::ArrowFunctionExpression(arrow) =
                    call.arguments[1].as_expression().map(peel_expr)?
                else {
                    return None;
                };
                if arrow.params.items.len() != 2 || arrow.params.rest.is_some() {
                    return None;
                }
                let index = arrow.params.items[1]
                    .pattern
                    .get_identifier_name()?
                    .to_string();
                let mut locals = BTreeMap::new();
                if let Some(expression) = arrow.get_expression() {
                    return self.formulas_from_object(expression, &index, &locals);
                }
                let body = arrow.body.as_function_body()?;
                for statement in &body.statements {
                    match statement {
                        Statement::VariableDeclaration(declaration)
                            if declaration.kind.is_const() =>
                        {
                            for declarator in &declaration.declarations {
                                let name = declarator.id.get_identifier_name()?.to_string();
                                let expression = self.lower_index_formula(
                                    declarator.init.as_ref()?,
                                    &index,
                                    &locals,
                                    0,
                                )?;
                                if !expression.is_bounded() {
                                    return None;
                                }
                                locals.insert(name, expression);
                            }
                        }
                        Statement::ReturnStatement(statement) => {
                            return self.formulas_from_object(
                                statement.argument.as_ref()?,
                                &index,
                                &locals,
                            );
                        }
                        _ => return None,
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn formulas_from_object(
        &mut self,
        expression: &'s Expression<'s>,
        index: &str,
        locals: &BTreeMap<String, IndexFormula>,
    ) -> Option<BTreeMap<String, IndexFormula>> {
        let Expression::ObjectExpression(object) = peel_expr(expression) else {
            return None;
        };
        let mut fields = BTreeMap::new();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                return None;
            };
            let name = match &property.key {
                PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                PropertyKey::StringLiteral(id) => id.value.to_string(),
                _ => return None,
            };
            if let Some(formula) = self.lower_index_formula(&property.value, index, locals, 0)
                && formula.is_bounded()
            {
                fields.insert(name, formula);
            }
        }
        Some(fields)
    }

    fn lower_index_formula(
        &mut self,
        expression: &'s Expression<'s>,
        index: &str,
        locals: &BTreeMap<String, IndexFormula>,
        depth: usize,
    ) -> Option<IndexFormula> {
        if depth > 32 {
            return None;
        }
        let next = depth + 1;
        match peel_expr(expression) {
            Expression::Identifier(id) if id.name == index => Some(IndexFormula::Index),
            Expression::Identifier(id) if locals.contains_key(id.name.as_str()) => {
                locals.get(id.name.as_str()).cloned()
            }
            Expression::BinaryExpression(binary) => {
                let left = Box::new(self.lower_index_formula(&binary.left, index, locals, next)?);
                let right =
                    Box::new(self.lower_index_formula(&binary.right, index, locals, next)?);
                Some(match binary.operator {
                    BinaryOperator::Addition => IndexFormula::Add(left, right),
                    BinaryOperator::Subtraction => IndexFormula::Sub(left, right),
                    BinaryOperator::Multiplication => IndexFormula::Mul(left, right),
                    BinaryOperator::Division => IndexFormula::Div(left, right),
                    BinaryOperator::Remainder => IndexFormula::Rem(left, right),
                    _ => return None,
                })
            }
            Expression::UnaryExpression(unary)
                if unary.operator == UnaryOperator::UnaryNegation =>
            {
                Some(IndexFormula::Neg(Box::new(self.lower_index_formula(
                    &unary.argument,
                    index,
                    locals,
                    next,
                )?)))
            }
            Expression::CallExpression(call) if call.arguments.len() == 1 => {
                let Expression::StaticMemberExpression(member) = &call.callee else {
                    return None;
                };
                if !matches!(&member.object, Expression::Identifier(id) if id.name == "Math") {
                    return None;
                }
                let operand = Box::new(self.lower_index_formula(
                    call.arguments[0].as_expression()?,
                    index,
                    locals,
                    next,
                )?);
                match member.property.name.as_str() {
                    "floor" => Some(IndexFormula::Floor(operand)),
                    "sqrt" => Some(IndexFormula::Sqrt(operand)),
                    _ => None,
                }
            }
            _ => self
                .eval_static(expression)
                .and_then(|value| value.as_f64().filter(|value| value.is_finite()))
                .map(IndexFormula::Constant),
        }
    }
}
