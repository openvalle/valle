//! 2D/3D transform, transform-origin, and two-axis scale lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_perspective_origin_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        let (value, holes) = if let Some(value) = self
            .eval_static(expression)
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            (value, &[][..])
        } else {
            let Expression::TemplateLiteral(template) = strip_parens(expression) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "perspectiveOrigin must be a static string or closed two-axis template literal",
                );
                return None;
            };
            if template.quasis.len() != template.expressions.len() + 1 {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    template.span,
                    "malformed perspectiveOrigin template literal",
                );
                return None;
            }
            let mut value = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                let Some(cooked) = quasi.value.cooked else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        quasi.span,
                        "perspectiveOrigin template contains an invalid escape",
                    );
                    return None;
                };
                value.push_str(cooked.as_str());
                if index < template.expressions.len() {
                    value.push_str(&format!("__VALLE_HOLE_{index}__"));
                }
            }
            (value, &template.expressions[..])
        };
        let mut parts = value.split_whitespace();
        let x = parts.next();
        let y = parts.next();
        if parts.next().is_some() || x.is_none() || y.is_none() {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "perspectiveOrigin requires exactly two values, for example `50% 30%`",
            );
            return None;
        }
        let parse_axis = |token: &str, horizontal: bool| {
            let keyword = match (horizontal, token) {
                (true, "left") | (false, "top") => Some(0.0),
                (_, "center") => Some(50.0),
                (true, "right") | (false, "bottom") => Some(100.0),
                _ => None,
            };
            keyword
                .map(|value| Length {
                    value,
                    unit: valle_motion::value::LengthUnit::Percent,
                })
                .or_else(|| Length::parse(token))
        };
        self.extra_capabilities
            .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
        self.extra_capabilities
            .insert(CSS_3D_PERSPECTIVE_ORIGIN_CAPABILITY.to_owned());
        let mut lower_axis = |token: &str, horizontal: bool, axis: &str| {
            if let Some((index, suffix)) = transform_hole_parts(token) {
                let Some(value) = holes.get(index) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        "perspectiveOrigin template hole index is out of bounds",
                    );
                    return None;
                };
                let unit = match suffix {
                    "%" => "percent",
                    "px" => "px",
                    _ => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            expression.span(),
                            "dynamic perspectiveOrigin values require an explicit px or % suffix",
                        );
                        return None;
                    }
                };
                return Some(StyleBinding {
                    property: format!("motion-perspective-origin-{axis}-{unit}"),
                    value: StyleValue::Expr {
                        expr: self.lower_expr(value)?,
                    },
                });
            }
            let Some(value) = parse_axis(token, horizontal) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "perspectiveOrigin values must be CSS lengths, percentages, or matching position keywords",
                );
                return None;
            };
            Some(StyleBinding {
                property: format!("motion-perspective-origin-{axis}"),
                value: StyleValue::Static {
                    value: MotionValue::Length(value),
                },
            })
        };
        Some(vec![
            lower_axis(x.unwrap(), true, "x")?,
            lower_axis(y.unwrap(), false, "y")?,
        ])
    }

    pub(super) fn lower_transform_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        // Keep all 2D operations in authored order. Independent translate/rotate/scale
        // are separate CSS properties and must compose with this list after cascade.
        if let Some(serde_json::Value::String(value)) = self.eval_static(expression) {
            match valle_motion::style::parse_property("transform", &value) {
                Ok(_) => {
                    return Some(vec![StyleBinding {
                        property: "transform".into(),
                        value: StyleValue::Static {
                            value: MotionValue::Str(value),
                        },
                    }]);
                }
                Err(reason) => {
                    if let Some(parts) = parse_ordered_transform(&value)
                        && parts.iter().any(|(name, _)| is_css_3d_function(name))
                    {
                        self.extra_capabilities
                            .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
                        return self.lower_css_3d_transform_parts(expression.span(), parts, &[]);
                    }
                    self.style_diagnostic(expression.span(), None, reason);
                    return None;
                }
            }
        }
        if let Expression::TemplateLiteral(template) = strip_parens(expression) {
            let mut source = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                source.push_str(quasi.value.cooked?.as_str());
                if index < template.expressions.len() {
                    source.push_str(&format!("__VALLE_HOLE_{index}__"));
                }
            }
            if let Some(parts) = parse_ordered_transform(&source)
                && parts.iter().any(|(name, _)| is_css_3d_function(name))
            {
                self.extra_capabilities
                    .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
                return self.lower_css_3d_transform_parts(
                    template.span,
                    parts,
                    &template.expressions,
                );
            }
        }
        Some(vec![StyleBinding {
            property: "transform".into(),
            value: StyleValue::Expr {
                expr: self.lower_css_value_expression(expression)?,
            },
        }])
    }

    pub(super) fn lower_css_3d_transform_parts(
        &mut self,
        span: Span,
        parts: Vec<(String, String)>,
        holes: &[Expression<'s>],
    ) -> Option<Vec<StyleBinding>> {
        let mut bindings = Vec::new();
        for (name, argument) in parts {
            if name == "translate" {
                let arguments = argument.split(',').map(str::trim).collect::<Vec<_>>();
                if !(1..=2).contains(&arguments.len())
                    || arguments.iter().any(|value| value.is_empty())
                {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "translate requires one value or exactly two comma-separated values",
                    );
                    return None;
                }
                let values = [arguments[0], arguments.get(1).copied().unwrap_or("0")];
                for (axis, argument) in ["x", "y"].into_iter().zip(values) {
                    let (value, percent) = self.lower_css_3d_length_argument(
                        span,
                        &format!("translate{axis}"),
                        argument,
                        holes,
                        true,
                    )?;
                    bindings.push(StyleBinding {
                        property: if percent {
                            format!("motion-transform-3d-translate-{axis}-percent")
                        } else {
                            format!("motion-transform-3d-translate-{axis}")
                        },
                        value,
                    });
                }
                continue;
            }
            if matches!(name.as_str(), "translate3d" | "scale3d") {
                let Some(arguments) = split_transform_arguments(&argument, 3) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{name} requires exactly three comma-separated values"),
                    );
                    return None;
                };
                let (prefix, kind) = if name == "translate3d" {
                    ("translate", Css3dArgument::Length)
                } else {
                    ("scale", Css3dArgument::Number)
                };
                for (axis, argument) in ["x", "y", "z"].into_iter().zip(arguments) {
                    let (value, percent) = if prefix == "translate" {
                        self.lower_css_3d_length_argument(
                            span,
                            &format!("{prefix}{axis}"),
                            argument,
                            holes,
                            axis != "z",
                        )?
                    } else {
                        (
                            self.lower_css_3d_argument(
                                span,
                                &format!("{prefix}{axis}"),
                                argument,
                                holes,
                                kind,
                            )?,
                            false,
                        )
                    };
                    bindings.push(StyleBinding {
                        property: if percent {
                            format!("motion-transform-3d-{prefix}-{axis}-percent")
                        } else {
                            format!("motion-transform-3d-{prefix}-{axis}")
                        },
                        value,
                    });
                }
                continue;
            }
            if name == "rotate3d" {
                let Some(arguments) = split_transform_arguments(&argument, 4) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "rotate3d requires exactly four comma-separated values",
                    );
                    return None;
                };
                for (axis, argument) in ["x", "y", "z"].into_iter().zip(&arguments[..3]) {
                    let value = self.lower_css_3d_argument(
                        span,
                        &format!("rotate3d.{axis}"),
                        argument,
                        holes,
                        Css3dArgument::Number,
                    )?;
                    bindings.push(StyleBinding {
                        property: format!("motion-transform-3d-rotate-axis-{axis}"),
                        value,
                    });
                }
                let value = self.lower_css_3d_argument(
                    span,
                    "rotate3d.angle",
                    arguments[3],
                    holes,
                    Css3dArgument::Angle,
                )?;
                bindings.push(StyleBinding {
                    property: "motion-transform-3d-rotate-axis-angle".into(),
                    value,
                });
                continue;
            }
            if name == "scale" {
                let arguments = argument.split(',').map(str::trim).collect::<Vec<_>>();
                if !(1..=2).contains(&arguments.len())
                    || arguments.iter().any(|value| value.is_empty())
                {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "scale requires one value or exactly two comma-separated values",
                    );
                    return None;
                }
                for (axis, argument) in ["x", "y"].into_iter().zip(
                    arguments
                        .iter()
                        .copied()
                        .chain(arguments.first().copied())
                        .take(2),
                ) {
                    let value = self.lower_css_3d_argument(
                        span,
                        &format!("scale{axis}"),
                        argument,
                        holes,
                        Css3dArgument::Number,
                    )?;
                    bindings.push(StyleBinding {
                        property: format!("motion-transform-3d-scale-{axis}"),
                        value,
                    });
                }
                continue;
            }
            if matches!(name.as_str(), "translateX" | "translateY" | "translateZ") {
                let axis = match name.as_str() {
                    "translateX" => "x",
                    "translateY" => "y",
                    _ => "z",
                };
                let (value, percent) =
                    self.lower_css_3d_length_argument(span, &name, &argument, holes, axis != "z")?;
                bindings.push(StyleBinding {
                    property: if percent {
                        format!("motion-transform-3d-translate-{axis}-percent")
                    } else {
                        format!("motion-transform-3d-translate-{axis}")
                    },
                    value,
                });
                continue;
            }
            let (property, kind) = match name.as_str() {
                "rotate" | "rotateZ" => ("motion-transform-3d-rotate-z", Css3dArgument::Angle),
                "rotateX" => ("motion-transform-3d-rotate-x", Css3dArgument::Angle),
                "rotateY" => ("motion-transform-3d-rotate-y", Css3dArgument::Angle),
                "scaleX" => ("motion-transform-3d-scale-x", Css3dArgument::Number),
                "scaleY" => ("motion-transform-3d-scale-y", Css3dArgument::Number),
                "scaleZ" => ("motion-transform-3d-scale-z", Css3dArgument::Number),
                _ => unreachable!("parse_ordered_transform closes function names"),
            };
            let value = self.lower_css_3d_argument(span, &name, &argument, holes, kind)?;
            bindings.push(StyleBinding {
                property: property.into(),
                value,
            });
        }
        Some(bindings)
    }

    fn lower_css_3d_length_argument(
        &mut self,
        span: Span,
        function: &str,
        argument: &str,
        holes: &[Expression<'s>],
        allow_percent: bool,
    ) -> Option<(StyleValue, bool)> {
        if let Some((index, suffix)) = transform_hole_parts(argument) {
            let Some(expression) = holes.get(index) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "transform template hole index is out of bounds",
                );
                return None;
            };
            let percent = suffix == "%";
            if suffix != "px" && !(allow_percent && percent) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "{function} dynamic values require an explicit px suffix{}",
                        if allow_percent { " or % suffix" } else { "" }
                    ),
                );
                return None;
            }
            if percent {
                self.extra_capabilities
                    .insert(CSS_TRANSFORM_PERCENT_CAPABILITY.to_owned());
            }
            return Some((
                StyleValue::Expr {
                    expr: self.lower_expr(expression)?,
                },
                percent,
            ));
        }
        if argument.contains("__VALLE_HOLE_") {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{function} accepts one whole numeric template hole"),
            );
            return None;
        }
        if argument == "0" {
            return Some((
                StyleValue::Static {
                    value: MotionValue::Number(0.0),
                },
                false,
            ));
        }
        let Some(length) = Length::parse(argument) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{function} requires a finite px length or percentage"),
            );
            return None;
        };
        let percent = length.unit == valle_motion::value::LengthUnit::Percent;
        if !matches!(
            length.unit,
            valle_motion::value::LengthUnit::Px | valle_motion::value::LengthUnit::Percent
        ) || (percent && !allow_percent)
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "{function} requires a finite px length{}",
                    if allow_percent { " or percentage" } else { "" }
                ),
            );
            return None;
        }
        if percent {
            self.extra_capabilities
                .insert(CSS_TRANSFORM_PERCENT_CAPABILITY.to_owned());
        }
        Some((
            StyleValue::Static {
                value: MotionValue::Number(length.value),
            },
            percent,
        ))
    }

    pub(super) fn lower_css_3d_argument(
        &mut self,
        span: Span,
        function: &str,
        argument: &str,
        holes: &[Expression<'s>],
        kind: Css3dArgument,
    ) -> Option<StyleValue> {
        if let Some((index, suffix)) = transform_hole_parts(argument) {
            let Some(expression) = holes.get(index) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "transform template hole index is out of bounds",
                );
                return None;
            };
            let mut expr = self.lower_expr(expression)?;
            match kind {
                Css3dArgument::Length if suffix == "px" => {}
                Css3dArgument::Number if suffix.is_empty() => {}
                Css3dArgument::Angle if matches!(suffix, "deg" | "rad" | "turn") => {
                    let factor = match suffix {
                        "deg" => 1.0,
                        "rad" => 180.0 / core::f64::consts::PI,
                        _ => 360.0,
                    };
                    if factor != 1.0 {
                        let rhs = self.push(
                            Expr::Const {
                                value: MotionValue::Number(factor),
                            },
                            expression.span(),
                        );
                        expr = self.push(Expr::Mul { lhs: expr, rhs }, expression.span());
                    }
                }
                Css3dArgument::Length => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{function} dynamic values require an explicit px suffix"),
                    );
                    return None;
                }
                Css3dArgument::Angle => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{function} dynamic values require deg, rad, or turn"),
                    );
                    return None;
                }
                Css3dArgument::Number => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{function} dynamic values must not have a unit"),
                    );
                    return None;
                }
            }
            return Some(StyleValue::Expr { expr });
        }
        if argument.contains("__VALLE_HOLE_") {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{function} accepts one whole numeric template hole"),
            );
            return None;
        }
        let value = match kind {
            Css3dArgument::Length => {
                if argument == "0" {
                    Some(0.0)
                } else {
                    Length::parse(argument)
                        .filter(|length| length.unit == valle_motion::value::LengthUnit::Px)
                        .map(|length| length.value)
                }
            }
            Css3dArgument::Angle => Angle::parse(argument).map(Angle::as_degrees),
            Css3dArgument::Number => argument.parse::<f64>().ok(),
        }
        .filter(|value| value.is_finite());
        let Some(value) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{function} requires a finite value with the expected CSS unit"),
            );
            return None;
        };
        Some(StyleValue::Static {
            value: MotionValue::Number(value),
        })
    }
}

fn is_css_3d_function(name: &str) -> bool {
    matches!(
        name,
        "translateZ"
            | "translate3d"
            | "rotateX"
            | "rotateY"
            | "rotateZ"
            | "rotate3d"
            | "scaleZ"
            | "scale3d"
    )
}
