//! CSS filters, paint diagnostics, displacement, blur, FLIP, and motion path.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_css_filter_style(
        &mut self,
        expression: &'s Expression<'s>,
        property: &str,
    ) -> Option<Vec<StyleBinding>> {
        let text = string_literal(expression).or_else(|| {
            self.fold_to_value(expression).and_then(|v| {
                if let MotionValue::Str(text) = v {
                    Some(text)
                } else {
                    None
                }
            })
        });
        let mut frame_dependent = text.is_none();
        let value = if let Some(text) = text {
            if let Err(reason) = valle_motion::style::parse_property(property, &text) {
                self.style_diagnostic(expression.span(), None, reason);
                return None;
            }
            if property == "filter"
                && valle_motion::style::advanced_filter::parse(&text)
                    .ok()
                    .flatten()
                    .is_some()
            {
                self.extra_capabilities
                    .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
                frame_dependent = text.trim().starts_with("film-grain(");
            }
            StyleValue::Static {
                value: MotionValue::Str(text),
            }
        } else {
            if property == "filter" {
                self.extra_capabilities
                    .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
            }
            let expr = self.lower_css_value_expression(expression)?;
            if property == "filter" {
                self.diagnose_filter_ranges(expr);
            }
            StyleValue::Expr { expr }
        };
        let mut styles = vec![StyleBinding {
            property: property.into(),
            value,
        }];
        if property == "filter" && frame_dependent {
            // Grain changes once per local frame, including random seeks and TimeScope.
            let frame = self.scoped_context_expr(ContextInput::LocalFrame, expression.span());
            styles.push(StyleBinding {
                property: "motion-filter-frame".into(),
                value: StyleValue::Expr { expr: frame },
            });
        }
        Some(styles)
    }

    fn diagnose_filter_ranges(&mut self, expr: ExprId) {
        use valle_motion::expr::TemplatePart;
        let Some(Expr::Template { parts }) = self.expr_arena.values.get(expr.0 as usize) else {
            return;
        };
        let mut template = String::new();
        for part in parts {
            match part {
                TemplatePart::Text { value } => template.push_str(value),
                TemplatePart::Expr { expr } => {
                    if let Some(Expr::Const { value }) = self.expr_arena.values.get(expr.0 as usize)
                    {
                        template.push_str(&valle_motion::css_token(value));
                    } else {
                        template.push_str(&format!("@{}@", expr.0));
                    }
                }
            }
        }
        let Some((name, body)) = template.trim().split_once('(') else {
            return;
        };
        let Some(body) = body.strip_suffix(')') else {
            return;
        };
        for (raw, parameter) in body
            .split_whitespace()
            .zip(valle_motion::style::advanced_filter::parameters(name))
        {
            let raw = raw.strip_suffix(parameter.unit).unwrap_or(raw);
            let Some(id) = raw
                .strip_prefix('@')
                .and_then(|s| s.strip_suffix('@'))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            self.diagnose_parameter_range(
                ExprId(id),
                &format!("{name} {}", parameter.name),
                parameter.unit,
                parameter.min,
                parameter.max,
            );
        }
    }

    pub(super) fn diagnose_parameter_range(
        &mut self,
        expr: ExprId,
        label: &str,
        unit: &str,
        min: f64,
        max: f64,
    ) {
        use valle_motion::time_function::{ValueRange, numeric_range};
        let range = numeric_range(&self.expr_arena.values, expr, &|input| match input {
            ContextInput::LocalProgress => ValueRange::new(0.0, 1.0),
            ContextInput::CompositionSeconds => {
                ValueRange::new(0.0, self.composition.as_ref()?.duration().ok()?.as_f64())
            }
            // FPS is a host input, so frame-count-based expressions cannot use
            // the optional author default as a universal proof.
            _ => None,
        });
        let Some(range) = range else {
            return;
        };
        let span = self.expr_arena.spans[expr.0 as usize];
        if range.lower > max || range.upper < min {
            self.illegal(
                DiagCode::StyleInvalidValue,
                span,
                format!(
                    "{label} range {:.6e}..={:.6e}{} is outside {min}..={max}{unit}",
                    range.lower, range.upper, unit
                ),
            );
        } else if range.lower < min - 1e-9 || range.upper > max + 1e-9 {
            self.warn(DiagCode::ParameterRange,span,format!("{label} may leave {min}..={max}{unit}; conservative range {:.6e}..={:.6e}{}. Use clamp() explicitly if intended; runtime values remain strict.",range.lower,range.upper,unit));
        }
    }

    /// CSS keywords such as `none` are strings here, not general Motion enum values.
    pub(super) fn lower_css_value_expression(
        &mut self,
        expression: &Expression<'_>,
    ) -> Option<ExprId> {
        let expression = strip_parens(expression);
        if let Some(text) = string_literal(expression).or_else(|| {
            (!matches!(expression, Expression::TemplateLiteral(template) if !template.expressions.is_empty()))
                .then(|| self.eval_static(expression)).flatten()
                .and_then(|value| value.as_str().map(str::to_owned))
        }) {
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
                let mut when_true = self.lower_css_value_expression(&branch.consequent)?;
                let mut when_false = self.lower_css_value_expression(&branch.alternate)?;
                if self.expr_arena.types[when_true.0 as usize]
                    != self.expr_arena.types[when_false.0 as usize]
                {
                    when_true = self.css_scalar_string(when_true, branch.consequent.span());
                    when_false = self.css_scalar_string(when_false, branch.alternate.span());
                }
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

    /// CSS scalar branches can mix literal strings with typed angles/lengths/numbers.
    /// Keep ordinary typed conditionals strict; coerce only at the CSS consumer.
    fn css_scalar_string(&mut self, expr: ExprId, span: Span) -> ExprId {
        use valle_motion::expr::ExprType;
        if matches!(
            self.expr_arena.types[expr.0 as usize],
            Some(ExprType::Number | ExprType::Length | ExprType::Angle | ExprType::Color)
        ) {
            self.push(
                Expr::Template {
                    parts: vec![valle_motion::expr::TemplatePart::Expr { expr }],
                },
                span,
            )
        } else {
            expr
        }
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

    /// Auto blur derives screen-space velocity from nearby output-time layouts. Explicit velocity
    /// remains a frame-pure spatial shutter approximation in pixels per frame.
    pub(super) fn lower_node_motion_blur_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        if matches!(strip_parens(expression), Expression::StringLiteral(value) if value.value == "auto")
        {
            self.extra_capabilities
                .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
            return Some(vec![
                StyleBinding {
                    property: "motion-velocity-blur-auto".into(),
                    value: StyleValue::Static {
                        value: MotionValue::Bool(true),
                    },
                },
                StyleBinding {
                    property: "motion-velocity-blur-shutter".into(),
                    value: StyleValue::Static {
                        value: MotionValue::Number(180.0),
                    },
                },
            ]);
        }
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style.motionBlur must be \"auto\" or motionBlur(velocityPoint, shutterAngle?)",
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
                    color_space: valle_draw::program::GradientInterpolation::Srgb,
                    extrapolate_left: Extrapolation::Extend,
                    extrapolate_right: Extrapolation::Extend,
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
        // Preserve spring overshoot in both directions. Only prevent a size crossing zero from
        // reflecting the subtree; translation continues along the authored trajectory.
        let non_negative = |this: &mut Self, input: ExprId| {
            let zero = this.push(
                Expr::Const {
                    value: MotionValue::Number(0.0),
                },
                span,
            );
            let negative = this.push(
                Expr::Compare {
                    op: CompareOp::Lt,
                    lhs: input,
                    rhs: zero,
                },
                span,
            );
            this.push(
                Expr::Select {
                    condition: negative,
                    when_true: zero,
                    when_false: input,
                },
                span,
            )
        };
        let scale_x = non_negative(self, scale_x);
        let scale_y = non_negative(self, scale_y);
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
