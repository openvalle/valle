//! Unit/style lowering, transforms, and node-local effects.

use super::*;

impl<'s> Compiler<'s> {
    /// Lower per-unit text styles using a closed set of supported properties.
    pub(super) fn lower_unit_style(&mut self, object: &'s ObjectExpression<'s>) -> UnitStyle {
        let mut style = UnitStyle::default();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "perUnit spread is illegal; list every property explicitly",
                );
                continue;
            };
            let name = match &property.key {
                PropertyKey::StaticIdentifier(name) => name.name.to_string(),
                PropertyKey::StringLiteral(name) => name.value.to_string(),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.key.span(),
                        "computed perUnit property names are illegal",
                    );
                    continue;
                }
            };
            let value = &property.value;
            match name.as_str() {
                "opacity" => {
                    style.opacity =
                        self.number_binding(value, "perUnit opacity", |value| {
                            (0.0..=1.0).contains(&value)
                        })
                }
                "translate" => style.translate = self.point_binding(value, "perUnit translate"),
                "scale" => style.scale = self.point_binding(value, "perUnit scale"),
                "rotate" => {
                    style.rotate = self.number_binding(value, "perUnit rotate", f64::is_finite)
                }
                "color" => style.color = self.color_binding(value, "perUnit color"),
                _ => self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!(
                        "unknown perUnit property `{name}`; allowed: opacity, translate, scale, rotate, color"
                    ),
                ),
            }
        }
        style
    }

    /// Require split and perUnit together.
    pub(super) fn close_per_unit(
        &mut self,
        span: Span,
        split: Option<TextSplit>,
        style: Option<UnitStyle>,
    ) -> Option<Box<PerUnit>> {
        match (split, style) {
            (Some(split), Some(style)) if !style.is_empty() => {
                Some(Box::new(PerUnit { split, style }))
            }
            (Some(_), Some(_)) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "perUnit must bind at least one of opacity/translate/scale/rotate/color",
                );
                None
            }
            (Some(_), None) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "Text split requires perUnit",
                );
                None
            }
            (None, Some(_)) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "Text perUnit requires split",
                );
                None
            }
            (None, None) => None,
        }
    }

    pub(super) fn lower_style(
        &mut self,
        object: &super::style_objects::ResolvedStyle<'s>,
        path: &str,
        layout_id: Option<&str>,
    ) -> Vec<StyleBinding> {
        let diagnostic_start = self.diagnostics.len();
        let mut styles = Vec::new();
        let mut saw_transform = false;
        let mut saw_motion_path = false;
        let mut saw_motion_path_component = false;
        let mut saw_layout_transition = false;
        let has_authored_font_size = object.contains("font-size");
        let has_layout_transition = object.contains("layout-transition");
        let saved_scope = self.style_scope();
        for entry in &object.properties {
            self.restore_style_scope(&entry.scope);
            let property = entry.property;
            let name = &entry.name;
            let property_name = camel_to_kebab(&name);
            if property_name == "layout-transition" {
                if saw_layout_transition {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.layoutTransition may only be declared once",
                    );
                    continue;
                }
                saw_layout_transition = true;
                let Some(layout_id) = layout_id else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.layoutTransition requires a static layoutId attribute on the same View",
                    );
                    continue;
                };
                if let Some(mut bindings) =
                    self.lower_flip_style(&property.value, layout_id, property.span())
                {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if has_layout_transition
                && matches!(
                    property_name.as_str(),
                    "position"
                        | "left"
                        | "top"
                        | "width"
                        | "height"
                        | "transform"
                        | "translate"
                        | "scale"
                        | "rotate"
                        | "transform-origin"
                        | "motion-path"
                )
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!(
                        "style.layoutTransition owns `{property_name}`; move that geometry into defineLayoutStates"
                    ),
                );
                continue;
            }
            // Author `--*` / `var()` are a permanent rejection. Keep the original CSS so
            // artifact admission can report the same diagnostic as a direct style object.
            let static_css = self.eval_static(&property.value);
            let variable_reference = static_css
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .is_some_and(valle_motion::style::contains_variable);
            if property_name.starts_with("--") || variable_reference {
                let value = match static_css {
                    Some(serde_json::Value::String(value)) => MotionValue::Str(value),
                    Some(serde_json::Value::Number(value)) if property_name.starts_with("--") => {
                        MotionValue::Number(value.as_f64().unwrap_or(f64::NAN))
                    }
                    _ => {
                        self.illegal(DiagCode::GrammarForbidden, property.value.span(),
                            "CSS custom properties currently require a prepare-time string or finite number");
                        continue;
                    }
                };
                if matches!(
                    property_name.as_str(),
                    "transform" | "translate" | "rotate" | "transform-origin"
                ) {
                    if saw_motion_path {
                        self.illegal(DiagCode::GrammarForbidden, property.span(), "CSS variable transforms cannot be combined with motionPath on one node");
                        continue;
                    }
                    if property_name == "transform" {
                        saw_transform = true;
                    } else {
                        saw_motion_path_component = true;
                    }
                }
                styles.push(StyleBinding {
                    property: property_name,
                    value: StyleValue::Static { value },
                });
                continue;
            }
            if property_name == "transform" {
                if saw_motion_path {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.transform cannot be combined with style.motionPath on one node",
                    );
                    continue;
                }
                saw_transform = true;
                if let Some(mut bindings) = self.lower_transform_style(&property.value) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "project-quad" {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "style.projectQuad is not an author API; use perspective, transformStyle, ordered CSS 3D transform functions, and backfaceVisibility",
                );
                continue;
            }
            if property_name == "motion-path" {
                if saw_motion_path {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.motionPath may only be declared once on a node",
                    );
                    continue;
                }
                if saw_transform || saw_motion_path_component {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.motionPath cannot be combined with transform, translate, rotate, or transformOrigin on one node",
                    );
                    continue;
                }
                saw_motion_path = true;
                if let Some(mut bindings) = self.lower_motion_path_style(&property.value) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "displacement" {
                if let Some(mut bindings) = self.lower_displacement_style(&property.value, false) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "backdrop-displacement" {
                if let Some(mut bindings) = self.lower_displacement_style(&property.value, true) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "motion-blur" {
                if let Some(mut bindings) = self.lower_node_motion_blur_style(&property.value) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "perspective-origin" {
                if let Some(mut bindings) = self.lower_perspective_origin_style(&property.value) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if property_name == "transform-style" {
                let value = self.eval_static(&property.value);
                if !matches!(value, Some(serde_json::Value::String(ref value)) if value == "preserve-3d")
                {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "transformStyle must be the static string `preserve-3d`",
                    );
                    continue;
                }
                self.extra_capabilities
                    .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
                styles.push(StyleBinding {
                    property: property_name,
                    value: StyleValue::Static {
                        value: MotionValue::Enum("preserve-3d".into()),
                    },
                });
                continue;
            }
            if property_name == "backface-visibility" {
                let value = self.eval_static(&property.value);
                let Some(value @ ("hidden" | "visible")) =
                    value.as_ref().and_then(|value| value.as_str())
                else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "backfaceVisibility must be the static string `hidden` or `visible`",
                    );
                    continue;
                };
                self.extra_capabilities
                    .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
                styles.push(StyleBinding {
                    property: property_name,
                    value: StyleValue::Static {
                        value: MotionValue::Enum(value.into()),
                    },
                });
                continue;
            }
            if matches!(property_name.as_str(), "filter" | "backdrop-filter") {
                if let Some(value) = self.lower_css_filter_style(&property.value, &property_name) {
                    styles.push(StyleBinding {
                        property: property_name,
                        value,
                    });
                }
                continue;
            }
            if property_name == "fit-text" {
                if has_authored_font_size {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.fitText owns fontSize; do not declare both on one Text node",
                    );
                    continue;
                }
                if let Some(mut bindings) = self.lower_fit_text_style(&property.value) {
                    styles.append(&mut bindings);
                }
                continue;
            }
            if matches!(property_name.as_str(), "scale-x" | "scale-y") {
                let axis = usize::from(property_name == "scale-y");
                let scalar = match static_motion_value(&property.value) {
                    Some(MotionValue::Number(value)) => StyleValue::Static {
                        value: MotionValue::Point(if axis == 0 {
                            Point::new(value, 1.0)
                        } else {
                            Point::new(1.0, value)
                        }),
                    },
                    Some(_) => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            format!("style.{name} requires a unitless Number"),
                        );
                        continue;
                    }
                    None => match self.fold_to_value(&property.value) {
                        Some(MotionValue::Number(value)) => StyleValue::Static {
                            value: MotionValue::Point(if axis == 0 {
                                Point::new(value, 1.0)
                            } else {
                                Point::new(1.0, value)
                            }),
                        },
                        Some(_) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                property.value.span(),
                                format!("style.{name} requires a unitless Number"),
                            );
                            continue;
                        }
                        None => {
                            let scalar = match self.lower_expr(&property.value) {
                                Some(expr) => expr,
                                None => continue,
                            };
                            let one = self.push(
                                Expr::Const {
                                    value: MotionValue::Number(1.0),
                                },
                                property.value.span(),
                            );
                            StyleValue::Expr {
                                expr: self.push(
                                    if axis == 0 {
                                        Expr::MakePoint { x: scalar, y: one }
                                    } else {
                                        Expr::MakePoint { x: one, y: scalar }
                                    },
                                    property.value.span(),
                                ),
                            }
                        }
                    },
                };
                self.extra_capabilities
                    .insert(TRANSFORM_SCALE2D_CAPABILITY.to_owned());
                styles.push(StyleBinding {
                    property: "scale".into(),
                    value: scalar,
                });
                continue;
            }
            if matches!(
                property_name.as_str(),
                "translate" | "rotate" | "transform-origin"
            ) {
                if saw_motion_path {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "style.motionPath cannot be combined with translate, rotate, or transformOrigin on one node",
                    );
                    continue;
                }
                saw_motion_path_component = true;
            }
            let spec = valle_motion::style::property_spec(&property_name);
            if let Err(error) = spec.admit() {
                let at = property.value.span();
                let value = string_literal(&property.value).unwrap_or_else(|| {
                    self.source
                        .get(at.start as usize..at.end as usize)
                        .unwrap_or("")
                        .to_owned()
                });
                let issue = valle_motion::style::StyleIssue::admission(error, &value);
                // The label already names the property, so the issue contributes its detail only;
                // repeating the property would bury the replacement.
                let mut diagnostic = diagnostic_at(
                    self.source,
                    issue.code(),
                    property.key.span(),
                    format!("style.{name}: {}", issue.detail()),
                );
                diagnostic.node_path = Some(path.into());
                diagnostic.style = Some(issue);
                self.push_diagnostic(diagnostic);
                continue;
            }
            if spec.lowering == valle_motion::style::PropertyLowering::Motion
                && property_name.starts_with("motion-")
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    "internal Motion bindings must be produced by their authoring helpers",
                );
                continue;
            }
            // Try literals, then compile-time folding, then runtime lowering. Folded values must
            // become Static entries without redundant arena nodes.
            // Preserve ambiguous CSS keywords/family names and percentage opacity in their
            // property grammar: generic Motion inference treats `none` as a paint color.
            // Other prepared values retain their typed representation.
            let css_literal = (spec.lowering == valle_motion::style::PropertyLowering::Css
                || matches!(property_name.as_str(), "translate" | "rotate" | "scale"))
            .then(|| self.eval_static(&property.value))
            .flatten()
            .and_then(|value| {
                value
                    .as_str()
                    .filter(|text| {
                        matches!(
                            property_name.as_str(),
                            "translate" | "rotate" | "scale" | "font-family" | "opacity"
                        ) || text.trim().eq_ignore_ascii_case("none")
                    })
                    .map(|text| MotionValue::Str(text.to_owned()))
            });
            let mut value = match css_literal.or_else(|| static_motion_value(&property.value)) {
                Some(value) => StyleValue::Static { value },
                None => match self.fold_to_value(&property.value) {
                    Some(value) => StyleValue::Static { value },
                    None => {
                        match if matches!(property_name.as_str(), "background" | "background-image")
                        {
                            self.lower_css_value_expression(&property.value)
                        } else {
                            self.lower_expr(&property.value)
                        } {
                            Some(expr) => StyleValue::Expr { expr },
                            None => continue,
                        }
                    }
                },
            };
            if property_name == "font-family" {
                match &mut value {
                    StyleValue::Static {
                        value: MotionValue::Str(family),
                    } => {
                        if let Some(control_name) =
                            family.strip_prefix("asset://").map(str::to_owned)
                        {
                            let Some(control) = self.controls.assets.get(&control_name) else {
                                self.illegal(
                                    DiagCode::UnknownProp,
                                    property.value.span(),
                                    format!(
                                        "fontFamily references unknown font control `{control_name}`"
                                    ),
                                );
                                continue;
                            };
                            if control.kind != AssetKind::Font {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    property.value.span(),
                                    format!(
                                        "fontFamily control `{control_name}` must have kind font"
                                    ),
                                );
                                continue;
                            }
                            let Some(resource) = self
                                .resource_refs
                                .iter()
                                .find(|resource| resource.control == control_name)
                            else {
                                self.unsupported(
                                    property.value.span(),
                                    format!(
                                        "font control `{control_name}` is used by fontFamily but has no bound resource for this component instance"
                                    ),
                                );
                                continue;
                            };
                            *family = font_family_alias(&resource.content_hash);
                            self.used_font_controls.insert(control_name);
                            self.extra_capabilities
                                .insert(FONT_ASSET_CAPABILITY.to_owned());
                        }
                    }
                    StyleValue::Static { .. } => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.value.span(),
                            "fontFamily must be a static string",
                        );
                        continue;
                    }
                    StyleValue::Expr { .. } => {
                        self.unsupported(
                            property.value.span(),
                            "frame-varying fontFamily would change shaping and is not supported",
                        );
                        continue;
                    }
                }
            }
            if matches!(property_name.as_str(), "translate" | "transform-origin") {
                value = match value {
                    StyleValue::Static {
                        value: MotionValue::Point(point),
                    } => StyleValue::Static {
                        value: MotionValue::Length2(Length2::px(point.x, point.y)),
                    },
                    StyleValue::Static {
                        value: MotionValue::Vec2(vector),
                    } => StyleValue::Static {
                        value: MotionValue::Length2(Length2::px(vector.x, vector.y)),
                    },
                    StyleValue::Expr { expr } => StyleValue::Expr {
                        expr: self.push(Expr::ToLength2 { input: expr }, property.span()),
                    },
                    value => value,
                };
            }
            if property_name == "scale" {
                self.note_scale_binding(&value);
            }
            if property_name == "perspective" {
                self.extra_capabilities
                    .insert(CSS_3D_TRANSFORM_CAPABILITY.to_owned());
            }
            if !valle_motion::style::is_motion_property(&property_name)
                && let StyleValue::Static { value } = &value
                && let Err(reason) = valle_motion::style::parse_property(
                    &property_name,
                    &valle_motion::css_token(value),
                )
            {
                self.style_diagnostic(property.value.span(), Some(path), reason);
                continue;
            }
            styles.push(StyleBinding {
                property: property_name,
                value,
            });
        }
        self.restore_style_scope(&saved_scope);
        if styles
            .iter()
            .filter(|style| style.property == "scale")
            .count()
            > 1
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                object.span,
                "declare independent scale only once; do not mix style.scale with style.scaleX/scaleY",
            );
        }
        if styles.is_empty() && !object.properties.is_empty() && self.diagnostics.is_empty() {
            self.unsupported(
                object.span,
                format!("style at `{path}` did not lower to any supported value"),
            );
        }
        for diagnostic in &mut self.diagnostics[diagnostic_start..] {
            diagnostic.node_path.get_or_insert_with(|| path.to_owned());
        }
        styles
    }

    /// Bounded one-pass text fitting. Takumi computes the line scale directly from the fixed
    /// content box; there is no measure -> mutate -> remeasure feedback loop and no iteration.
    pub(super) fn lower_fit_text_style(
        &mut self,
        expression: &'s Expression<'s>,
    ) -> Option<Vec<StyleBinding>> {
        let value = self.eval_static(expression).or_else(|| {
            self.illegal(
                DiagCode::BuiltinRejected,
                expression.span(),
                "style.fitText must be the prepare-time result of fitText({ minFontSize, maxFontSize })",
            );
            None
        })?;
        let object = value.as_object().filter(|object| {
            object
                .get("__valleType")
                .and_then(serde_json::Value::as_str)
                == Some("fitText")
        });
        let Some(object) = object else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                "style.fitText must call fitText({ minFontSize, maxFontSize })",
            );
            return None;
        };
        let min = object
            .get("minFontSize")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)?;
        let max = object
            .get("maxFontSize")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= min)?;
        let limit = valle_motion::css_token(&MotionValue::Number(min / max * 100.0));
        Some(vec![
            StyleBinding {
                property: "font-size".into(),
                value: StyleValue::Static {
                    value: MotionValue::Number(max),
                },
            },
            StyleBinding {
                property: "text-fit".into(),
                value: StyleValue::Static {
                    value: MotionValue::Str(format!("shrink {limit}%")),
                },
            },
        ])
    }

    pub(super) fn note_scale_binding(&mut self, value: &StyleValue) {
        let point = match value {
            StyleValue::Static {
                value: MotionValue::Point(_),
            } => true,
            StyleValue::Expr { expr } => matches!(
                self.expr_arena.values.get(expr.0 as usize),
                Some(Expr::MakePoint { .. })
            ),
            _ => false,
        };
        if point {
            self.extra_capabilities
                .insert(TRANSFORM_SCALE2D_CAPABILITY.to_owned());
        }
    }
}
