//! Small AST/constant helpers shared across lowering domains.

use super::*;

pub(super) fn array_items<'a>(expression: &'a Expression<'a>) -> Option<Vec<&'a Expression<'a>>> {
    let Expression::ArrayExpression(array) = peel_expr(expression) else {
        return None;
    };
    array
        .elements
        .iter()
        .map(|element| match element {
            ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_) => None,
            value => value.as_expression(),
        })
        .collect()
}

pub(super) fn static_number(expression: &Expression<'_>) -> Option<f64> {
    match strip_parens(expression) {
        Expression::NumericLiteral(value) if value.value.is_finite() => Some(value.value),
        Expression::UnaryExpression(value)
            if value.operator == oxc::ast::ast::UnaryOperator::UnaryNegation =>
        {
            static_number(&value.argument).map(|value| -value)
        }
        _ => None,
    }
}

pub(super) fn static_motion_value(expression: &Expression<'_>) -> Option<MotionValue> {
    match strip_parens(expression) {
        Expression::NumericLiteral(value) if value.value.is_finite() => {
            Some(MotionValue::Number(value.value))
        }
        Expression::UnaryExpression(value)
            if value.operator == oxc::ast::ast::UnaryOperator::UnaryNegation =>
        {
            static_number(expression).map(MotionValue::Number)
        }
        Expression::StringLiteral(value) => Some(motion_value_from_string(value.value.as_str())),
        Expression::BooleanLiteral(value) => Some(MotionValue::Bool(value.value)),
        _ => None,
    }
}

pub(super) fn motion_value_from_string(value: &str) -> MotionValue {
    Length2::parse(value)
        .map(MotionValue::Length2)
        .or_else(|| Length::parse(value).map(MotionValue::Length))
        .or_else(|| Angle::parse(value).map(MotionValue::Angle))
        .or_else(|| Rgba::parse(value).map(MotionValue::Color))
        .unwrap_or_else(|| MotionValue::Str(value.to_string()))
}

pub(super) fn is_deterministic_math_alias(expression: &Expression<'_>) -> bool {
    if matches!(
        strip_parens(expression),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Remainder
    ) {
        return true;
    }
    let Expression::CallExpression(call) = strip_parens(expression) else {
        return false;
    };
    match &call.callee {
        Expression::Identifier(identifier) => {
            matches!(identifier.name.as_str(), "deg" | "rad" | "pow")
        }
        Expression::StaticMemberExpression(member) => {
            matches!(
                &member.object,
                Expression::Identifier(object) if object.name == "Math"
            ) && matches!(
                member.property.name.as_str(),
                "sqrt"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "atan2"
                    | "pow"
                    | "floor"
                    | "ceil"
                    | "round"
                    | "trunc"
            )
        }
        _ => false,
    }
}

pub(super) fn parse_svg_path(data: &str) -> Option<(Vec<PathVerb>, Vec<Point>)> {
    let path = kurbo::BezPath::from_svg(data).ok()?;
    let mut verbs = Vec::new();
    let mut points = Vec::new();
    for element in path.elements() {
        match *element {
            kurbo::PathEl::MoveTo(point) => {
                verbs.push(PathVerb::Move);
                points.push(Point::new(point.x, point.y));
            }
            kurbo::PathEl::LineTo(point) => {
                verbs.push(PathVerb::Line);
                points.push(Point::new(point.x, point.y));
            }
            kurbo::PathEl::QuadTo(control, point) => {
                verbs.push(PathVerb::Quad);
                points.extend([
                    Point::new(control.x, control.y),
                    Point::new(point.x, point.y),
                ]);
            }
            kurbo::PathEl::CurveTo(control_1, control_2, point) => {
                verbs.push(PathVerb::Cubic);
                points.extend([
                    Point::new(control_1.x, control_1.y),
                    Point::new(control_2.x, control_2.y),
                    Point::new(point.x, point.y),
                ]);
            }
            kurbo::PathEl::ClosePath => verbs.push(PathVerb::Close),
        }
    }
    (!verbs.is_empty()
        && points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()))
    .then_some((verbs, points))
}

