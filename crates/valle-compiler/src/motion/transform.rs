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
        if let Some(serde_json::Value::String(value)) = self.eval_static(expression) {
            if let Some(parts) = parse_core_transform(&value) {
                if !parts.iter().any(|(name, argument)| {
                    matches!(name.as_str(), "translate" | "translateX" | "translateY")
                        && argument.contains('%')
                }) {
                    return self.lower_transform_parts(expression.span(), parts, &[]);
                }
            }
            if let Some(parts) = parse_ordered_transform(&value) {
                self.extra_capabilities
                    .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
                return self.lower_css_3d_transform_parts(expression.span(), parts, &[]);
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "transform must use the supported 2D sequence or ordered CSS 3D translate/rotate/scale functions",
            );
            return None;
        }
        let Expression::TemplateLiteral(template) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "transform must be a static string or a closed template literal",
            );
            return None;
        };
        if template.quasis.len() != template.expressions.len() + 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                template.span,
                "malformed transform template literal",
            );
            return None;
        }
        let mut source = String::new();
        for (index, quasi) in template.quasis.iter().enumerate() {
            let Some(cooked) = quasi.value.cooked else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    quasi.span,
                    "transform template contains an invalid escape",
                );
                return None;
            };
            source.push_str(cooked.as_str());
            if index < template.expressions.len() {
                source.push_str(&format!("__VALLE_HOLE_{index}__"));
            }
        }
        if let Some(parts) = parse_core_transform(&source) {
            if !parts.iter().any(|(name, argument)| {
                matches!(name.as_str(), "translate" | "translateX" | "translateY")
                    && argument.contains('%')
            }) {
                return self.lower_transform_parts(template.span, parts, &template.expressions);
            }
        }
        if let Some(parts) = parse_ordered_transform(&source) {
            self.extra_capabilities
                .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
            return self.lower_css_3d_transform_parts(template.span, parts, &template.expressions);
        }
        self.illegal(
            DiagCode::GrammarForbidden,
            template.span,
            "transform must use the supported 2D sequence or ordered CSS 3D translate/rotate/scale functions",
        );
        None
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

    pub(super) fn lower_transform_parts(
        &mut self,
        span: Span,
        parts: Vec<(String, String)>,
        holes: &[Expression<'s>],
    ) -> Option<Vec<StyleBinding>> {
        if parts.iter().any(|(name, _)| name == "translate")
            && parts
                .iter()
                .any(|(name, _)| matches!(name.as_str(), "translateX" | "translateY"))
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "transform cannot mix translate(...) with translateX(...)/translateY(...) syntax",
            );
            return None;
        }
        if parts.iter().any(|(name, _)| name == "scale")
            && parts
                .iter()
                .any(|(name, _)| matches!(name.as_str(), "scaleX" | "scaleY"))
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "transform cannot mix scale(...) with scaleX(...)/scaleY(...)",
            );
            return None;
        }
        let mut bindings = Vec::with_capacity(parts.len());
        let mut translate_axis_static = [None, None];
        let mut translate_axis_expr = [None, None];
        let mut scale_axis_static = [None, None];
        let mut scale_axis_expr = [None, None];
        for (name, argument) in parts {
            if matches!(name.as_str(), "translateX" | "translateY") {
                let axis = usize::from(name == "translateY");
                if let Some((index, suffix)) = transform_hole_parts(&argument) {
                    if suffix != "px" {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!(
                                "transform `{name}` dynamic value needs an explicit px suffix, as in `{name}(${{value}}px)`"
                            ),
                        );
                        return None;
                    }
                    let Some(expression) = holes.get(index) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "transform template hole index is out of bounds",
                        );
                        return None;
                    };
                    translate_axis_expr[axis] = Some(self.lower_expr(expression)?);
                } else if argument.contains("__VALLE_HOLE_") {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("transform `{name}` accepts one whole numeric hole followed by px"),
                    );
                    return None;
                } else {
                    let value = argument
                        .strip_suffix("px")
                        .or_else(|| (argument == "0").then_some("0"))
                        .and_then(|value| value.parse::<f64>().ok())
                        .filter(|value| value.is_finite());
                    let Some(value) = value else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!("transform `{name}` requires a finite px length"),
                        );
                        return None;
                    };
                    translate_axis_static[axis] = Some(value);
                }
                continue;
            }
            if matches!(name.as_str(), "scaleX" | "scaleY") {
                let axis = usize::from(name == "scaleY");
                if let Some((index, suffix)) = transform_hole_parts(&argument) {
                    if !suffix.is_empty() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!("transform `{name}` takes a unitless Number hole"),
                        );
                        return None;
                    }
                    let Some(expression) = holes.get(index) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "transform template hole index is out of bounds",
                        );
                        return None;
                    };
                    scale_axis_expr[axis] = Some(self.lower_expr(expression)?);
                } else {
                    let Some(value) = argument
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!("transform `{name}` requires a finite unitless number"),
                        );
                        return None;
                    };
                    scale_axis_static[axis] = Some(value);
                }
                continue;
            }
            let property = match name.as_str() {
                "translate" => "translate",
                "rotate" => "rotate",
                "scale" => "scale",
                _ => unreachable!("parse_core_transform closes function names"),
            };
            if property == "scale" && argument.contains(',') {
                if let Some(binding) = self.lower_two_axis_scale_argument(span, &argument, holes) {
                    self.extra_capabilities
                        .insert(TRANSFORM_SCALE2D_CAPABILITY.to_owned());
                    bindings.push(binding);
                } else {
                    return None;
                }
                continue;
            }
            let value = if let Some((index, suffix)) = transform_hole_parts(&argument) {
                let Some(expression) = holes.get(index) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "transform template hole index is out of bounds",
                    );
                    return None;
                };
                let mut expr = self.lower_expr(expression)?;
                match (property, suffix) {
                    (_, "") => {
                        if property == "translate" {
                            expr = self.push(Expr::ToLength2 { input: expr }, expression.span());
                        }
                    }
                    // Convert numeric transform holes to Angle using identity interpolation with
                    // Extend extrapolation, preserving exact values without adding an IR variant.
                    ("rotate", "deg" | "rad" | "turn") => {
                        let unit = match suffix {
                            "deg" => AngleUnit::Deg,
                            "rad" => AngleUnit::Rad,
                            _ => AngleUnit::Turn,
                        };
                        let stop = |value: f64| InterpolateStop {
                            input: value,
                            output: MotionValue::Angle(Angle { value, unit }),
                        };
                        expr = self.push(
                            Expr::Interpolate {
                                input: expr,
                                stops: vec![stop(0.0), stop(1.0)],
                                easings: Vec::new(),
                                extrapolate_left: Extrapolation::Extend,
                                extrapolate_right: Extrapolation::Extend,
                            },
                            expression.span(),
                        );
                    }
                    ("rotate", _) => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "rotate hole accepts only a static `deg`/`rad`/`turn` suffix, \
                             as in `rotate(${a}deg)`",
                        );
                        return None;
                    }
                    ("translate", _) => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "translate takes one Length2-valued hole — build pairs with \
                             `point(x, y)` or interpolate over \"0px 32px\" stops",
                        );
                        return None;
                    }
                    _ => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "scale takes a unitless Number hole",
                        );
                        return None;
                    }
                }
                StyleValue::Expr { expr }
            } else if argument.contains("__VALLE_HOLE_") {
                // Reject unsupported hole placement without leaking internal placeholder names into
                // diagnostics.
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "transform `{name}` hole must span the whole argument, optionally \
                         followed by a static angle unit on rotate"
                    ),
                );
                return None;
            } else {
                let value = match property {
                    "translate" => Length2::parse(&argument).map(MotionValue::Length2),
                    "rotate" => Angle::parse(&argument).map(MotionValue::Angle),
                    "scale" => argument
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .map(MotionValue::Number),
                    _ => None,
                };
                let Some(value) = value else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("transform `{name}` has an invalid typed value `{argument}`"),
                    );
                    return None;
                };
                StyleValue::Static { value }
            };
            bindings.push(StyleBinding {
                property: property.into(),
                value,
            });
        }
        if translate_axis_static.iter().any(Option::is_some)
            || translate_axis_expr.iter().any(Option::is_some)
        {
            let value = if translate_axis_expr.iter().all(Option::is_none) {
                StyleValue::Static {
                    value: MotionValue::Length2(Length2::px(
                        translate_axis_static[0].unwrap_or(0.0),
                        translate_axis_static[1].unwrap_or(0.0),
                    )),
                }
            } else {
                let mut axes = [None, None];
                for index in 0..2 {
                    axes[index] = translate_axis_expr[index].or_else(|| {
                        Some(self.push(
                            Expr::Const {
                                value: MotionValue::Number(
                                    translate_axis_static[index].unwrap_or(0.0),
                                ),
                            },
                            span,
                        ))
                    });
                }
                let point = self.push(
                    Expr::MakePoint {
                        x: axes[0].expect("translate x is synthesized"),
                        y: axes[1].expect("translate y is synthesized"),
                    },
                    span,
                );
                StyleValue::Expr {
                    expr: self.push(Expr::ToLength2 { input: point }, span),
                }
            };
            bindings.insert(
                0,
                StyleBinding {
                    property: "translate".into(),
                    value,
                },
            );
        }
        if scale_axis_static.iter().any(Option::is_some)
            || scale_axis_expr.iter().any(Option::is_some)
        {
            self.extra_capabilities
                .insert(TRANSFORM_SCALE2D_CAPABILITY.to_owned());
            let value = if scale_axis_expr.iter().all(Option::is_none) {
                StyleValue::Static {
                    value: MotionValue::Point(Point::new(
                        scale_axis_static[0].unwrap_or(1.0),
                        scale_axis_static[1].unwrap_or(1.0),
                    )),
                }
            } else {
                let mut axes = [None, None];
                for index in 0..2 {
                    axes[index] = scale_axis_expr[index].or_else(|| {
                        Some(self.push(
                            Expr::Const {
                                value: MotionValue::Number(scale_axis_static[index].unwrap_or(1.0)),
                            },
                            span,
                        ))
                    });
                }
                StyleValue::Expr {
                    expr: self.push(
                        Expr::MakePoint {
                            x: axes[0].expect("scale x is synthesized"),
                            y: axes[1].expect("scale y is synthesized"),
                        },
                        span,
                    ),
                }
            };
            bindings.push(StyleBinding {
                property: "scale".into(),
                value,
            });
        }
        Some(bindings)
    }

    pub(super) fn lower_two_axis_scale_argument(
        &mut self,
        span: Span,
        argument: &str,
        holes: &[Expression<'s>],
    ) -> Option<StyleBinding> {
        let mut parts = argument.split(',').map(str::trim);
        let first = parts.next()?;
        let second = parts.next()?;
        if parts.next().is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "scale(x, y) takes exactly two unitless numbers",
            );
            return None;
        }
        let mut axis = |part: &str| -> Option<ExprId> {
            if let Some((index, suffix)) = transform_hole_parts(part) {
                if !suffix.is_empty() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "scale(x, y) holes must be unitless numbers",
                    );
                    return None;
                }
                let expression = holes.get(index)?;
                return self.lower_expr(expression);
            }
            let value = part.parse::<f64>().ok().filter(|value| value.is_finite())?;
            Some(self.push(
                Expr::Const {
                    value: MotionValue::Number(value),
                },
                span,
            ))
        };
        let x = axis(first)?;
        let y = axis(second)?;
        if matches!(
            (
                self.expr_arena.values.get(x.0 as usize),
                self.expr_arena.values.get(y.0 as usize)
            ),
            (
                Some(Expr::Const {
                    value: MotionValue::Number(sx)
                }),
                Some(Expr::Const {
                    value: MotionValue::Number(sy)
                })
            ) if sx.is_finite() && sy.is_finite()
        ) {
            let (
                Some(Expr::Const {
                    value: MotionValue::Number(sx),
                }),
                Some(Expr::Const {
                    value: MotionValue::Number(sy),
                }),
            ) = (
                self.expr_arena.values.get(x.0 as usize).cloned(),
                self.expr_arena.values.get(y.0 as usize).cloned(),
            )
            else {
                unreachable!("checked above");
            };
            return Some(StyleBinding {
                property: "scale".into(),
                value: StyleValue::Static {
                    value: MotionValue::Point(Point::new(sx, sy)),
                },
            });
        }
        Some(StyleBinding {
            property: "scale".into(),
            value: StyleValue::Expr {
                expr: self.push(Expr::MakePoint { x, y }, span),
            },
        })
    }
}
