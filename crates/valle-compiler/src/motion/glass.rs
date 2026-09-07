//! G3.1 Motion Glass JSX lowering: `<Glass>` / `<GlassField>` → normalized typed Artifact
//! nodes with defaults expanded.
//!
//! G4.7: production admission is open — the production compile entry lowers both tags and
//! declares `motion-glass` (see `tests/motion_glass.rs`); the old admission gate and the
//! test-only fixture compile entry were removed with the admission commit.
//! `MotionGlassNotAdmitted` remains declared in `valle-motion::diag` as a historical
//! diagnostic code for pre-admission artifacts (as-built record, not a live gate).

use super::*;
use valle_motion::glass::{
    GlassCharacter, GlassDriveBinding, GlassEnvironmentBinding, GlassFieldMotionBinding,
    GlassFieldNode, GlassForegroundIntent, GlassForegroundTone, GlassLightBinding, GlassLightSpace,
    GlassMaterialBinding, GlassMergeIntent, GlassNode, GlassShapeBinding,
    GlassSurfaceMotionBinding,
};
use valle_motion::glass::{GlassFieldId, GlassSurfaceId};

#[derive(Default)]
pub(super) struct GlassProps {
    pub surface_id: Option<String>,
    pub field_id: Option<String>,
    pub shape_kind: Option<String>,
    pub shape_radius: Option<NumberValue>,
    pub shape_path: Option<PathValue>,
    pub material: Option<GlassMaterialBinding>,
    pub character: Option<String>,
    pub intensity: Option<NumberValue>,
    pub settle: Option<f64>,
    pub drive_translation: Option<PointValue>,
    pub drive_pressure: Option<NumberValue>,
    pub drive_twist: Option<NumberValue>,
    pub presence: Option<NumberValue>,
    pub tone: Option<String>,
    pub protection: Option<NumberValue>,
    pub merge_distance: Option<f64>,
    pub environment_authored: bool,
    pub light_direction: Option<PointValue>,
    pub light_elevation: Option<NumberValue>,
    pub light_intensity: Option<NumberValue>,
    pub light_space: Option<String>,
    /// G3.6 compile-time UI usage lint: accepted, validated, and dropped — it never enters the
    /// Artifact or the render ABI.
    pub usage_profile: Option<String>,
    pub usage_role: Option<String>,
}

/// Static value of a JSON-ish literal inside an object property.
fn object_property_name(property: &oxc::ast::ast::ObjectProperty<'_>) -> Option<String> {
    match &property.key {
        PropertyKey::StaticIdentifier(name) => Some(name.name.to_string()),
        PropertyKey::StringLiteral(name) => Some(name.value.to_string()),
        _ => None,
    }
}

