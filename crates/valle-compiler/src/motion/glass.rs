//! `<Glass>` and `<GlassField>` JSX → typed Artifact nodes. A Glass lays out like a View; its
//! `path` and `fillRule` lower with the Clip attributes in `jsx.rs`.

use super::*;
use valle_motion::glass::{
    DEFAULT_PRESENCE, GlassFieldNode, GlassLightBinding, GlassMaterialBinding, GlassMergeIntent,
    GlassNode,
};

#[derive(Default)]
pub(super) struct GlassProps {
    pub material: Option<GlassMaterialBinding>,
    pub light: Option<GlassLightBinding>,
    pub presence: Option<NumberValue>,
    pub merge_distance: Option<f64>,
}

/// One field of a `material`, `light` or `merge` object: an expression in an object literal, or
/// a value of a static object the literal spreads or the attribute is.
enum Field<'s> {
    Expr(&'s Expression<'s>),
    Static(serde_json::Value),
}

fn object_property_name(property: &oxc::ast::ast::ObjectProperty<'_>) -> Option<String> {
    match &property.key {
        PropertyKey::StaticIdentifier(name) => Some(name.name.to_string()),
        PropertyKey::StringLiteral(name) => Some(name.value.to_string()),
        _ => None,
    }
}

fn non_negative(value: f64) -> bool {
    value >= 0.0
}

