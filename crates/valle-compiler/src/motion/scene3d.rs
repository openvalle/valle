//! Scene3D authoring admission and typed scene lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_scene3d_node(
        &mut self,
        element: &'s JSXElement<'s>,
        key: String,
        class_names: Vec<String>,
        styles: Vec<StyleBinding>,
        visibility: Option<ExprId>,
        camera_object: Option<&'s ObjectExpression<'s>>,
    ) -> Option<PendingNode> {
        let camera_object = camera_object.or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.opening_element.span(),
                "Scene3D requires camera={{ position, target, orbitYaw?, orbitPitch?, distance?, fov?, near?, far? }}",
            );
            None
        })?;
        let camera_values = self.scene3d_object_values(
            camera_object,
            "Scene3D camera",
            &[
                "position",
                "target",
                "orbitYaw",
                "orbitPitch",
                "distance",
                "fov",
                "near",
                "far",
            ],
        )?;
        let position = self.scene3d_required_vec3(
            camera_values.get("position").copied(),
            camera_object.span(),
            "Scene3D camera.position",
        )?;
        let target = self.scene3d_required_vec3(
            camera_values.get("target").copied(),
            camera_object.span(),
            "Scene3D camera.target",
        )?;
        let number = |value: f64| NumberValue::Static { value };
        let orbit_yaw_degrees = camera_values
            .get("orbitYaw")
            .and_then(|value| self.number_binding(value, "Scene3D camera.orbitYaw", |_| true))
            .unwrap_or_else(|| number(0.0));
        let orbit_pitch_degrees = camera_values
            .get("orbitPitch")
            .and_then(|value| self.number_binding(value, "Scene3D camera.orbitPitch", |_| true))
            .unwrap_or_else(|| number(0.0));
        let base_delta = [
            f64::from(position.0[0] - target.0[0]),
            f64::from(position.0[1] - target.0[1]),
            f64::from(position.0[2] - target.0[2]),
        ];
        let base_distance = valle_draw::math::sqrt(
            base_delta
                .into_iter()
                .map(|component| component * component)
                .sum(),
        );
        let distance = camera_values
            .get("distance")
            .and_then(|value| {
                self.number_binding(value, "Scene3D camera.distance", |value| value > 0.0)
            })
            .unwrap_or_else(|| number(base_distance));
        let fov_y_degrees = camera_values
            .get("fov")
            .and_then(|value| {
                self.number_binding(value, "Scene3D camera.fov", |value| {
                    (10.0..=120.0).contains(&value)
                })
            })
            .unwrap_or_else(|| number(38.0));
        let static_fov = match &fov_y_degrees {
            NumberValue::Static { value } => *value as f32,
            NumberValue::Expr { .. } => 38.0,
        };
        let near = self.scene3d_static_number(
            camera_values.get("near").copied(),
            0.1,
            "Scene3D camera.near",
        )?;
        let far = self.scene3d_static_number(
            camera_values.get("far").copied(),
            100.0,
            "Scene3D camera.far",
        )?;

        let mut meshes = Vec::new();
        let mut mesh_frames = Vec::new();
        let mut lights = Vec::new();
        let mut light_intensities = Vec::new();
        let mut anchors = Vec::new();
        for child in &element.children {
            let JSXChild::Element(child) = child else {
                if matches!(child, JSXChild::Text(text) if text.value.trim().is_empty())
                    || matches!(child, JSXChild::ExpressionContainer(container) if matches!(container.expression, JSXExpression::EmptyExpression(_)))
                {
                    continue;
                }
                self.illegal(
                    DiagCode::GrammarForbidden,
                    child.span(),
                    "Scene3D children must be explicit Mesh/Anchor3D/AmbientLight/DirectionalLight elements",
                );
                continue;
            };
            let child_tag = match &child.opening_element.name {
                JSXElementName::Identifier(id) => id.name.as_str(),
                JSXElementName::IdentifierReference(id) => id.name.as_str(),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        child.opening_element.name.span(),
                        "Scene3D child tag must be a static identifier",
                    );
                    continue;
                }
            };
            if child.children.iter().any(
                |nested| !matches!(nested, JSXChild::Text(text) if text.value.trim().is_empty()),
            ) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    child.span(),
                    format!("Scene3D child <{child_tag}> is a leaf"),
                );
                continue;
            }
            match child_tag {
                "Mesh" => {
                    let attrs = self.scene3d_attributes(
                        child,
                        "Mesh",
                        &[
                            "key",
                            "src",
                            "material",
                            "position",
                            "rotation",
                            "scale",
                            "translateX",
                            "translateY",
                            "translateZ",
                            "rotateX",
                            "rotateY",
                            "rotateZ",
                            "scaleX",
                            "scaleY",
                            "scaleZ",
                        ],
                    )?;
                    let mesh_key = self.scene3d_required_attr_string(&attrs, "key", "Mesh")?;
                    let source = self.scene3d_required_attr_string(&attrs, "src", "Mesh")?;
                    let Some(model_control) = source.strip_prefix("asset://").map(str::to_owned)
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            child.span(),
                            "Mesh src must be asset://<model3d-control>",
                        );
                        continue;
                    };
                    if !self
                        .controls
                        .assets
                        .get(&model_control)
                        .is_some_and(|asset| asset.kind == AssetKind::Model3d)
                    {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            child.span(),
                            format!("Mesh model control `{model_control}` must have kind model3d"),
                        );
                        continue;
                    }
                    let material = self.scene3d_material(&attrs, child.span())?;
                    let transform = valle_motion::scene3d::Transform3D {
                        translation: self.scene3d_optional_attr_vec3(
                            &attrs,
                            "position",
                            valle_motion::scene3d::Vec3::ZERO,
                            "Mesh position",
                        )?,
                        rotation_degrees: self.scene3d_optional_attr_vec3(
                            &attrs,
                            "rotation",
                            valle_motion::scene3d::Vec3::ZERO,
                            "Mesh rotation",
                        )?,
                        scale: self.scene3d_optional_attr_vec3(
                            &attrs,
                            "scale",
                            valle_motion::scene3d::Vec3::ONE,
                            "Mesh scale",
                        )?,
                    };
                    let frame_number = |this: &mut Self, name: &str, default: f64| {
                        attrs
                            .get(name)
                            .and_then(|(value, span)| {
                                this.scene3d_attr_number(value, *span, &format!("Mesh {name}"))
                            })
                            .unwrap_or(NumberValue::Static { value: default })
                    };
                    mesh_frames.push(Scene3DMeshBinding {
                        key: mesh_key.clone(),
                        translation_x: frame_number(self, "translateX", 0.0),
                        translation_y: frame_number(self, "translateY", 0.0),
                        translation_z: frame_number(self, "translateZ", 0.0),
                        rotation_x_degrees: frame_number(self, "rotateX", 0.0),
                        rotation_y_degrees: frame_number(self, "rotateY", 0.0),
                        rotation_z_degrees: frame_number(self, "rotateZ", 0.0),
                        scale_x: frame_number(self, "scaleX", 1.0),
                        scale_y: frame_number(self, "scaleY", 1.0),
                        scale_z: frame_number(self, "scaleZ", 1.0),
                    });
                    self.source_ledger.object_spans
                        .push((
                            key.clone(),
                            mesh_key.clone(),
                            child.span(),
                            self.component_stack.clone(),
                        ));
                    meshes.push(valle_motion::scene3d::MeshSpec {
                        key: mesh_key,
                        model_control,
                        material,
                        transform,
                    });
                }
                "Anchor3D" => {
                    let attrs = self.scene3d_attributes(
                        child,
                        "Anchor3D",
                        &["key", "parent", "position"],
                    )?;
                    anchors.push(valle_motion::scene3d::AnchorSpec {
                        key: self.scene3d_required_attr_string(&attrs, "key", "Anchor3D")?,
                        parent: self.scene3d_required_attr_string(&attrs, "parent", "Anchor3D")?,
                        position: self.scene3d_required_attr_vec3(
                            &attrs,
                            "position",
                            "Anchor3D position",
                        )?,
                    });
                }
                "AmbientLight" => {
                    let attrs =
                        self.scene3d_attributes(child, "AmbientLight", &["key", "intensity"])?;
                    let intensity = attrs
                        .get("intensity")
                        .and_then(|(value, span)| {
                            self.scene3d_attr_number(value, *span, "AmbientLight intensity")
                        })
                        .unwrap_or(NumberValue::Static { value: 1.0 });
                    let static_intensity = match &intensity {
                        NumberValue::Static { value } => *value as f32,
                        NumberValue::Expr { .. } => 1.0,
                    };
                    lights.push(valle_motion::scene3d::LightSpec::Ambient {
                        intensity: static_intensity,
                    });
                    light_intensities.push(intensity);
                }
                "DirectionalLight" => {
                    let attrs = self.scene3d_attributes(
                        child,
                        "DirectionalLight",
                        &["key", "direction", "intensity"],
                    )?;
                    let intensity = attrs
                        .get("intensity")
                        .and_then(|(value, span)| {
                            self.scene3d_attr_number(value, *span, "DirectionalLight intensity")
                        })
                        .unwrap_or(NumberValue::Static { value: 1.0 });
                    let static_intensity = match &intensity {
                        NumberValue::Static { value } => *value as f32,
                        NumberValue::Expr { .. } => 1.0,
                    };
                    lights.push(valle_motion::scene3d::LightSpec::Directional {
                        direction: self.scene3d_required_attr_vec3(
                            &attrs,
                            "direction",
                            "DirectionalLight direction",
                        )?,
                        intensity: static_intensity,
                    });
                    light_intensities.push(intensity);
                }
                _ => self.illegal(
                    DiagCode::GrammarForbidden,
                    child.span(),
                    format!(
                        "unknown Scene3D child <{child_tag}>; use Mesh, Anchor3D, AmbientLight, or DirectionalLight"
                    ),
                ),
            }
        }

        let scene = valle_motion::scene3d::Scene3DSpec {
            camera: valle_motion::scene3d::CameraSpec {
                position,
                target,
                fov_y_degrees: static_fov,
                near,
                far,
            },
            meshes,
            lights,
            anchors,
        };
        if let Err(errors) = scene.validate() {
            for error in errors.0 {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    format!("Scene3D{}: {}", error.path, error.message),
                );
            }
            return None;
        }
        self.extra_capabilities
            .insert(SCENE3D_LAYER_CAPABILITY.to_owned());
        Some(PendingNode {
            space: None,
            span: element.span(),
            expansion_stack: self.component_stack.clone(),
            key,
            kind: NodeKind::Scene3D {
                scene,
                frame: Scene3DFrameBinding {
                    camera: Scene3DCameraBinding {
                        orbit_yaw_degrees,
                        orbit_pitch_degrees,
                        distance,
                        fov_y_degrees,
                    },
                    meshes: mesh_frames,
                    light_intensities,
                },
            },
            class_names,
            styles,
            visibility,
            semantic: None,
            children: Vec::new(),
            is_mask_source: false,
        })
    }

    pub(super) fn attr_object_literal(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
        label: &str,
    ) -> Option<&'s ObjectExpression<'s>> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must be an object literal"),
            );
            return None;
        };
        let Some(expression) = container.expression.as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must not be empty"),
            );
            return None;
        };
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

    pub(super) fn scene3d_object_values(
        &mut self,
        object: &'s ObjectExpression<'s>,
        label: &str,
        allowed: &[&str],
    ) -> Option<BTreeMap<String, &'s Expression<'s>>> {
        let mut values = BTreeMap::new();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!("{label} cannot use spreads"),
                );
                continue;
            };
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("{label} keys must be static"),
                );
                continue;
            };
            if !allowed.contains(&name.as_str()) {
                self.illegal(
                    DiagCode::UnknownProp,
                    property.key.span(),
                    format!("unknown {label} field `{name}`"),
                );
                continue;
            }
            if values.insert(name.clone(), &property.value).is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!("{label} field `{name}` is duplicated"),
                );
            }
        }
        self.diagnostics.is_empty().then_some(values)
    }

    pub(super) fn scene3d_attributes(
        &mut self,
        element: &'s JSXElement<'s>,
        label: &str,
        allowed: &[&str],
    ) -> Option<BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>> {
        let mut values = BTreeMap::new();
        for item in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = item else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    item.span(),
                    format!("{label} cannot use spread attributes"),
                );
                continue;
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.name.span(),
                    format!("{label} attribute names must be static"),
                );
                continue;
            };
            let name = name.name.to_string();
            if !allowed.contains(&name.as_str()) {
                self.illegal(
                    DiagCode::UnknownProp,
                    attribute.span(),
                    format!("unknown {label} attribute `{name}`"),
                );
                continue;
            }
            if values
                .insert(name.clone(), (&attribute.value, attribute.span()))
                .is_some()
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.span(),
                    format!("{label} attribute `{name}` is duplicated"),
                );
            }
        }
        self.diagnostics.is_empty().then_some(values)
    }

    pub(super) fn scene3d_required_attr_string(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        name: &str,
        label: &str,
    ) -> Option<String> {
        let Some((value, span)) = attrs.get(name) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                Span::default(),
                format!("{label} requires `{name}`"),
            );
            return None;
        };
        self.attr_static_string(value, *span, &format!("{label} {name}"))
    }

    pub(super) fn scene3d_attr_number(
        &mut self,
        value: &'s Option<JSXAttributeValue<'s>>,
        span: Span,
        label: &str,
    ) -> Option<NumberValue> {
        match value {
            Some(JSXAttributeValue::StringLiteral(value)) => value
                .value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| NumberValue::Static { value }),
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.expr_number_value(expression)),
            _ => None,
        }
        .or_else(|| {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} must be a finite number expression"),
            );
            None
        })
    }

    pub(super) fn scene3d_required_vec3(
        &mut self,
        expression: Option<&'s Expression<'s>>,
        span: Span,
        label: &str,
    ) -> Option<valle_motion::scene3d::Vec3> {
        let Some(expression) = expression else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} is required"),
            );
            return None;
        };
        self.scene3d_vec3(expression, label)
    }

    pub(super) fn scene3d_vec3(
        &mut self,
        expression: &'s Expression<'s>,
        label: &str,
    ) -> Option<valle_motion::scene3d::Vec3> {
        let values = self
            .eval_static(expression)
            .and_then(|value| value.as_array().cloned())
            .filter(|values| values.len() == 3)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| value.as_f64().filter(|value| value.is_finite()))
                    .collect::<Option<Vec<_>>>()
            });
        let Some(values) = values else {
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!("{label} must be a prepare-time [x, y, z] finite number array"),
            );
            return None;
        };
        Some(valle_motion::scene3d::Vec3::new(
            values[0] as f32,
            values[1] as f32,
            values[2] as f32,
        ))
    }

    pub(super) fn scene3d_required_attr_vec3(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        name: &str,
        label: &str,
    ) -> Option<valle_motion::scene3d::Vec3> {
        let Some((value, span)) = attrs.get(name) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                Span::default(),
                format!("{label} is required"),
            );
            return None;
        };
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                format!("{label} must be a prepare-time [x, y, z] array"),
            );
            return None;
        };
        self.scene3d_vec3(container.expression.as_expression()?, label)
    }

    pub(super) fn scene3d_optional_attr_vec3(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        name: &str,
        default: valle_motion::scene3d::Vec3,
        label: &str,
    ) -> Option<valle_motion::scene3d::Vec3> {
        if attrs.contains_key(name) {
            self.scene3d_required_attr_vec3(attrs, name, label)
        } else {
            Some(default)
        }
    }

    pub(super) fn scene3d_static_number(
        &mut self,
        expression: Option<&'s Expression<'s>>,
        default: f32,
        label: &str,
    ) -> Option<f32> {
        let Some(expression) = expression else {
            return Some(default);
        };
        self.eval_static(expression)
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite())
            .map(|value| value as f32)
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("{label} must be a prepare-time finite number"),
                );
                None
            })
    }

    pub(super) fn scene3d_material(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        span: Span,
    ) -> Option<valle_motion::scene3d::MaterialSpec> {
        let Some((value, material_span)) = attrs.get("material") else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "Mesh requires material={{ type, color, texture? }}",
            );
            return None;
        };
        let object = self.attr_object_literal(value, *material_span, "Mesh material")?;
        let values =
            self.scene3d_object_values(object, "Mesh material", &["type", "color", "texture"])?;
        let kind = match values
            .get("type")
            .and_then(|value| self.eval_static(value))
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
        {
            Some("unlit") => valle_motion::scene3d::MaterialKind::Unlit,
            Some("lambert") => valle_motion::scene3d::MaterialKind::Lambert,
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    object.span(),
                    "Mesh material.type must be `unlit` or `lambert`",
                );
                return None;
            }
        };
        let color_text = values
            .get("color")
            .and_then(|value| self.eval_static(value))
            .and_then(|value| value.as_str().map(str::to_owned));
        let Some(color) = color_text.as_deref().and_then(Rgba::parse) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                object.span(),
                "Mesh material.color must be a static CSS color",
            );
            return None;
        };
        let texture_control = if let Some(expression) = values.get("texture") {
            let texture = self
                .eval_static(expression)
                .and_then(|value| value.as_str().map(str::to_owned));
            let Some(texture) = texture else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "Mesh material.texture must be a static asset://<image-control>",
                );
                return None;
            };
            let Some(control) = texture.strip_prefix("asset://").map(str::to_owned) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    "Mesh material.texture must be asset://<image-control>",
                );
                return None;
            };
            if !self
                .controls
                .assets
                .get(&control)
                .is_some_and(|asset| asset.kind == AssetKind::Image)
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("Mesh texture control `{control}` must have kind image"),
                );
                return None;
            }
            Some(control)
        } else {
            None
        };
        Some(valle_motion::scene3d::MaterialSpec {
            kind,
            color: valle_motion::scene3d::Color4([
                f32::from(color.r) / 255.0,
                f32::from(color.g) / 255.0,
                f32::from(color.b) / 255.0,
                f32::from(color.a) / 255.0,
            ]),
            texture_control,
        })
    }
}
