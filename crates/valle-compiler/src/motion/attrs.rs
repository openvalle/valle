//! Typed JSX attributes, paint/path conversion, and GeometryBatch fields.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn attr_paint_value(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<PaintValue> {
        let expression = match value {
            Some(JSXAttributeValue::StringLiteral(value)) => {
                return self.static_solid_paint(value.value.as_str(), span, name);
            }
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                container.expression.as_expression()?
            }
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("path {name} must be a color or gradient expression"),
                );
                return None;
            }
        };

        if let Some(value) = self.eval_static(expression) {
            if let Some(text) = value.as_str() {
                return self.static_solid_paint(text, span, name);
            }
            if let Some(paint) = static_paint_from_json(&value) {
                return Some(paint);
            }
        }

        if let Expression::CallExpression(call) = strip_parens(expression)
            && let Expression::Identifier(callee) = &call.callee
            && matches!(
                callee.name.as_str(),
                "linearGradient" | "radialGradient" | "conicGradient"
            )
        {
            return self.lower_gradient_call(call, span);
        }

        self.lower_expr(expression).map(|expr| PaintValue::Solid {
            color: ColorValue::Expr { expr },
        })
    }

    pub(super) fn static_solid_paint(
        &mut self,
        value: &str,
        span: Span,
        name: &str,
    ) -> Option<PaintValue> {
        if value == "none" {
            return None;
        }
        Rgba::parse(value)
            .map(|value| PaintValue::Solid {
                color: ColorValue::Static { value },
            })
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("path {name} `{value}` is not a color or gradient"),
                );
                None
            })
    }

    pub(super) fn lower_gradient_call(
        &mut self,
        call: &oxc::ast::ast::CallExpression<'_>,
        span: Span,
    ) -> Option<PaintValue> {
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if !(3..=4).contains(&call.arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "{} requires geometry, stops, and optional spread",
                    callee.name
                ),
            );
            return None;
        }
        let spread = if let Some(argument) = call.arguments.get(3) {
            self.gradient_spread(argument.as_expression()?, argument.span())?
        } else {
            SpreadMode::Pad
        };
        let first = call.arguments[0].as_expression()?;
        let second = call.arguments[1].as_expression()?;
        let stops = self.lower_gradient_stops(call.arguments[2].as_expression()?, span)?;
        match callee.name.as_str() {
            "linearGradient" => Some(PaintValue::Linear {
                start: self.point_binding(first, "linear gradient start")?,
                end: self.point_binding(second, "linear gradient end")?,
                stops,
                spread,
            }),
            "radialGradient" => Some(PaintValue::Radial {
                center: self.point_binding(first, "radial gradient center")?,
                radius: self
                    .number_binding(second, "radial gradient radius", |value| value > 0.0)?,
                stops,
                spread,
            }),
            "conicGradient" => Some(PaintValue::Conic {
                center: self.point_binding(first, "conic gradient center")?,
                start_angle: self.number_binding(second, "conic gradient start angle", |_| true)?,
                stops,
                spread,
            }),
            _ => None,
        }
    }

    pub(super) fn gradient_spread(
        &mut self,
        expression: &Expression<'_>,
        span: Span,
    ) -> Option<SpreadMode> {
        let value = self
            .eval_static(expression)
            .and_then(|value| value.as_str().map(str::to_string));
        match value.as_deref() {
            Some("pad") => Some(SpreadMode::Pad),
            Some("repeat") => Some(SpreadMode::Repeat),
            Some("reflect") => Some(SpreadMode::Reflect),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "gradient spread must be the static string pad, repeat, or reflect",
                );
                None
            }
        }
    }

    pub(super) fn lower_gradient_stops(
        &mut self,
        expression: &Expression<'_>,
        span: Span,
    ) -> Option<Vec<GradientStopValue>> {
        if let Some(value) = self.eval_static(expression)
            && let Some(stops) = static_gradient_stops_from_json(&value)
        {
            return Some(stops);
        }
        let Some(items) = array_items(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "gradient stops must be a fixed array of gradientStop(offset, color)",
            );
            return None;
        };
        if !(2..=64).contains(&items.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "gradient requires 2..=64 fixed stops",
            );
            return None;
        }
        items
            .into_iter()
            .map(|item| self.lower_gradient_stop(item))
            .collect()
    }

    pub(super) fn lower_gradient_stop(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<GradientStopValue> {
        if let Some(value) = self.eval_static(expression)
            && let Some(stop) = static_gradient_stop_from_json(&value)
        {
            return Some(stop);
        }
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "gradient stop must call gradientStop(offset, color)",
            );
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if callee.name != "gradientStop" || call.arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "gradient stop must call gradientStop(offset, color)",
            );
            return None;
        }
        Some(GradientStopValue {
            offset: self.number_binding(
                call.arguments[0].as_expression()?,
                "gradient stop offset",
                |value| (0.0..=1.0).contains(&value),
            )?,
            color: self.color_binding(call.arguments[1].as_expression()?, "gradient stop color")?,
        })
    }

    pub(super) fn point_binding(
        &mut self,
        expression: &Expression<'_>,
        name: &str,
    ) -> Option<PointValue> {
        if let Some(value) = self.eval_static(expression)
            && let Some(MotionValue::Point(value)) = motion_value_from_json(&value)
        {
            return Some(PointValue::Static { value });
        }
        self.lower_expr(expression)
            .map(|expr| PointValue::Expr { expr })
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("{name} must produce Point"),
                );
                None
            })
    }

    pub(super) fn number_binding(
        &mut self,
        expression: &Expression<'_>,
        name: &str,
        static_valid: impl Fn(f64) -> bool,
    ) -> Option<NumberValue> {
        if let Some(value) = self
            .eval_static(expression)
            .and_then(|value| value.as_f64())
        {
            if value.is_finite() && static_valid(value) {
                return Some(NumberValue::Static { value });
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!("{name} has an invalid static value"),
            );
            return None;
        }
        self.lower_expr(expression)
            .map(|expr| NumberValue::Expr { expr })
    }

    pub(super) fn color_binding(
        &mut self,
        expression: &Expression<'_>,
        name: &str,
    ) -> Option<ColorValue> {
        if let Some(value) = self.eval_static(expression)
            && let Some(MotionValue::Color(value)) = motion_value_from_json(&value)
        {
            return Some(ColorValue::Static { value });
        }
        self.lower_expr(expression)
            .map(|expr| ColorValue::Expr { expr })
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("{name} must produce Color"),
                );
                None
            })
    }

    pub(super) fn attr_path_value(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
    ) -> Option<PathValue> {
        let static_path = |value: &serde_json::Value| match value {
            serde_json::Value::String(value) => parse_path_data(value),
            _ => match motion_value_from_json(value) {
                Some(MotionValue::PathData(value)) => Some(value),
                _ => None,
            },
        };
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => parse_path_data(value.value.as_str())
                .map(|value| PathValue::Static { value })
                .or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "Path d is not valid finite SVG path data",
                    );
                    None
                }),
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                let expression = container.expression.as_expression()?;
                // Reject spreads and holes even when QuickJS could evaluate every coordinate
                // statically.
                if let Expression::CallExpression(call) = peel_expr(expression)
                    && matches!(&call.callee, Expression::Identifier(id) if id.name == "line")
                    && let Some(arg) = call.arguments.first().and_then(Argument::as_expression)
                    && self.array_shape_illegal(arg, "line(points)")
                {
                    return None;
                }
                if let Some(value) = self.eval_static(expression)
                    && let Some(value) = static_path(&value)
                {
                    return Some(PathValue::Static { value });
                }
                let expr = self.lower_expr(expression)?;
                let policy = geometry_eval_policy(&self.expr_arena.values, expr).or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "Path d expression must produce PathData",
                    );
                    None
                })?;
                Some(PathValue::Expr { expr, policy })
            }
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "Path d must be an SVG string or PathData expression",
                );
                None
            }
        }
    }

    pub(super) fn attr_number_expr(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<ExprId> {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => value
                .value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| {
                    self.push(
                        Expr::Const {
                            value: MotionValue::Number(value),
                        },
                        span,
                    )
                })
                .or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("SVG shape attribute `{name}` must be a finite number"),
                    );
                    None
                }),
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                let expression = container.expression.as_expression()?;
                self.lower_expr(expression).or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("SVG shape attribute `{name}` must be a numeric expression"),
                    );
                    None
                })
            }
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("SVG shape attribute `{name}` must be a number"),
                );
                None
            }
        }
    }

    pub(super) fn attr_shape_points(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
    ) -> Option<Vec<ExprId>> {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => {
                let numbers = value
                    .value
                    .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
                    .filter(|part| !part.is_empty())
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
                    .filter(|items| items.len() % 2 == 0 && items.iter().all(|v| v.is_finite()));
                let Some(numbers) = numbers else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "SVG points must be finite x,y pairs",
                    );
                    return None;
                };
                Some(
                    numbers
                        .chunks_exact(2)
                        .map(|pair| {
                            let x = self.push(
                                Expr::Const {
                                    value: MotionValue::Number(pair[0]),
                                },
                                span,
                            );
                            let y = self.push(
                                Expr::Const {
                                    value: MotionValue::Number(pair[1]),
                                },
                                span,
                            );
                            self.push(Expr::MakePoint { x, y }, span)
                        })
                        .collect(),
                )
            }
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                let expression = container.expression.as_expression()?;
                self.collect_array_elements(expression, "SVG points")
            }
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "SVG points must be a static string or fixed-length Point array",
                );
                None
            }
        }
    }

    pub(super) fn lower_svg_shape(
        &mut self,
        shape: &str,
        numbers: &BTreeMap<String, ExprId>,
        authored_points: Option<Vec<ExprId>>,
        span: Span,
    ) -> Option<PathValue> {
        let constant = |this: &mut Self, value: f64| {
            this.push(
                Expr::Const {
                    value: MotionValue::Number(value),
                },
                span,
            )
        };
        let number = |this: &mut Self, name: &str, default: Option<f64>| {
            numbers
                .get(name)
                .copied()
                .or_else(|| default.map(|value| constant(this, value)))
        };
        let required = |this: &mut Self, name: &str| {
            number(this, name, None).or_else(|| {
                this.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("<{shape}> requires `{name}`"),
                );
                None
            })
        };
        let add = |this: &mut Self, lhs, rhs| this.push(Expr::Add { lhs, rhs }, span);
        let sub = |this: &mut Self, lhs, rhs| this.push(Expr::Sub { lhs, rhs }, span);
        let mul = |this: &mut Self, lhs, rhs| this.push(Expr::Mul { lhs, rhs }, span);
        let point = |this: &mut Self, x, y| this.push(Expr::MakePoint { x, y }, span);

        let expr = match shape {
            "line" => {
                let x1 = number(self, "x1", Some(0.0))?;
                let y1 = number(self, "y1", Some(0.0))?;
                let x2 = number(self, "x2", Some(0.0))?;
                let y2 = number(self, "y2", Some(0.0))?;
                let p1 = point(self, x1, y1);
                let p2 = point(self, x2, y2);
                self.push(
                    Expr::PathTemplate {
                        verbs: vec![valle_draw::PathVerb::Move, valle_draw::PathVerb::Line],
                        points: vec![p1, p2],
                    },
                    span,
                )
            }
            "polyline" | "polygon" => {
                let points = authored_points.or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("<{shape}> requires `points`"),
                    );
                    None
                })?;
                let min = if shape == "polygon" { 3 } else { 2 };
                if points.len() < min {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("<{shape}> requires at least {min} points"),
                    );
                    return None;
                }
                let mut verbs = vec![valle_draw::PathVerb::Move];
                verbs.extend(std::iter::repeat_n(
                    valle_draw::PathVerb::Line,
                    points.len() - 1,
                ));
                if shape == "polygon" {
                    verbs.push(valle_draw::PathVerb::Close);
                }
                self.push(Expr::PathTemplate { verbs, points }, span)
            }
            "rect" => {
                let x = number(self, "x", Some(0.0))?;
                let y = number(self, "y", Some(0.0))?;
                let width = required(self, "width")?;
                let height = required(self, "height")?;
                let right = add(self, x, width);
                let bottom = add(self, y, height);
                let p0 = point(self, x, y);
                let p1 = point(self, right, y);
                let p2 = point(self, right, bottom);
                let p3 = point(self, x, bottom);
                self.push(
                    Expr::PathTemplate {
                        verbs: vec![
                            valle_draw::PathVerb::Move,
                            valle_draw::PathVerb::Line,
                            valle_draw::PathVerb::Line,
                            valle_draw::PathVerb::Line,
                            valle_draw::PathVerb::Close,
                        ],
                        points: vec![p0, p1, p2, p3],
                    },
                    span,
                )
            }
            "circle" => {
                let cx = number(self, "cx", Some(0.0))?;
                let cy = number(self, "cy", Some(0.0))?;
                let radius = required(self, "r")?;
                let center = point(self, cx, cy);
                let start_angle = constant(self, 0.0);
                let end_angle = constant(self, core::f64::consts::TAU);
                self.push(
                    Expr::PathArc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                    },
                    span,
                )
            }
            "ellipse" => {
                let cx = number(self, "cx", Some(0.0))?;
                let cy = number(self, "cy", Some(0.0))?;
                let rx = required(self, "rx")?;
                let ry = required(self, "ry")?;
                let kappa = constant(self, 0.552_284_749_830_793_6);
                let kx = mul(self, rx, kappa);
                let ky = mul(self, ry, kappa);
                let left = sub(self, cx, rx);
                let right = add(self, cx, rx);
                let top = sub(self, cy, ry);
                let bottom = add(self, cy, ry);
                let cx_minus_kx = sub(self, cx, kx);
                let cx_plus_kx = add(self, cx, kx);
                let cy_minus_ky = sub(self, cy, ky);
                let cy_plus_ky = add(self, cy, ky);
                let points = vec![
                    point(self, right, cy),
                    point(self, right, cy_plus_ky),
                    point(self, cx_plus_kx, bottom),
                    point(self, cx, bottom),
                    point(self, cx_minus_kx, bottom),
                    point(self, left, cy_plus_ky),
                    point(self, left, cy),
                    point(self, left, cy_minus_ky),
                    point(self, cx_minus_kx, top),
                    point(self, cx, top),
                    point(self, cx_plus_kx, top),
                    point(self, right, cy_minus_ky),
                    point(self, right, cy),
                ];
                self.push(
                    Expr::PathTemplate {
                        verbs: vec![
                            valle_draw::PathVerb::Move,
                            valle_draw::PathVerb::Cubic,
                            valle_draw::PathVerb::Cubic,
                            valle_draw::PathVerb::Cubic,
                            valle_draw::PathVerb::Cubic,
                            valle_draw::PathVerb::Close,
                        ],
                        points,
                    },
                    span,
                )
            }
            _ => return None,
        };
        let policy = geometry_eval_policy(&self.expr_arena.values, expr)?;
        Some(PathValue::Expr { expr, policy })
    }

    pub(super) fn attr_rect_value(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<RectValue> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must be a Rect expression"),
            );
            return None;
        };
        let expression = container.expression.as_expression()?;
        if let Some(value) = self.eval_static(expression)
            && let Some(MotionValue::Rect(value)) = motion_value_from_json(&value)
        {
            if value.x.is_finite()
                && value.y.is_finite()
                && value.width.is_finite()
                && value.height.is_finite()
                && value.width > 0.0
                && value.height > 0.0
            {
                return Some(RectValue::Static { value });
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must have finite coordinates and positive size"),
            );
            return None;
        }
        self.lower_expr(expression)
            .map(|expr| RectValue::Expr { expr })
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("{name} must produce Rect"),
                );
                None
            })
    }

    pub(super) fn attr_static_json(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<serde_json::Value> {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => {
                Some(serde_json::Value::String(value.value.to_string()))
            }
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.eval_static(expression))
                .or_else(|| {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        format!("GeometryBatch `{name}` must be prepare-time data"),
                    );
                    None
                }),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("GeometryBatch `{name}` needs an explicit value"),
                );
                None
            }
        }
    }

    pub(super) fn attr_static_strings(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<Vec<String>> {
        let value = self.attr_static_json(value, span, name)?;
        let items = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        items
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("GeometryBatch `{name}` must be a string or string array"),
                );
                None
            })
    }

    pub(super) fn attr_batch_sizes(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
    ) -> Option<(Vec<Point>, Option<BatchPointField>)> {
        let expression = self.attr_batch_expression(value, span, "sizes")?;
        if self.is_batch_field_call(expression) {
            let field = self.batch_field_parts(expression, "sizes")?;
            let from = self.static_batch_points(field.from, span, "sizes.from", true)?;
            let to = self.static_batch_points(field.to, span, "sizes.to", true)?;
            let progress = self.lower_expr(field.progress)?;
            return Some((
                from,
                Some(BatchPointField {
                    to,
                    progress,
                    stagger: field.stagger,
                }),
            ));
        }
        let values = self.static_batch_points(expression, span, "sizes", true)?;
        Some((values, None))
    }

    pub(super) fn static_batch_points(
        &mut self,
        expression: &'s Expression<'s>,
        span: Span,
        name: &str,
        allow_scalar: bool,
    ) -> Option<Vec<Point>> {
        let value = self.eval_static(expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                format!("GeometryBatch `{name}` must be prepare-time data"),
            );
            None
        })?;
        let items = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        items
            .iter()
            .map(|value| {
                allow_scalar
                    .then(|| value.as_f64().map(|value| Point::new(value, value)))
                    .flatten()
                    .or_else(|| match motion_value_from_json(value) {
                        Some(MotionValue::Point(point)) => Some(point),
                        _ => None,
                    })
            })
            .collect::<Option<Vec<_>>>()
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "GeometryBatch `{name}` must be {}",
                        if allow_scalar {
                            "a number/point or an array of them"
                        } else {
                            "a point array"
                        }
                    ),
                );
                None
            })
    }

    pub(super) fn attr_batch_fills(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
    ) -> Option<(Vec<Rgba>, Option<BatchColorField>)> {
        if let Some(JSXAttributeValue::StringLiteral(literal)) = value {
            let value = serde_json::Value::String(literal.value.to_string());
            return match motion_value_from_json(&value) {
                Some(MotionValue::Color(color)) => Some((vec![color], None)),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "GeometryBatch `fills` must be a color or color array",
                    );
                    None
                }
            };
        }
        let expression = self.attr_batch_expression(value, span, "fills")?;
        if self.is_batch_field_call(expression) {
            let field = self.batch_field_parts(expression, "fills")?;
            let from = self.static_batch_colors(field.from, span, "fills.from")?;
            let to = self.static_batch_colors(field.to, span, "fills.to")?;
            let progress = self.lower_expr(field.progress)?;
            return Some((
                from,
                Some(BatchColorField {
                    to,
                    progress,
                    stagger: field.stagger,
                }),
            ));
        }
        let values = self.static_batch_colors(expression, span, "fills")?;
        Some((values, None))
    }

    pub(super) fn static_batch_colors(
        &mut self,
        expression: &'s Expression<'s>,
        span: Span,
        name: &str,
    ) -> Option<Vec<Rgba>> {
        let value = self.eval_static(expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                format!("GeometryBatch `{name}` must be prepare-time data"),
            );
            None
        })?;
        let items = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        items
            .iter()
            .map(|value| match motion_value_from_json(value) {
                Some(MotionValue::Color(color)) => Some(color),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("GeometryBatch `{name}` must be a color or color array"),
                );
                None
            })
    }

    pub(super) fn attr_batch_opacities(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
    ) -> Option<(Vec<f64>, Option<BatchNumberField>)> {
        let expression = self.attr_batch_expression(value, span, "opacities")?;
        if self.is_batch_field_call(expression) {
            let field = self.batch_field_parts(expression, "opacities")?;
            let from = self.static_batch_numbers(field.from, span, "opacities.from")?;
            let to = self.static_batch_numbers(field.to, span, "opacities.to")?;
            let progress = self.lower_expr(field.progress)?;
            return Some((
                from,
                Some(BatchNumberField {
                    to,
                    progress,
                    stagger: field.stagger,
                }),
            ));
        }
        let values = self.static_batch_numbers(expression, span, "opacities")?;
        Some((values, None))
    }

    pub(super) fn static_batch_numbers(
        &mut self,
        expression: &'s Expression<'s>,
        span: Span,
        name: &str,
    ) -> Option<Vec<f64>> {
        let value = self.eval_static(expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                format!("GeometryBatch `{name}` must be prepare-time data"),
            );
            None
        })?;
        let items = value.as_array().cloned().unwrap_or_else(|| vec![value]);
        items
            .iter()
            .map(|value| value.as_f64().filter(|value| value.is_finite()))
            .collect::<Option<Vec<_>>>()
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("GeometryBatch `{name}` must be a finite number or number array"),
                );
                None
            })
    }

    pub(super) fn attr_batch_positions(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
    ) -> Option<(BatchPositions, Option<BatchPointField>)> {
        let expression = self.attr_batch_expression(value, span, "positions")?;
        if self.is_batch_field_call(expression) {
            let field = self.batch_field_parts(expression, "positions")?;
            let from = self.static_batch_points(field.from, span, "positions.from", false)?;
            let to = self.static_batch_points(field.to, span, "positions.to", false)?;
            let progress = self.lower_expr(field.progress)?;
            return Some((
                BatchPositions::Static { values: from },
                Some(BatchPointField {
                    to,
                    progress,
                    stagger: field.stagger,
                }),
            ));
        }
        if let Expression::CallExpression(call) = strip_parens(expression)
            && matches!(&call.callee, Expression::Identifier(callee) if callee.name == "particles")
        {
            if call.arguments.len() != 3 {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    call.span(),
                    "particles(frame, ctx.fps, options) takes exactly three arguments",
                );
                return None;
            }
            let fps = call.arguments[1].as_expression()?;
            if self.src(fps) != "ctx.fps" {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    fps.span(),
                    "particles' second argument must be exactly `ctx.fps`",
                );
                return None;
            }
            let frame = self.lower_expr(call.arguments[0].as_expression()?)?;
            let options_expression = call.arguments[2].as_expression()?;
            let value = self.eval_static(options_expression).or_else(|| {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    options_expression.span(),
                    "particles options must be a prepare-time object",
                );
                None
            })?;
            let object = value.as_object()?;
            let number = |name: &str| object.get(name)?.as_f64().filter(|v| v.is_finite());
            let integer = |name: &str| {
                let value = number(name)?;
                (value.fract() == 0.0 && value >= 0.0 && value <= 9_007_199_254_740_991.0)
                    .then_some(value as u64)
            };
            let typed = |name: &str| object.get(name).and_then(motion_value_from_json);
            let emitter = match typed("emitter") {
                Some(MotionValue::Rect(value)) => value,
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        options_expression.span(),
                        "particles emitter must be rect(x,y,width,height)",
                    );
                    return None;
                }
            };
            let gravity = match object.get("gravity").and_then(motion_value_from_json) {
                Some(MotionValue::Point(value)) => value,
                None => Point::new(0.0, 0.0),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        options_expression.span(),
                        "particles gravity must be point(x,y)",
                    );
                    return None;
                }
            };
            let birth_interval = object
                .get("birth")
                .and_then(serde_json::Value::as_object)
                .and_then(|birth| birth.get("interval"))
                .and_then(serde_json::Value::as_f64);
            let range = |axis: &str| -> Option<[f64; 2]> {
                let values = object.get("velocity")?.as_object()?.get(axis)?.as_array()?;
                if values.len() != 2 {
                    return None;
                }
                Some([values[0].as_f64()?, values[1].as_f64()?])
            };
            let (
                Some(seed),
                Some(count),
                Some(birth_interval),
                Some(lifetime),
                Some(velocity_x),
                Some(velocity_y),
            ) = (
                integer("seed"),
                integer("count"),
                birth_interval,
                number("lifetime"),
                range("x"),
                range("y"),
            )
            else {
                self.illegal(DiagCode::GrammarForbidden, options_expression.span(), "particles needs integer seed/count, birth.interval, lifetime, and velocity.x/y two-number ranges");
                return None;
            };
            return Some((
                BatchPositions::Particles {
                    frame,
                    spec: ParticleSpec {
                        seed,
                        count: u32::try_from(count).unwrap_or(u32::MAX),
                        emitter,
                        birth_interval,
                        lifetime,
                        velocity_x,
                        velocity_y,
                        gravity,
                        looping: object
                            .get("loop")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    },
                },
                None,
            ));
        }
        let points = self.static_batch_points(expression, span, "positions", false)?;
        Some((BatchPositions::Static { values: points }, None))
    }

    pub(super) fn attr_batch_expression(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
        name: &str,
    ) -> Option<&'s Expression<'s>> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("GeometryBatch `{name}` must be an expression"),
            );
            return None;
        };
        container.expression.as_expression().or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("GeometryBatch `{name}` must be an expression"),
            );
            None
        })
    }

    pub(super) fn is_batch_field_call(&self, expression: &Expression<'s>) -> bool {
        matches!(
            strip_parens(expression),
            Expression::CallExpression(call)
                if matches!(&call.callee, Expression::Identifier(callee) if callee.name == "field")
        )
    }

    pub(super) fn batch_field_parts(
        &mut self,
        expression: &'s Expression<'s>,
        name: &str,
    ) -> Option<BatchFieldParts<'s>> {
        let Expression::CallExpression(call) = strip_parens(expression) else {
            return None;
        };
        if call.arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!("field(...) for GeometryBatch `{name}` takes exactly one object"),
            );
            return None;
        }
        let Some(Expression::ObjectExpression(object)) = call
            .arguments
            .first()
            .and_then(Argument::as_expression)
            .map(strip_parens)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                format!("field(...) for GeometryBatch `{name}` requires an object literal"),
            );
            return None;
        };

        let mut from = None;
        let mut to = None;
        let mut progress = None;
        let mut stagger = 0.0;
        let mut saw_stagger = false;
        let mut saw = BTreeSet::new();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "GeometryBatch field objects cannot use spread properties",
                );
                continue;
            };
            let Some(key) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    "GeometryBatch field keys must be static",
                );
                continue;
            };
            if !saw.insert(key.clone()) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("GeometryBatch field has duplicate `{key}`"),
                );
                continue;
            }
            match key.as_str() {
                "from" => from = Some(&property.value),
                "to" => to = Some(&property.value),
                "progress" => progress = Some(&property.value),
                "stagger" => {
                    saw_stagger = true;
                    stagger = self
                        .eval_static(&property.value)
                        .and_then(|value| value.as_f64())
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or_else(|| {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                property.value.span(),
                                "GeometryBatch field stagger must be a prepare-time finite number >= 0",
                            );
                            f64::NAN
                        });
                }
                _ => self.illegal(
                    DiagCode::UnknownProp,
                    property.key.span(),
                    format!("GeometryBatch field does not admit `{key}`"),
                ),
            }
        }
        if saw_stagger && !stagger.is_finite() {
            return None;
        }
        let (Some(from), Some(to), Some(progress)) = (from, to, progress) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                object.span(),
                format!("GeometryBatch `{name}` field requires from, to, and progress"),
            );
            return None;
        };
        Some(BatchFieldParts {
            from,
            to,
            progress,
            stagger,
        })
    }

    /// Fold a numeric field when static; otherwise lower it to a per-frame expression.
    pub(super) fn expr_number_value(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<NumberValue> {
        if let Some(value) = self
            .eval_static(expression)
            .and_then(|value| value.as_f64())
            && value.is_finite()
        {
            return Some(NumberValue::Static { value });
        }
        Some(NumberValue::Expr {
            expr: self.lower_expr(expression)?,
        })
    }

    /// Fold a point object or point(x, y) when static; otherwise lower its expression.
    pub(super) fn expr_point_value(
        &mut self,
        expression: &'s Expression<'s>,
        span: Span,
    ) -> Option<PointValue> {
        if let Some(serde_json::Value::Object(object)) = self.eval_static(expression) {
            let x = object.get("x").and_then(serde_json::Value::as_f64);
            let y = object.get("y").and_then(serde_json::Value::as_f64);
            if let (Some(x), Some(y)) = (x, y)
                && x.is_finite()
                && y.is_finite()
            {
                return Some(PointValue::Static {
                    value: valle_draw::Point::new(x, y),
                });
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "a static point needs finite `x` and `y`",
            );
            return None;
        }
        Some(PointValue::Expr {
            expr: self.lower_expr(expression)?,
        })
    }

    pub(super) fn attr_number_value(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<NumberValue> {
        let static_value = match value {
            Some(JSXAttributeValue::StringLiteral(value)) => value.value.parse::<f64>().ok(),
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.eval_static(expression))
                .and_then(|value| {
                    value
                        .as_f64()
                        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
                }),
            _ => None,
        };
        if let Some(value) = static_value {
            if value.is_finite() && (0.0..=1.0).contains(&value) {
                return Some(NumberValue::Static { value });
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must be a finite number in 0..=1"),
            );
            return None;
        }
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must be a number expression"),
            );
            return None;
        };
        let expr = self.lower_expr(container.expression.as_expression()?)?;
        Some(NumberValue::Expr { expr })
    }

    pub(super) fn attr_nonnegative_number_value(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<NumberValue> {
        let static_value = match value {
            Some(JSXAttributeValue::StringLiteral(value)) => value.value.parse::<f64>().ok(),
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.eval_static(expression))
                .and_then(|value| {
                    value
                        .as_f64()
                        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
                }),
            _ => None,
        };
        if let Some(value) = static_value {
            if value.is_finite() && value >= 0.0 {
                return Some(NumberValue::Static { value });
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must be a finite non-negative number"),
            );
            return None;
        }
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must be a number expression"),
            );
            return None;
        };
        let expr = self.lower_expr(container.expression.as_expression()?)?;
        Some(NumberValue::Expr { expr })
    }

    pub(super) fn attr_arrow_kind(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<ArrowKind> {
        match self.attr_static_string(value, span, name)?.as_str() {
            "none" => None,
            "triangle" => Some(ArrowKind::Triangle),
            "open" => Some(ArrowKind::Open),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("{name} must be none, triangle, or open"),
                );
                None
            }
        }
    }

    pub(super) fn attr_static_string(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
        name: &str,
    ) -> Option<String> {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => Some(value.value.to_string()),
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.eval_static(expression))
                .and_then(|value| match value {
                    serde_json::Value::String(value) => Some(value),
                    serde_json::Value::Number(value) => Some(value.to_string()),
                    _ => None,
                })
                .or_else(|| {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{name} must be a static string"),
                    );
                    None
                }),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("{name} must be a static string"),
                );
                None
            }
        }
    }
}
