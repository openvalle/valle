//! CSS filters, paint diagnostics, displacement, blur, FLIP, and motion path.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_css_filter_style(
        &mut self,
        expression: &'s Expression<'s>,
        property: &str,
    ) -> Option<StyleValue> {
        if let Some(text) = string_literal(expression).or_else(|| {
            self.fold_to_value(expression)
                .and_then(|value| match value {
                    MotionValue::Str(text) => Some(text),
                    _ => None,
                })
        }) {
            return match normalize_css_filter(&text) {
                Ok(None) => None,
                Ok(Some(kept)) => Some(StyleValue::Static {
                    value: MotionValue::Str(kept),
                }),
                Err(reason) => {
                    self.illegal(DiagCode::GrammarForbidden, expression.span(), reason);
                    None
                }
            };
        }
        let expr = self.lower_css_value_expression(expression)?;
        let _ = property;
        Some(StyleValue::Expr { expr })
    }

    /// CSS keywords such as `none` are strings here, not general Motion enum values.
    pub(super) fn lower_css_value_expression(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<ExprId> {
        let expression = strip_parens(expression);
        if let Some(text) = string_literal(expression) {
            return Some(self.push(
                Expr::Const {
                    value: MotionValue::Str(text),
                },
                expression.span(),
            ));
        }
        if let Expression::ConditionalExpression(branch) = expression {
            if self.expr_arena.depth >= MAX_EXPR_NESTING {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "CSS value branch nesting exceeds the expression budget",
                );
                return None;
            }
            self.expr_arena.depth += 1;
            let result = (|| {
                let condition = self.lower_expr(&branch.test)?;
                let when_true = self.lower_css_value_expression(&branch.consequent)?;
                let when_false = self.lower_css_value_expression(&branch.alternate)?;
                Some(self.push(
                    Expr::Select {
                        condition,
                        when_true,
                        when_false,
                    },
                    expression.span(),
                ))
            })();
            self.expr_arena.depth -= 1;
            return result;
        }
        self.lower_expr(expression)
    }

    pub(super) fn diagnose_path_paint(
        &mut self,
        span: Span,
        tag: &str,
        svg_shape: Option<&str>,
        path: &PathValue,
        saw_explicit_fill: bool,
        saw_explicit_stroke: bool,
    ) {
        if !saw_explicit_stroke || saw_explicit_fill {
            return;
        }
        match path_topology(svg_shape, path, &self.expr_arena.values) {
            PathTopology::Open => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "stroked <{tag}> requires an explicit fill; write fill=\"none\" for a stroke-only path or fill=\"#000\" for black"
                    ),
                );
            }
            PathTopology::Closed | PathTopology::Unknown => {}
        }
    }

    pub(super) fn diagnose_border_paint(
        &mut self,
        span: Span,
        class_names: &[String],
        styles: &[StyleBinding],
    ) {
        let mut width = false;
        let mut color = false;
        let mut style_present = false;
        let mut style_none = false;
        for class_name in class_names {
            if tailwind_sets_border_width(class_name) {
                width = true;
            }
            if tailwind_sets_border_color(class_name) {
                color = true;
            }
            if tailwind_sets_border_style(class_name) {
                style_present = true;
                if *class_name == "border-none" {
                    style_none = true;
                }
            }
        }
        for style in styles {
            let name = style.property.as_str();
            if is_border_width_property(name) {
                width = true;
            }
            if is_border_color_property(name) {
                color = true;
            }
            if is_border_style_property(name) {
                style_present = true;
                if matches!(
                    &style.value,
                    StyleValue::Static {
                        value: MotionValue::Str(value) | MotionValue::Enum(value)
                    } if value == "none"
                ) {
                    style_none = true;
                }
            }
        }
        if (width || color) && (!style_present || style_none) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "border width/color requires an explicit borderStyle (CSS default is none); write borderStyle: \"solid\" or a Tailwind border-solid class",
            );
        }
    }

    /// Lower displacement to reserved typed bindings resolved into ProgramRecording filters by the
    /// shared layout layer.
    pub(super) fn lower_displacement_style(
        &mut self,
        expression: &'s Expression<'s>,
        backdrop: bool,
    ) -> Option<Vec<StyleBinding>> {
        let author_property = if backdrop {
            "style.backdropDisplacement"
        } else {
            "style.displacement"
        };
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!("{author_property} must call displacement(seed, frequencyPoint, scale, options?)"),
            );
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.callee.span(),
                "style.displacement only accepts the displacement primitive",
            );
            return None;
        };
        if callee.name != "displacement" || !(3..=4).contains(&call.arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "displacement(seed, frequencyPoint, scale, options?) requires three or four arguments",
            );
            return None;
        }
        let seed_expr = call.arguments[0].as_expression()?;
        // Compile-time integer seeds remain static; frame-time seeds require `displacement-seed-
        // expr`. Evaluation never rounds seeds.
        let seed_id = self.lower_expr(seed_expr)?;
        let seed_value = match self.expr_arena.values.get(seed_id.0 as usize) {
            Some(Expr::Const {
                value: MotionValue::Number(value),
            }) if value.is_finite()
                && value.fract() == 0.0
                && (0.0..=f64::from(u32::MAX)).contains(value) =>
            {
                StyleValue::Static {
                    value: MotionValue::Number(*value),
                }
            }
            Some(Expr::Const { .. }) => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    seed_expr.span(),
                    "displacement seed must be a finite integer in 0..=4294967295",
                );
                return None;
            }
            _ => {
                self.extra_capabilities
                    .insert(DISPLACEMENT_SEED_EXPR_CAPABILITY.to_owned());
                StyleValue::Expr { expr: seed_id }
            }
        };
        let frequency = self.lower_expr(call.arguments[1].as_expression()?)?;
        let scale = self.lower_expr(call.arguments[2].as_expression()?)?;
        let mut octaves = 2_u64;
        let mut mode = "fractal";
        if let Some(options) = call.arguments.get(3) {
            let value = options
                .as_expression()
                .and_then(|expression| self.eval_static(expression));
            let Some(object) = value.as_ref().and_then(serde_json::Value::as_object) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    options.span(),
                    "displacement options must be a static object",
                );
                return None;
            };
            for (name, value) in object {
                match name.as_str() {
                    "octaves" => match value.as_u64() {
                        Some(value @ 1..=4) => octaves = value,
                        _ => {
                            self.illegal(
                                DiagCode::BuiltinRejected,
                                options.span(),
                                "displacement octaves must be a static integer from 1 to 4",
                            );
                            return None;
                        }
                    },
                    "mode" => match value.as_str() {
                        Some("fractal") => mode = "fractal",
                        Some("turbulence") => mode = "turbulence",
                        _ => {
                            self.illegal(
                                DiagCode::BuiltinRejected,
                                options.span(),
                                "displacement mode must be \"fractal\" or \"turbulence\"",
                            );
                            return None;
                        }
                    },
                    other => {
                        self.illegal(
                            DiagCode::BuiltinRejected,
                            options.span(),
                            format!("unknown displacement option `{other}`; use octaves / mode"),
                        );
                        return None;
                    }
                }
            }
        }
        self.extra_capabilities.insert(
            if backdrop {
                BACKDROP_DISPLACEMENT_CAPABILITY
            } else {
                valle_motion::NODE_ADVANCED_FILTER_CAPABILITY
            }
            .to_owned(),
        );
        let prefix = if backdrop {
            "motion-backdrop-displacement"
        } else {
            "motion-displacement"
        };
        Some(vec![
            StyleBinding {
                property: format!("{prefix}-seed"),
                value: seed_value,
            },
            StyleBinding {
                property: format!("{prefix}-frequency"),
                value: StyleValue::Expr { expr: frequency },
            },
            StyleBinding {
                property: format!("{prefix}-scale"),
                value: StyleValue::Expr { expr: scale },
            },
            StyleBinding {
                property: format!("{prefix}-octaves"),
                value: StyleValue::Static {
                    value: MotionValue::Number(octaves as f64),
                },
            },
            StyleBinding {
                property: format!("{prefix}-mode"),
                value: StyleValue::Static {
                    value: MotionValue::Enum(mode.into()),
                },
            },
        ])
    }

    /// Velocity is explicit and frame-pure (px/frame); shutter controls exposure length. This is
    /// a spatial shutter approximation, not hidden previous-frame state or temporal replay.
    pub(super) fn lower_node_motion_blur_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style.motionBlur must call motionBlur(velocityPoint, shutterAngle?)",
            );
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.callee.span(),
                "style.motionBlur only accepts the motionBlur primitive",
            );
            return None;
        };
        if callee.name != "motionBlur" || !(1..=2).contains(&call.arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "motionBlur(velocityPoint, shutterAngle?) takes one or two arguments",
            );
            return None;
        }
        let velocity = self.lower_expr(call.arguments[0].as_expression()?)?;
        let shutter = if let Some(value) = call.arguments.get(1) {
            self.lower_expr(value.as_expression()?)?
        } else {
            self.push(
                Expr::Const {
                    value: MotionValue::Number(180.0),
                },
                call.span(),
            )
        };
        self.extra_capabilities
            .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
        Some(vec![
            StyleBinding {
                property: "motion-velocity-blur-velocity".into(),
                value: StyleValue::Expr { expr: velocity },
            },
            StyleBinding {
                property: "motion-velocity-blur-shutter".into(),
                value: StyleValue::Expr { expr: shutter },
            },
        ])
    }

    /// Prepare fixed-topology FLIP states by laying out the final rectangle once and animating an
    /// inverse translation and two-axis scale to identity.
    pub(super) fn lower_flip_style(
        &mut self,
        expression: &'s Expression<'s>,
        layout_id: &str,
        span: Span,
    ) -> Option<Vec<StyleBinding>> {
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style.layoutTransition must call flip(layouts, from, to, progress)",
            );
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.callee.span(),
                "style.layoutTransition only accepts the flip primitive",
            );
            return None;
        };
        if callee.name != "flip" || call.arguments.len() != 4 {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "flip(layouts, from, to, progress) takes exactly four arguments",
            );
            return None;
        }
        let plan_expression = call.arguments[0].as_expression()?;
        let plan_value = self.eval_static(plan_expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                plan_expression.span(),
                "flip layouts must come from a prepare-time defineLayoutStates result",
            );
            None
        })?;
        let plan: StaticLayoutStates = serde_json::from_value(plan_value).ok().or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                plan_expression.span(),
                "flip layouts must be a valid defineLayoutStates result",
            );
            None
        })?;
        let ids = plan.ids.iter().cloned().collect::<BTreeSet<_>>();
        let valid_plan = plan.marker == "layoutStates"
            && (2..=16).contains(&plan.states.len())
            && !ids.is_empty()
            && ids.len() == plan.ids.len()
            && ids.len() <= 512
            && plan.states.values().all(|state| {
                state.keys().cloned().collect::<BTreeSet<_>>() == ids
                    && state.values().all(|rect| {
                        rect.marker == "rect"
                            && rect.x.is_finite()
                            && rect.y.is_finite()
                            && rect.width.is_finite()
                            && rect.height.is_finite()
                            && rect.width > 0.0
                            && rect.height > 0.0
                    })
            });
        if !valid_plan {
            self.illegal(
                DiagCode::BuiltinRejected,
                plan_expression.span(),
                "flip layout states must contain 2..=16 states with the same 1..=512 positive Rect entries",
            );
            return None;
        }
        if !ids.contains(layout_id) {
            self.illegal(
                DiagCode::UnknownProp,
                span,
                format!("layoutId `{layout_id}` does not exist in defineLayoutStates"),
            );
            return None;
        }
        let state_name = |at: usize, label: &str, this: &mut Self| {
            let expression = call.arguments[at].as_expression()?;
            let value = this.eval_static(expression)?;
            let Some(value) = value.as_str() else {
                this.illegal(
                    DiagCode::BuiltinRejected,
                    expression.span(),
                    format!("flip {label} state must be a static string"),
                );
                return None;
            };
            Some(value.to_owned())
        };
        let from_name = state_name(1, "from", self)?;
        let to_name = state_name(2, "to", self)?;
        let Some(from) = plan
            .states
            .get(&from_name)
            .and_then(|state| state.get(layout_id))
        else {
            self.illegal(
                DiagCode::UnknownProp,
                call.arguments[1].span(),
                format!("flip references unknown from state `{from_name}`"),
            );
            return None;
        };
        let Some(to) = plan
            .states
            .get(&to_name)
            .and_then(|state| state.get(layout_id))
        else {
            self.illegal(
                DiagCode::UnknownProp,
                call.arguments[2].span(),
                format!("flip references unknown to state `{to_name}`"),
            );
            return None;
        };
        let progress = self.lower_expr(call.arguments[3].as_expression()?)?;
        let number = |this: &mut Self, from: f64, to: f64| {
            this.push(
                Expr::Interpolate {
                    input: progress,
                    stops: vec![
                        InterpolateStop {
                            input: 0.0,
                            output: MotionValue::Number(from),
                        },
                        InterpolateStop {
                            input: 1.0,
                            output: MotionValue::Number(to),
                        },
                    ],
                    easings: Vec::new(),
                    extrapolate_left: Extrapolation::Clamp,
                    extrapolate_right: Extrapolation::Clamp,
                },
                span,
            )
        };
        let translate_x = number(self, from.x - to.x, 0.0);
        let translate_y = number(self, from.y - to.y, 0.0);
        let translate_point = self.push(
            Expr::MakePoint {
                x: translate_x,
                y: translate_y,
            },
            span,
        );
        let translate = self.push(
            Expr::ToLength2 {
                input: translate_point,
            },
            span,
        );
        let scale_x = number(self, from.width / to.width, 1.0);
        let scale_y = number(self, from.height / to.height, 1.0);
        let scale = self.push(
            Expr::MakePoint {
                x: scale_x,
                y: scale_y,
            },
            span,
        );
        self.extra_capabilities.insert(FLIP_CAPABILITY.to_owned());
        self.extra_capabilities
            .insert(TRANSFORM_SCALE2D_CAPABILITY.to_owned());
        Some(vec![
            StyleBinding {
                property: "position".into(),
                value: StyleValue::Static {
                    value: MotionValue::Str("absolute".into()),
                },
            },
            StyleBinding {
                property: "left".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(to.x),
                },
            },
            StyleBinding {
                property: "top".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(to.y),
                },
            },
            StyleBinding {
                property: "width".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(to.width),
                },
            },
            StyleBinding {
                property: "height".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(to.height),
                },
            },
            StyleBinding {
                property: "transform-origin".into(),
                value: StyleValue::Static {
                    value: MotionValue::Length2(Length2::px(0.0, 0.0)),
                },
            },
            StyleBinding {
                property: "translate".into(),
                value: StyleValue::Expr { expr: translate },
            },
            StyleBinding {
                property: "scale".into(),
                value: StyleValue::Expr { expr: scale },
            },
        ])
    }

    pub(super) fn lower_motion_path_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style.motionPath must call motionPath(path, progress) or follow(path, progress)",
            );
            return None;
        };
        let Expression::Identifier(callee) = &call.callee else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.callee.span(),
                "style.motionPath only accepts the motionPath or follow primitive",
            );
            return None;
        };
        if !matches!(callee.name.as_str(), "motionPath" | "follow")
            || !(2..=3).contains(&call.arguments.len())
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "motionPath/follow(path, progress, options?) requires two or three arguments",
            );
            return None;
        }
        let path = self.lower_expr(call.arguments[0].as_expression()?)?;
        let progress = self.lower_expr(call.arguments[1].as_expression()?)?;
        let mut anchor = "center";
        let mut rotate = "auto";
        let mut angle_offset = 0.0;
        if let Some(options) = call.arguments.get(2) {
            let Some(value) = options
                .as_expression()
                .and_then(|expression| self.eval_static(expression))
            else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    options.span(),
                    "motionPath options must be a static object literal",
                );
                return None;
            };
            let Some(object) = value.as_object() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    options.span(),
                    "motionPath options must be an object",
                );
                return None;
            };
            for (name, value) in object {
                match name.as_str() {
                    "anchor" => match value.as_str() {
                        Some("center") => anchor = "center",
                        Some("topLeft") => anchor = "topLeft",
                        _ => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                options.span(),
                                "motionPath anchor must be \"center\" or \"topLeft\"",
                            );
                            return None;
                        }
                    },
                    "rotate" => match value.as_str() {
                        Some("auto") => rotate = "auto",
                        Some("none") => rotate = "none",
                        _ => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                options.span(),
                                "motionPath rotate must be \"auto\" or \"none\"",
                            );
                            return None;
                        }
                    },
                    "angleOffset" => match value.as_f64() {
                        Some(value) if value.is_finite() => angle_offset = value,
                        _ => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                options.span(),
                                "motionPath angleOffset must be a finite static number",
                            );
                            return None;
                        }
                    },
                    other => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            options.span(),
                            format!(
                                "unknown motionPath option `{other}`; use anchor / rotate / angleOffset"
                            ),
                        );
                        return None;
                    }
                }
            }
        }
        let point = self.push(Expr::PathPointAt { path, progress }, call.span());
        let translate = self.push(Expr::ToLength2 { input: point }, call.span());
        let mut bindings = vec![
            StyleBinding {
                property: "motion-path-anchor".into(),
                value: StyleValue::Static {
                    value: MotionValue::Enum(anchor.into()),
                },
            },
            StyleBinding {
                property: "translate".into(),
                value: StyleValue::Expr { expr: translate },
            },
        ];
        if angle_offset != 0.0 {
            bindings.push(StyleBinding {
                property: "motion-path-angle-offset".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(angle_offset),
                },
            });
        }
        if rotate == "auto" {
            let rotate = self.push(Expr::PathAngleAt { path, progress }, call.span());
            bindings.push(StyleBinding {
                property: "rotate".into(),
                value: StyleValue::Expr { expr: rotate },
            });
        }
        Some(bindings)
    }
}