impl<'s> Compiler<'s> {
    /// Static string from an object-property expression.
    fn glass_static_string(
        &mut self,
        expression: &Expression<'_>,
        span: Span,
        name: &str,
    ) -> Option<String> {
        self.eval_static(expression)
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
            })
    }

    /// Path value from an object-property expression (static string/`line([...])` or typed).
    fn glass_path_value(
        &mut self,
        expression: &Expression<'_>,
        span: Span,
        name: &str,
    ) -> Option<PathValue> {
        if let Some(value) = self.eval_static(expression) {
            if let Some(text) = value.as_str() {
                return parse_path_data(text)
                    .map(|value| PathValue::Static { value })
                    .or_else(|| {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!("{name} is not valid path data"),
                        );
                        None
                    });
            }
            if let Some(MotionValue::PathData(value)) = motion_value_from_json(&value) {
                return Some(PathValue::Static { value });
            }
        }
        let expr = self.lower_expr(expression)?;
        let policy = geometry_eval_policy(&self.expr_arena.values, expr).or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name} must produce PathData"),
            );
            None
        })?;
        Some(PathValue::Expr { expr, policy })
    }

    /// Object literal from an object-property expression.
    fn glass_object_literal<'a>(
        &mut self,
        expression: &'a Expression<'a>,
        span: Span,
        label: &str,
    ) -> Option<&'a ObjectExpression<'a>> {
        let Expression::ObjectExpression(object) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must be an object literal"),
            );
            return None;
        };
        Some(object)
    }

    /// Entry for glass-specific attributes from the JSX dispatch loop.
    pub(super) fn lower_glass_attribute(
        &mut self,
        name: &str,
        attribute: &'s oxc::ast::ast::JSXAttribute<'s>,
        props: &mut GlassProps,
    ) {
        let value = &attribute.value;
        let span = attribute.span();
        match name {
            "surfaceId" => {
                props.surface_id = self.attr_static_string(value, span, "surfaceId");
            }
            "fieldId" => {
                props.field_id = self.attr_static_string(value, span, "fieldId");
            }
            "shape" => {
                let Some(object) = self.attr_object_literal(value, span, "Glass shape") else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.span(),
                            "Glass shape spread is illegal",
                        );
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            "computed Glass shape names are illegal",
                        );
                        continue;
                    };
                    match property_name.as_str() {
                        "kind" => {
                            props.shape_kind = self.glass_static_string(
                                &property.value,
                                property.key.span(),
                                "shape.kind",
                            );
                        }
                        "radius" => {
                            props.shape_radius =
                                self.number_binding(&property.value, "shape.radius", |value| {
                                    value.is_finite() && value >= 0.0
                                });
                        }
                        "path" => {
                            props.shape_path = self.glass_path_value(
                                &property.value,
                                property.key.span(),
                                "shape.path",
                            );
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass shape property `{other}`"),
                        ),
                    }
                }
            }
            "material" => {
                let Some(object) = self.attr_object_literal(value, span, "Glass material") else {
                    return;
                };
                let mut material = GlassMaterialBinding::defaults();
                let mut saw = false;
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.span(),
                            "Glass material spread is illegal",
                        );
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            "computed Glass material names are illegal",
                        );
                        continue;
                    };
                    match property_name.as_str() {
                        "clarity" => {
                            material.clarity = self
                                .number_binding(&property.value, "material.clarity", |value| {
                                    value.is_finite() && (0.0..=1.0).contains(&value)
                                })
                                .unwrap_or(material.clarity);
                            saw = true;
                        }
                        "depth" => {
                            material.depth = self
                                .number_binding(&property.value, "material.depth", |value| {
                                    value.is_finite() && (0.0..=1.0).contains(&value)
                                })
                                .unwrap_or(material.depth);
                            saw = true;
                        }
                        "tint" => {
                            if let Some(value) =
                                self.color_binding(&property.value, "material.tint")
                            {
                                material.tint = value;
                                saw = true;
                            }
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass material property `{other}`"),
                        ),
                    }
                }
                if saw {
                    props.material = Some(material);
                }
            }
            "motion" => {
                let Some(object) = self.attr_object_literal(value, span, "Glass motion") else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.span(),
                            "Glass motion spread is illegal",
                        );
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            "computed Glass motion names are illegal",
                        );
                        continue;
                    };
                    match property_name.as_str() {
                        "character" => {
                            props.character = self.glass_static_string(
                                &property.value,
                                property.key.span(),
                                "motion.character",
                            );
                        }
                        "intensity" => {
                            props.intensity =
                                self.number_binding(&property.value, "motion.intensity", |value| {
                                    value.is_finite() && (0.0..=1.0).contains(&value)
                                });
                        }
                        "settle" => {
                            if let Some(value) = self
                                .eval_static(&property.value)
                                .and_then(|value| value.as_f64())
                            {
                                props.settle = Some(value);
                            } else {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    property.key.span(),
                                    "motion.settle must be a static finite number",
                                );
                            }
                        }
                        "drive" => {
                            let Some(drive) = self.glass_object_literal(
                                &property.value,
                                property.key.span(),
                                "Glass drive",
                            ) else {
                                continue;
                            };
                            for drive_property in &drive.properties {
                                let ObjectPropertyKind::ObjectProperty(drive_property) =
                                    drive_property
                                else {
                                    continue;
                                };
                                let Some(drive_name) = object_property_name(drive_property) else {
                                    continue;
                                };
                                match drive_name.as_str() {
                                    "translation" => {
                                        props.drive_translation = self.point_binding(
                                            &drive_property.value,
                                            "motion.drive.translation",
                                        );
                                    }
                                    "pressure" => {
                                        props.drive_pressure = self.number_binding(
                                            &drive_property.value,
                                            "motion.drive.pressure",
                                            |value| value.is_finite(),
                                        );
                                    }
                                    "twist" => {
                                        props.drive_twist = self.number_binding(
                                            &drive_property.value,
                                            "motion.drive.twist",
                                            |value| value.is_finite(),
                                        );
                                    }
                                    other => self.illegal(
                                        DiagCode::GrammarForbidden,
                                        drive_property.key.span(),
                                        format!("unknown Glass drive property `{other}`"),
                                    ),
                                }
                            }
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass motion property `{other}`"),
                        ),
                    }
                }
            }
            "presence" => {
                if let Some(expression) = attr_expression(value) {
                    props.presence = self.number_binding(expression, "presence", |value| {
                        value.is_finite() && (0.0..=1.0).contains(&value)
                    });
                } else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "presence must be a number expression",
                    );
                }
            }
            "foreground" => {
                let Some(object) = self.attr_object_literal(value, span, "Glass foreground") else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        continue;
                    };
                    match property_name.as_str() {
                        "tone" => {
                            props.tone = self.glass_static_string(
                                &property.value,
                                property.key.span(),
                                "foreground.tone",
                            );
                        }
                        "protection" => {
                            props.protection = self.number_binding(
                                &property.value,
                                "foreground.protection",
                                |value| value.is_finite() && (0.0..=1.0).contains(&value),
                            );
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass foreground property `{other}`"),
                        ),
                    }
                }
            }
            "merge" => {
                let Some(object) = self.attr_object_literal(value, span, "GlassField merge") else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        continue;
                    };
                    if property_name == "distance" {
                        if let Some(value) = self
                            .eval_static(&property.value)
                            .and_then(|value| value.as_f64())
                        {
                            props.merge_distance = Some(value);
                        } else {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                property.key.span(),
                                "merge.distance must be a static finite number",
                            );
                        }
                    } else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown GlassField merge property `{property_name}`"),
                        );
                    }
                }
            }
            "environment" => {
                props.environment_authored = true;
                let Some(object) = self.attr_object_literal(value, span, "Glass environment")
                else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        continue;
                    };
                    match property_name.as_str() {
                        "light" => {
                            let Some(light) = self.glass_object_literal(
                                &property.value,
                                property.key.span(),
                                "Glass light",
                            ) else {
                                continue;
                            };
                            for light_property in &light.properties {
                                let ObjectPropertyKind::ObjectProperty(light_property) =
                                    light_property
                                else {
                                    continue;
                                };
                                let Some(light_name) = object_property_name(light_property) else {
                                    continue;
                                };
                                match light_name.as_str() {
                                    "direction" => {
                                        props.light_direction = self.point_binding(
                                            &light_property.value,
                                            "environment.light.direction",
                                        );
                                    }
                                    "elevation" => {
                                        props.light_elevation = self.number_binding(
                                            &light_property.value,
                                            "environment.light.elevation",
                                            |value| {
                                                value.is_finite() && (0.0..=1.0).contains(&value)
                                            },
                                        );
                                    }
                                    "intensity" => {
                                        props.light_intensity = self.number_binding(
                                            &light_property.value,
                                            "environment.light.intensity",
                                            |value| {
                                                value.is_finite() && (0.0..=1.0).contains(&value)
                                            },
                                        );
                                    }
                                    "space" => {
                                        props.light_space = self.glass_static_string(
                                            &light_property.value,
                                            light_property.key.span(),
                                            "environment.light.space",
                                        );
                                    }
                                    other => self.illegal(
                                        DiagCode::GrammarForbidden,
                                        light_property.key.span(),
                                        format!("unknown Glass light property `{other}`"),
                                    ),
                                }
                            }
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass environment property `{other}`"),
                        ),
                    }
                }
            }
            "usage" => {
                // G3.6: compile-time UI usage lint. Accepted and validated, then dropped —
                // usage never enters the Artifact or the render ABI.
                let Some(object) = self.attr_object_literal(value, span, "Glass usage") else {
                    return;
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    let Some(property_name) = object_property_name(property) else {
                        continue;
                    };
                    match property_name.as_str() {
                        "profile" => {
                            let value = self.glass_static_string(
                                &property.value,
                                property.key.span(),
                                "usage.profile",
                            );
                            match value.as_deref() {
                                Some("interface") => props.usage_profile = value,
                                Some(other) => self.illegal(
                                    DiagCode::GrammarForbidden,
                                    property.key.span(),
                                    format!("usage.profile must be \"interface\", got `{other}`"),
                                ),
                                None => {}
                            }
                        }
                        "role" => {
                            let value = self.glass_static_string(
                                &property.value,
                                property.key.span(),
                                "usage.role",
                            );
                            match value.as_deref() {
                                Some("control" | "navigation" | "overlay") => props.usage_role = value,
                                Some(other) => self.illegal(
                                    DiagCode::GrammarForbidden,
                                    property.key.span(),
                                    format!("usage.role must be control, navigation or overlay, got `{other}`"),
                                ),
                                None => {}
                            }
                        }
                        other => self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("unknown Glass usage property `{other}`"),
                        ),
                    }
                }
            }
            other => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("unknown Glass attribute `{other}`"),
            ),
        }
    }

    /// Finishes the Glass/GlassField node kind with defaults expanded.
    pub(super) fn finish_glass_kind(
        &mut self,
        kind_tag: &str,
        props: &GlassProps,
        span: Span,
    ) -> Option<NodeKind> {
        match kind_tag {
            "glass" => {
                let surface_id = props.surface_id.as_deref().unwrap_or("");
                if surface_id.is_empty() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "<Glass> requires a non-empty static surfaceId",
                    );
                    return None;
                }
                let shape = match props.shape_kind.as_deref() {
                    Some("circle") => GlassShapeBinding::Circle,
                    Some("capsule") => GlassShapeBinding::Capsule,
                    Some("continuousRect") => GlassShapeBinding::ContinuousRect {
                        radius: props
                            .shape_radius
                            .clone()
                            .unwrap_or(NumberValue::Static { value: 0.0 }),
                    },
                    Some("path") => {
                        let Some(path) = props.shape_path.clone() else {
                            self.illegal(DiagCode::GrammarForbidden, span, "Glass shape.kind \"path\" requires a `path` property (closed polygon)");
                            return None;
                        };
                        GlassShapeBinding::Path {
                            path,
                            reveal_origin: None,
                        }
                    }
                    Some(other) => {
                        self.illegal(DiagCode::GrammarForbidden, span, format!("Glass shape.kind must be circle, capsule or continuousRect, got `{other}`"));
                        return None;
                    }
                    None => {
                        self.illegal(DiagCode::GrammarForbidden, span, "<Glass> requires shape={{ kind: \"circle\" | \"continuousRect\" | \"capsule\" }}");
                        return None;
                    }
                };
                let character = match props.character.as_deref() {
                    Some("responsive") => Some(GlassCharacter::Responsive),
                    Some("fluid") => Some(GlassCharacter::Fluid),
                    Some("viscous") => Some(GlassCharacter::Viscous),
                    Some("elastic") => Some(GlassCharacter::Elastic),
                    Some(other) => {
                        self.illegal(DiagCode::GrammarForbidden, span, format!("motion.character must be responsive|fluid|viscous|elastic, got `{other}`"));
                        return None;
                    }
                    None => None,
                };
                let node = GlassNode {
                    surface_id: GlassSurfaceId::new(surface_id).ok()?,
                    field_id: props
                        .field_id
                        .clone()
                        .and_then(|id| GlassFieldId::new(id).ok()),
                    shape,
                    // Scope normalization runs after the complete JSX tree exists. Until then
                    // these options preserve whether the author actually supplied an
                    // independent-only property; injecting defaults here made it impossible to
                    // distinguish a legal field member from an illegal override.
                    material: props.material.clone(),
                    environment: props.environment_authored.then(|| GlassEnvironmentBinding {
                        light: GlassLightBinding {
                            direction: props.light_direction.clone().unwrap_or_else(|| {
                                PointValue::Static {
                                    value: Point::new(-0.35, -0.94),
                                }
                            }),
                            elevation: props
                                .light_elevation
                                .clone()
                                .unwrap_or(NumberValue::Static { value: 0.55 }),
                            intensity: props
                                .light_intensity
                                .clone()
                                .unwrap_or(NumberValue::Static { value: 0.65 }),
                            space: match props.light_space.as_deref() {
                                Some("world") => GlassLightSpace::World,
                                Some("screen") | None => GlassLightSpace::Screen,
                                Some(other) => {
                                    self.illegal(DiagCode::GrammarForbidden, span, format!("environment.light.space must be screen or world, got `{other}`"));
                                    GlassLightSpace::Screen
                                }
                            },
                        },
                    }),
                    motion: GlassSurfaceMotionBinding {
                        character,
                        intensity: props
                            .intensity
                            .clone()
                            .unwrap_or(NumberValue::Static { value: 0.60 }),
                        settle: props.settle,
                        drive: GlassDriveBinding {
                            translation: props.drive_translation.clone(),
                            pressure: props.drive_pressure.clone(),
                            twist: props.drive_twist.clone(),
                        },
                    },
                    presence: props
                        .presence
                        .clone()
                        .unwrap_or(NumberValue::Static { value: 1.0 }),
                    foreground: GlassForegroundIntent {
                        tone: match props.tone.as_deref() {
                            Some("auto") => GlassForegroundTone::Auto,
                            Some("light") => GlassForegroundTone::Light,
                            Some("dark") => GlassForegroundTone::Dark,
                            Some("none") => GlassForegroundTone::None,
                            Some(other) => {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    span,
                                    format!(
                                        "foreground.tone must be auto|light|dark|none, got `{other}`"
                                    ),
                                );
                                return None;
                            }
                            None => GlassForegroundTone::Auto,
                        },
                        protection: props
                            .protection
                            .clone()
                            .unwrap_or(NumberValue::Static { value: 0.65 }),
                    },
                };
                Some(NodeKind::Glass(node))
            }
            "glass-field" => {
                let field_id = props.field_id.as_deref().unwrap_or("");
                if field_id.is_empty() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "<GlassField> requires a non-empty static fieldId",
                    );
                    return None;
                }
                let character = match props.character.as_deref() {
                    Some("responsive") => GlassCharacter::Responsive,
                    Some("fluid") => GlassCharacter::Fluid,
                    Some("viscous") => GlassCharacter::Viscous,
                    Some("elastic") => GlassCharacter::Elastic,
                    Some(other) => {
                        self.illegal(DiagCode::GrammarForbidden, span, format!("motion.character must be responsive|fluid|viscous|elastic, got `{other}`"));
                        return None;
                    }
                    None => GlassCharacter::DEFAULT,
                };
                let node = GlassFieldNode {
                    field_id: GlassFieldId::new(field_id).ok()?,
                    material: props
                        .material
                        .clone()
                        .unwrap_or_else(GlassMaterialBinding::defaults),
                    motion: GlassFieldMotionBinding {
                        character,
                        intensity: props
                            .intensity
                            .clone()
                            .unwrap_or(NumberValue::Static { value: 0.60 }),
                        settle: props.settle.unwrap_or(0.32),
                        drive: GlassDriveBinding {
                            translation: props.drive_translation.clone(),
                            pressure: props.drive_pressure.clone(),
                            twist: props.drive_twist.clone(),
                        },
                    },
                    merge: GlassMergeIntent {
                        distance: props.merge_distance.unwrap_or(24.0),
                    },
                    environment: GlassEnvironmentBinding {
                        light: GlassLightBinding {
                            direction: props.light_direction.clone().unwrap_or_else(|| {
                                PointValue::Static {
                                    value: Point::new(-0.35, -0.94),
                                }
                            }),
                            elevation: props
                                .light_elevation
                                .clone()
                                .unwrap_or(NumberValue::Static { value: 0.55 }),
                            intensity: props
                                .light_intensity
                                .clone()
                                .unwrap_or(NumberValue::Static { value: 0.65 }),
                            space: match props.light_space.as_deref() {
                                Some("world") => GlassLightSpace::World,
                                Some("screen") | None => GlassLightSpace::Screen,
                                Some(other) => {
                                    self.illegal(DiagCode::GrammarForbidden, span, format!("environment.light.space must be screen or world, got `{other}`"));
                                    return None;
                                }
                            },
                        },
                    },
                };
                Some(NodeKind::GlassField(node))
            }
            _ => None,
        }
    }
}