impl<'s> Compiler<'s> {
    /// The fields of an object attribute, in order, so a later one wins as in JavaScript. The
    /// attribute is an object literal, or a static object such as `glassMaterial(0.8, 0.5)`; a
    /// literal may spread static objects.
    fn glass_fields(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
        label: &str,
    ) -> Vec<(String, Field<'s>, Span)> {
        let mut fields = Vec::new();
        let Some(expression) = attr_expression(value) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must be an object"),
            );
            return fields;
        };
        let Expression::ObjectExpression(object) = peel_expr(expression) else {
            self.static_fields(expression, label, &mut fields);
            return fields;
        };
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    let Some(name) = object_property_name(property) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            property.key.span(),
                            format!("computed {label} names are illegal"),
                        );
                        continue;
                    };
                    fields.push((name, Field::Expr(&property.value), property.key.span()));
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    self.static_fields(&spread.argument, label, &mut fields);
                }
            }
        }
        fields
    }

    fn static_fields(
        &mut self,
        expression: &'s Expression<'s>,
        label: &str,
        fields: &mut Vec<(String, Field<'s>, Span)>,
    ) {
        let span = expression.span();
        if let Some(serde_json::Value::Object(object)) = self.eval_static(expression) {
            fields.extend(
                object
                    .into_iter()
                    .map(|(name, value)| (name, Field::Static(value), span)),
            );
        } else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must be an object literal or a static object"),
            );
        }
    }

    fn glass_number(
        &mut self,
        field: Field<'s>,
        span: Span,
        label: &str,
        valid: impl Fn(f64) -> bool,
    ) -> Option<NumberValue> {
        let value = match field {
            Field::Expr(expression) => return self.number_binding(expression, label, valid),
            Field::Static(value) => value.as_f64(),
        };
        match value {
            Some(value) if value.is_finite() && valid(value) => Some(NumberValue::Static { value }),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("{label} has an invalid static value"),
                );
                None
            }
        }
    }

    fn glass_color(&mut self, field: Field<'s>, span: Span, label: &str) -> Option<ColorValue> {
        let value = match field {
            Field::Expr(expression) => return self.color_binding(expression, label),
            Field::Static(value) => value,
        };
        if let Some(MotionValue::Color(value)) = motion_value_from_json(&value) {
            return Some(ColorValue::Static { value });
        }
        self.illegal(
            DiagCode::GrammarForbidden,
            span,
            format!("{label} must produce Color"),
        );
        None
    }

    fn glass_point(&mut self, field: Field<'s>, span: Span, label: &str) -> Option<PointValue> {
        let value = match field {
            Field::Expr(expression) => return self.point_binding(expression, label),
            Field::Static(value) => value,
        };
        if let Some(MotionValue::Point(value)) = motion_value_from_json(&value) {
            return Some(PointValue::Static { value });
        }
        self.illegal(
            DiagCode::GrammarForbidden,
            span,
            format!("{label} must produce Point"),
        );
        None
    }

    /// Entry for glass-specific attributes from the JSX dispatch loop.
    pub(super) fn lower_glass_attribute(
        &mut self,
        kind_tag: &str,
        name: &str,
        attribute: &'s oxc::ast::ast::JSXAttribute<'s>,
        props: &mut GlassProps,
    ) {
        let value = &attribute.value;
        let span = attribute.span();
        let element = if kind_tag == "glass" {
            "Glass"
        } else {
            "GlassField"
        };
        match name {
            "material" => {
                let fields = self.glass_fields(value, span, "Glass material");
                props.material = Some(self.glass_material(fields));
            }
            "light" => {
                let fields = self.glass_fields(value, span, "Glass light");
                props.light = Some(self.glass_light(fields));
            }
            "presence" if kind_tag == "glass" => {
                if let Some(expression) = attr_expression(value) {
                    props.presence = self.number_binding(expression, "presence", |value| {
                        (0.0..=1.0).contains(&value)
                    });
                } else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "presence must be a number expression",
                    );
                }
            }
            "merge" if kind_tag == "glass-field" => {
                for (name, field, span) in self.glass_fields(value, span, "GlassField merge") {
                    if name != "distance" {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            format!("unknown GlassField merge property `{name}`"),
                        );
                        continue;
                    }
                    let distance = match field {
                        Field::Expr(expression) => self
                            .eval_static(expression)
                            .and_then(|value| value.as_f64()),
                        Field::Static(value) => value.as_f64(),
                    };
                    if distance.is_some() {
                        props.merge_distance = distance;
                    } else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "merge.distance must be a static finite number",
                        );
                    }
                }
            }
            "shape" => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "Glass `shape` was removed: a Glass is its box rounded by `borderRadius`, or its `path`",
            ),
            "surfaceId" | "fieldId" => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("Glass `{name}` was removed: a GlassField's members are the Glass inside it"),
            ),
            "environment" => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "Glass `environment` was removed: write `light={{ direction, elevation, intensity }}`",
            ),
            other => self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("unknown {element} attribute `{other}`"),
            ),
        }
    }

    fn glass_material(&mut self, fields: Vec<(String, Field<'s>, Span)>) -> GlassMaterialBinding {
        let mut material = GlassMaterialBinding::defaults();
        for (name, field, span) in fields {
            let label = format!("material.{name}");
            match name.as_str() {
                "bevel" => material.bevel = self.glass_number(field, span, &label, non_negative),
                "refraction" => {
                    material.refraction = self.glass_number(field, span, &label, non_negative);
                }
                "dispersion" => {
                    if let Some(value) = self.glass_number(field, span, &label, non_negative) {
                        material.dispersion = value;
                    }
                }
                "frost" => {
                    if let Some(value) = self.glass_number(field, span, &label, non_negative) {
                        material.frost = value;
                    }
                }
                "tint" => {
                    if let Some(value) = self.glass_color(field, span, &label) {
                        material.tint = value;
                    }
                }
                "clarity" | "depth" => self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "{label} was removed: write bevel, refraction and frost (the Motion docs show a glassMaterial() helper)"
                    ),
                ),
                other => self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("unknown Glass material property `{other}`"),
                ),
            }
        }
        material
    }

    fn glass_light(&mut self, fields: Vec<(String, Field<'s>, Span)>) -> GlassLightBinding {
        let mut light = GlassLightBinding::defaults();
        for (name, field, span) in fields {
            let label = format!("light.{name}");
            match name.as_str() {
                "direction" => {
                    if let Some(value) = self.glass_point(field, span, &label) {
                        light.direction = value;
                    }
                }
                "elevation" => {
                    if let Some(value) =
                        self.glass_number(field, span, &label, |value| (0.0..=1.0).contains(&value))
                    {
                        light.elevation = value;
                    }
                }
                "intensity" => {
                    if let Some(value) = self.glass_number(field, span, &label, non_negative) {
                        light.intensity = value;
                    }
                }
                "space" => self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "light.space was removed: the light stays fixed on the screen",
                ),
                other => self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("unknown Glass light property `{other}`"),
                ),
            }
        }
        light
    }

    /// The Glass or GlassField node. A Glass keeps an absent material and light absent: scope
    /// normalization decides, once the whole tree exists, whether it takes them from a field or
    /// gets the defaults.
    pub(super) fn finish_glass_kind(
        &mut self,
        kind_tag: &str,
        props: GlassProps,
        path: Option<PathValue>,
        fill_rule: FillRule,
    ) -> NodeKind {
        if kind_tag == "glass" {
            NodeKind::Glass(GlassNode {
                path,
                fill_rule,
                material: props.material,
                light: props.light,
                presence: props.presence.unwrap_or(NumberValue::Static {
                    value: DEFAULT_PRESENCE,
                }),
            })
        } else {
            NodeKind::GlassField(GlassFieldNode {
                material: props
                    .material
                    .unwrap_or_else(GlassMaterialBinding::defaults),
                light: props.light.unwrap_or_else(GlassLightBinding::defaults),
                merge: props
                    .merge_distance
                    .map_or_else(GlassMergeIntent::defaults, |distance| GlassMergeIntent {
                        distance,
                    }),
            })
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

    fn compile(source: &str) -> Result<SceneArtifact, Vec<CompilerDiagnostic>> {
        compile_motion(source).map(|compiled| compiled.artifact)
    }

    fn errors(source: &str) -> String {
        compile(source)
            .unwrap_err()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn scene(element: &str) -> String {
        format!("export default function Scene() {{\n  return {element};\n}}\n")
    }

    fn glasses(artifact: &SceneArtifact) -> Vec<&GlassNode> {
        artifact
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Glass(glass) => Some(glass),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_glass_on_its_own_gets_the_default_material_and_light() {
        let artifact = compile(&scene(
            r#"<Glass style={{ width: 420, height: 180, borderRadius: 90 }} />"#,
        ))
        .unwrap();
        let glass = glasses(&artifact)[0];
        assert_eq!(glass.path, None);
        assert_eq!(glass.material, Some(GlassMaterialBinding::defaults()));
        assert_eq!(glass.light, Some(GlassLightBinding::defaults()));
        assert_eq!(glass.presence, NumberValue::Static { value: 1.0 });
        assert!(
            artifact
                .capability_set
                .names
                .contains(&MOTION_GLASS_CAPABILITY.to_string())
        );
        artifact.validate().unwrap();
    }

    #[test]
    fn every_glass_property_lowers() {
        let artifact = compile(&scene(
            r##"<Glass path={line([point(0, 0), point(160, 0), point(80, 140)])} fillRule="evenOdd"
      material={{ bevel: 30, refraction: 9, dispersion: 0.2, frost: 12, tint: "#B9D7FF18" }}
      light={{ direction: point(0.6, -0.8), elevation: 0.4, intensity: 0.9 }}
      presence={0.5}
      style={{ width: 160, height: 140 }} />"##,
        ))
        .unwrap();
        let glass = glasses(&artifact)[0];
        assert!(matches!(glass.path, Some(PathValue::Static { .. })));
        assert_eq!(glass.fill_rule, FillRule::EvenOdd);
        let material = glass.material.as_ref().unwrap();
        assert_eq!(material.bevel, Some(NumberValue::Static { value: 30.0 }));
        assert_eq!(
            material.refraction,
            Some(NumberValue::Static { value: 9.0 })
        );
        assert_eq!(material.dispersion, NumberValue::Static { value: 0.2 });
        assert_eq!(material.frost, NumberValue::Static { value: 12.0 });
        let light = glass.light.as_ref().unwrap();
        assert_eq!(light.elevation, NumberValue::Static { value: 0.4 });
        assert_eq!(light.intensity, NumberValue::Static { value: 0.9 });
        assert_eq!(glass.presence, NumberValue::Static { value: 0.5 });
        artifact.validate().unwrap();
    }

    #[test]
    fn material_and_light_may_be_static_objects() {
        let artifact = compile(
            r##"
// clarity 1 is clear glass, 0 fully frosted; depth 0 is a thin pane, 1 a thick lens.
function glassMaterial(clarity: number, depth: number, tint = "#ffffff14") {
  const bevel = 8 + 40 * depth;
  return { bevel, refraction: bevel * (0.25 + 0.5 * depth), frost: 20 * (1 - clarity), tint };
}
const light = { direction: point(1, 0), intensity: 0.5 };
export default function Scene() {
  const ctx = useTime();
  return <Glass material={{ ...glassMaterial(0.75, 0.5), dispersion: ctx.progress }}
    light={light} style={{ width: 200, height: 100 }} />;
}
"##,
        )
        .unwrap();
        let glass = glasses(&artifact)[0];
        let material = glass.material.as_ref().unwrap();
        assert_eq!(material.bevel, Some(NumberValue::Static { value: 28.0 }));
        assert_eq!(
            material.refraction,
            Some(NumberValue::Static { value: 14.0 })
        );
        assert_eq!(material.frost, NumberValue::Static { value: 5.0 });
        assert!(matches!(material.tint, ColorValue::Static { .. }));
        assert!(matches!(material.dispersion, NumberValue::Expr { .. }));
        let light = glass.light.as_ref().unwrap();
        assert_eq!(
            light.direction,
            PointValue::Static {
                value: Point::new(1.0, 0.0)
            }
        );
        assert_eq!(light.intensity, NumberValue::Static { value: 0.5 });
        assert_eq!(light.elevation, GlassLightBinding::defaults().elevation);
        artifact.validate().unwrap();
    }

    #[test]
    fn field_members_take_the_fields_material_and_light() {
        let artifact = compile(&scene(
            r#"<GlassField material={{ frost: 8 }} merge={{ distance: 36 }}
      light={{ direction: point(-0.35, -0.94), intensity: 0.7 }}>
      <View style={{ transform: "translateX(40px)" }}>
        <Glass style={{ width: 100, height: 100, borderRadius: "50%" }} />
      </View>
      <Glass style={{ width: 100, height: 100, borderRadius: "50%" }} />
    </GlassField>"#,
        ))
        .unwrap();
        let NodeKind::GlassField(field) = &artifact.nodes[1].kind else {
            panic!("expected GlassField node");
        };
        assert_eq!(field.merge.distance, 36.0);
        assert_eq!(field.material.frost, NumberValue::Static { value: 8.0 });
        assert_eq!(field.light.intensity, NumberValue::Static { value: 0.7 });
        let members = glasses(&artifact);
        assert_eq!(members.len(), 2);
        for member in members {
            assert_eq!(member.material, None);
            assert_eq!(member.light, None);
        }
        artifact.validate().unwrap();
    }

    #[test]
    fn a_member_may_not_set_material_or_light() {
        let message = errors(&scene(
            r#"<GlassField><Glass material={{ frost: 4 }} light={{ intensity: 0 }} /></GlassField>"#,
        ));
        assert!(
            message.contains("in a <GlassField> takes the field's material"),
            "{message}"
        );
        assert!(
            message.contains("in a <GlassField> takes the field's light"),
            "{message}"
        );
    }

    #[test]
    fn a_glass_in_a_members_children_draws_on_its_own() {
        let artifact = compile(&scene(
            r#"<GlassField>
      <Glass style={{ width: 200, height: 200 }}>
        <Glass material={{ frost: 2 }} style={{ width: 40, height: 40 }} />
      </Glass>
    </GlassField>"#,
        ))
        .unwrap();
        let glasses = glasses(&artifact);
        assert_eq!(glasses[0].material, None);
        assert!(glasses[1].material.is_some() && glasses[1].light.is_some());
        artifact.validate().unwrap();
    }

    #[test]
    fn a_field_directly_inside_a_field_is_refused() {
        let message = errors(&scene(
            r#"<GlassField><GlassField><Glass /></GlassField></GlassField>"#,
        ));
        assert!(
            message.contains("a <GlassField> inside another"),
            "{message}"
        );
    }

    #[test]
    fn removed_and_unknown_attributes_are_refused() {
        for (element, expected) in [
            (
                r#"<Glass shape={{ kind: "circle" }} />"#,
                "Glass `shape` was removed",
            ),
            (
                r#"<Glass surfaceId="lens" />"#,
                "Glass `surfaceId` was removed",
            ),
            (
                r#"<GlassField fieldId="orbit"><Glass /></GlassField>"#,
                "Glass `fieldId` was removed",
            ),
            (
                r#"<Glass environment={{ light: {} }} />"#,
                "Glass `environment` was removed",
            ),
            (
                r#"<Glass material={{ clarity: 0.8 }} />"#,
                "material.clarity was removed",
            ),
            (
                r#"<Glass light={{ space: "world" }} />"#,
                "light.space was removed",
            ),
            (
                r#"<Glass motion={{ character: "fluid" }} />"#,
                "unknown Glass attribute `motion`",
            ),
            (
                r#"<GlassField presence={0.5}><Glass /></GlassField>"#,
                "unknown GlassField attribute `presence`",
            ),
            (
                r#"<Glass merge={{ distance: 4 }} />"#,
                "unknown Glass attribute `merge`",
            ),
            (
                r#"<Glass material={{ frost: -1 }} />"#,
                "material.frost has an invalid static value",
            ),
        ] {
            let message = errors(&scene(element));
            assert!(message.contains(expected), "{element}: {message}");
        }
    }
}