pub(super) fn parse_path_data(data: &str) -> Option<PathData> {
    let (verbs, points) = parse_svg_path(data)?;
    PathData::new(verbs, points).ok()
}

/// Map easing names directly to IR tags. Cubic-bezier x coordinates must be in [0,1] for monotonic
/// inversion; y coordinates may overshoot.
pub(super) fn parse_easing(name: &str) -> Option<MotionEasing> {
    match name.trim() {
        "linear" => Some(MotionEasing::Linear),
        "ease" => Some(MotionEasing::Ease),
        "easeIn" => Some(MotionEasing::EaseIn),
        "easeOut" => Some(MotionEasing::EaseOut),
        "easeInOut" => Some(MotionEasing::EaseInOut),
        "exp" => Some(MotionEasing::Exp),
        other => {
            let args = other.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
            let mut p = [0.0_f64; 4];
            let mut seen = 0usize;
            for part in args.split(',') {
                if seen == p.len() {
                    return None;
                }
                p[seen] = part.trim().parse().ok()?;
                seen += 1;
            }
            if seen != p.len() || !p.iter().all(|value| value.is_finite()) {
                return None;
            }
            ((0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[2]))
                .then_some(MotionEasing::CubicBezier { p })
        }
    }
}

pub(super) fn strip_parens<'a>(mut expression: &'a Expression<'a>) -> &'a Expression<'a> {
    while let Expression::ParenthesizedExpression(parenthesized) = expression {
        expression = &parenthesized.expression;
    }
    expression
}

/// Peel parens and TypeScript wrappers (`as const`, satisfies, non-null, assertion)
/// so array collectors see the underlying expression.
pub(super) fn peel_expr<'a>(mut expression: &'a Expression<'a>) -> &'a Expression<'a> {
    loop {
        expression = strip_parens(expression);
        match expression {
            Expression::TSAsExpression(inner) => expression = &inner.expression,
            Expression::TSSatisfiesExpression(inner) => expression = &inner.expression,
            Expression::TSTypeAssertion(inner) => expression = &inner.expression,
            Expression::TSNonNullExpression(inner) => expression = &inner.expression,
            _ => return expression,
        }
    }
}

pub(super) fn as_map_call<'a>(
    expression: &'a Expression<'a>,
) -> Option<&'a oxc::ast::ast::CallExpression<'a>> {
    let Expression::CallExpression(call) = peel_expr(expression) else {
        return None;
    };
    match &call.callee {
        Expression::StaticMemberExpression(member) if member.property.name == "map" => Some(call),
        _ => None,
    }
}

pub(super) fn record_module_const_inits<'s>(
    statement: &'s Statement<'s>,
    out: &mut BTreeMap<String, &'s Expression<'s>>,
) {
    let declaration = match statement {
        Statement::VariableDeclaration(declaration) => declaration.as_ref(),
        Statement::ExportDeclaration(export) => match &export.declaration {
            Declaration::VariableDeclaration(declaration) => declaration.as_ref(),
            _ => return,
        },
        _ => return,
    };
    if !declaration.kind.is_const() {
        return;
    }
    for declarator in &declaration.declarations {
        if let Some(name) = declarator.id.get_identifier_name()
            && let Some(init) = &declarator.init
        {
            out.insert(name.to_string(), init);
        }
    }
}

pub(super) fn prelude_peeled_const_text(
    source: &str,
    declaration: &oxc::ast::ast::VariableDeclaration<'_>,
) -> Option<String> {
    if !declaration.kind.is_const() || declaration.declarations.len() != 1 {
        return None;
    }
    let declarator = &declaration.declarations[0];
    let name = declarator.id.get_identifier_name()?;
    let init = declarator.init.as_ref()?;
    let peeled = peel_expr(init);
    if std::ptr::eq(peeled as *const _, init as *const _) {
        return None;
    }
    Some(format!(
        "const {name} = {};",
        &source[peeled.span().start as usize..peeled.span().end as usize]
    ))
}

pub(super) fn camel_to_kebab(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_uppercase() {
            output.push('-');
            output.push(character.to_ascii_lowercase());
        } else {
            output.push(character);
        }
    }
    output
}