/// JSX attribute expression unwrapping for number bindings.
fn attr_expression<'a>(value: &'a Option<JSXAttributeValue<'a>>) -> Option<&'a Expression<'a>> {
    match value {
        Some(JSXAttributeValue::ExpressionContainer(container)) => {
            container.expression.as_expression()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::compile_motion;
    use valle_motion::glass::MOTION_GLASS_CAPABILITY;

    fn compile_glass_for_test(source: &str) -> Result<SceneArtifact, Vec<CompilerDiagnostic>> {
        compile_motion(source).map(|compiled| compiled.artifact)
    }

    #[test]
    fn glass_circle_lowers_with_expanded_defaults() {
        let source = r##"
export const component = "glass-circle";
export default function Scene() {
  return (
    <Glass surfaceId="hero-lens" shape={{ kind: "circle" }}
      style={{ width: 420, height: 180 }} />
  );
}
"##;
        let artifact = compile_glass_for_test(source).unwrap();
        let NodeKind::Glass(glass) = &artifact.nodes[1].kind else {
            panic!("expected Glass node");
        };
        assert_eq!(glass.surface_id.as_str(), "hero-lens");
        assert_eq!(glass.shape, GlassShapeBinding::Circle);
        assert!(glass.material.is_some(), "defaults must be expanded");
        assert_eq!(glass.motion.settle, Some(0.32));
        assert_eq!(glass.presence, NumberValue::Static { value: 1.0 });
    }

    #[test]
    fn glass_full_props_lower() {
        let source = r##"
export const component = "glass-full";
export default function Scene() {
  return (
    <Glass surfaceId="lens"
      shape={{ kind: "continuousRect", radius: 44 }}
      material={{ clarity: 0.78, depth: 0.62, tint: "#B9D7FF18" }}
      environment={{ light: { direction: point(0.6, -0.8), elevation: 0.4, intensity: 0.9, space: "world" } }}
      motion={{ character: "elastic", intensity: 0.74, settle: 0.42,
               drive: { translation: point(8, 0), pressure: 12, twist: 0.2 } }}
      presence={0.5}
      foreground={{ tone: "auto", protection: 0.72 }}
      style={{ width: 420, height: 180 }} />
  );
}
"##;
        let artifact = compile_glass_for_test(source).unwrap();
        let NodeKind::Glass(glass) = &artifact.nodes[1].kind else {
            panic!("expected Glass node");
        };
        let material = glass.material.as_ref().unwrap();
        assert_eq!(material.clarity, NumberValue::Static { value: 0.78 });
        assert_eq!(glass.motion.character, Some(GlassCharacter::Elastic));
        assert!(glass.motion.drive.translation.is_some());
        assert_eq!(
            glass.environment.as_ref().unwrap().light.space,
            GlassLightSpace::World
        );
    }

    #[test]
    fn glass_field_lowers() {
        let source = r##"
export const component = "glass-field";
export default function Scene() {
  return (
    <GlassField fieldId="orbit" material={{ clarity: 0.82 }}
      motion={{ character: "fluid", intensity: 0.68 }}
      merge={{ distance: 36 }}
      environment={{ light: { direction: point(-0.35, -0.94), elevation: 0.55, intensity: 0.7, space: "screen" } }}>
      <Glass surfaceId="orbit-left" shape={{ kind: "circle" }}
        style={{ width: 100, height: 100 }} />
      <Glass surfaceId="orbit-right" shape={{ kind: "circle" }}
        style={{ width: 100, height: 100 }} />
    </GlassField>
  );
}
"##;
        let artifact = compile_glass_for_test(source).unwrap();
        let NodeKind::GlassField(field) = &artifact.nodes[1].kind else {
            panic!("expected GlassField node");
        };
        assert_eq!(field.field_id.as_str(), "orbit");
        assert_eq!(field.merge.distance, 36.0);
        assert_eq!(field.environment.light.space, GlassLightSpace::Screen);
        let glass_count = artifact
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Glass(_)))
            .count();
        assert_eq!(glass_count, 2);
        for node in &artifact.nodes {
            if let NodeKind::Glass(glass) = &node.kind {
                assert_eq!(glass.field_id.as_ref().map(|id| id.as_str()), Some("orbit"));
                assert!(glass.material.is_none());
                assert!(glass.environment.is_none());
                assert!(glass.motion.character.is_none());
                assert!(glass.motion.settle.is_none());
            }
        }
    }

    #[test]
    fn unsupported_backdrop_stability_is_rejected_instead_of_being_silently_dropped() {
        let source = r#"
export default function Scene() {
  return <Glass surfaceId="lens" shape={{ kind: "circle" }}
    environment={{ backdropStability: 0.16 }} />;
}
"#;
        let diagnostics = compile_glass_for_test(source).unwrap_err();
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unknown Glass environment property `backdropStability`")
        }));
    }

    #[test]
    fn member_field_id_is_derived_and_repeated_author_prop_is_rejected() {
        let source = r##"
export const component = "glass-field";
export default function Scene() {
  return (
    <GlassField fieldId="orbit">
      <Glass surfaceId="left" fieldId="orbit" shape={{ kind: "circle" }} />
    </GlassField>
  );
}
"##;
        let errors = compile_glass_for_test(source).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|diagnostic| diagnostic.message.contains("must not declare fieldId")),
            "{errors:?}"
        );
    }

    #[test]
    fn foreground_tone_contract_accepts_all_modes() {
        for tone in ["auto", "light", "dark", "none"] {
            let source = format!(
                r##"
export const component = "tone";
export default function Scene() {{
  return <Glass surfaceId="lens" shape={{{{ kind: "circle" }}}}
    foreground={{{{ tone: "{tone}" }}}} />;
}}
"##
            );
            compile_glass_for_test(&source).unwrap_or_else(|errors| panic!("{tone}: {errors:?}"));
        }
    }

    #[test]
    fn production_entry_compiles_glass_and_declares_capability() {
        // G4.7: production admission is open; compiled artifacts declare motion-glass.
        let source = r##"
export const component = "glass-open";
export default function Scene() {
  return <Glass surfaceId="hero-lens" shape={{ kind: "circle" }} />;
}
"##;
        let compiled = compile_motion(source).expect("production compile admits Glass");
        assert!(
            compiled
                .artifact
                .capability_set
                .names
                .contains(&MOTION_GLASS_CAPABILITY.to_string())
        );
        compiled.artifact.validate().expect("artifact validates");
    }

    // ---- G3.2 diagnostics: every author error points at the source span ----

    fn first_message(error: &[CompilerDiagnostic]) -> String {
        error
            .first()
            .map(|diagnostic| diagnostic.message.clone())
            .unwrap_or_default()
    }

    #[test]
    fn missing_surface_id_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return <Glass shape={{ kind: "circle" }} />;
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        assert!(first_message(&error).contains("surfaceId"), "{error:?}");
    }

    #[test]
    fn missing_shape_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return <Glass surfaceId="lens" />;
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        assert!(first_message(&error).contains("shape"), "{error:?}");
    }

    #[test]
    fn bad_shape_kind_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return <Glass surfaceId="lens" shape={{ kind: "blob" }} />;
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        assert!(first_message(&error).contains("blob"), "{error:?}");
    }

    #[test]
    fn unknown_glass_attribute_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return <Glass surfaceId="lens" shape={{ kind: "circle" }} shader="x" />;
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        assert!(
            error
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unknown Glass attribute")),
            "{error:?}"
        );
    }

    #[test]
    fn bad_character_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return (
    <Glass surfaceId="lens" shape={{ kind: "circle" }}
      motion={{ character: "watery" }} />
  );
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        assert!(first_message(&error).contains("watery"), "{error:?}");
    }

    #[test]
    fn settle_out_of_range_diagnostic() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return (
    <Glass surfaceId="lens" shape={{ kind: "circle" }}
      motion={{ settle: 3.0 }} />
  );
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        // settle=3.0 is outside [0.08, 1.20]: Artifact validation rejects at the fixture path.
        let message = error
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("settle"), "{message}");
    }

    #[test]
    fn duplicate_surface_id_rejected() {
        let source = r##"
export const component = "diag";
export default function Scene() {
  return (
    <Group>
      <Glass surfaceId="lens" shape={{ kind: "circle" }} style={{ left: 0, top: 0, width: 80, height: 80 }} />
      <Glass surfaceId="lens" shape={{ kind: "circle" }} style={{ left: 200, top: 0, width: 80, height: 80 }} />
    </Group>
  );
}
"##;
        let error = compile_glass_for_test(source).unwrap_err();
        let message = error
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            message.contains("surfaceId 'lens' appears twice"),
            "{message}"
        );
    }

    #[test]
    fn path_shape_lowers_with_closed_polygon() {
        let source = r##"
export const component = "path-glass";
export default function Scene() {
  return (
    <Glass surfaceId="plate" shape={{ kind: "path", path: line([point(40, 40), point(120, 40), point(120, 120), point(40, 120)]) }}
      style={{ width: 160, height: 160 }} />
  );
}
"##;
        let compiled = compile_motion(source).expect("path glass compiles");
        compiled.artifact.validate().expect("validates");
        let NodeKind::Glass(glass) = &compiled.artifact.nodes[1].kind else {
            panic!("glass node");
        };
        assert!(matches!(glass.shape, GlassShapeBinding::Path { .. }));
    }

    #[test]
    fn usage_lint_accepts_valid_and_rejects_unknown_role() {
        let valid = r##"
export const component = "usage";
export default function Scene() {
  return (
    <Glass surfaceId="lens" shape={{ kind: "circle" }}
      usage={{ profile: "interface", role: "control" }}
      style={{ width: 80, height: 80 }} />
  );
}
"##;
        // Valid usage compiles and never enters the Artifact (compile-time lint only).
        let artifact = compile_glass_for_test(valid).unwrap();
        let NodeKind::Glass(glass) = &artifact.nodes[1].kind else {
            panic!("glass node");
        };
        let _ = glass;
        let bad = r##"
export const component = "usage";
export default function Scene() {
  return (
    <Glass surfaceId="lens" shape={{ kind: "circle" }}
      usage={{ profile: "interface", role: "widget" }}
      style={{ width: 80, height: 80 }} />
  );
}
"##;
        let error = compile_glass_for_test(bad).unwrap_err();
        assert!(
            error
                .iter()
                .any(|diagnostic| diagnostic.message.contains("usage.role")),
            "{error:?}"
        );
    }
}
