//! Scene3D authoring admission and typed scene lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_scene3d_node(
        &mut self,
        element: &'s JSXElement<'s>,
        key: String,
        class_names: Vec<String>,
        class_conditions: BTreeMap<String, ExprId>,
        styles: Vec<StyleBinding>,
        visibility: Option<ExprId>,
        camera_object: Option<&'s ObjectExpression<'s>>,
        pbr_object: Option<&'s ObjectExpression<'s>>,
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
        let position = self.scene3d_frame_vec3(
            camera_values.get("position").copied(),
            camera_object.span(),
            "Scene3D camera.position",
        )?;
        let target = self.scene3d_frame_vec3(
            camera_values.get("target").copied(),
            camera_object.span(),
            "Scene3D camera.target",
        )?;
        let number = |value: f64| NumberValue::Static { value };
        let orbit_yaw_degrees = self.scene3d_number(
            camera_values.get("orbitYaw").copied(),
            0.0,
            "camera orbitYaw",
        )?;
        let orbit_pitch_degrees = self.scene3d_number(
            camera_values.get("orbitPitch").copied(),
            0.0,
            "camera orbitPitch",
        )?;
        let distance = match camera_values.get("distance") {
            Some(value) => Some(self.number_binding(value, "camera distance", |v| v > 0.0)?),
            None => None,
        };
        let camera_binding = Scene3DCameraBinding {
            position,
            target,
            orbit_yaw_degrees,
            orbit_pitch_degrees,
            distance,
            fov_y_degrees: self.scene3d_number(
                camera_values.get("fov").copied(),
                38.0,
                "camera fov",
            )?,
            near: self.scene3d_number(camera_values.get("near").copied(), 0.1, "camera near")?,
            far: self.scene3d_number(camera_values.get("far").copied(), 100.0, "camera far")?,
        };
        if let Some(Err(error)) = camera_binding.constant() {
            self.illegal(
                DiagCode::GrammarForbidden,
                camera_object.span(),
                error.to_string(),
            );
            return None;
        }

        let mut meshes = Vec::new();
        let mut mesh_frames = Vec::new();
        let mut lights = Vec::new();
        let mut light_frames = Vec::new();
        let mut anchors = Vec::new();
        let mut pbr = valle_motion::scene3d::PbrOptions::default();
        let mut exposure = number(1.0);
        let mut environment_intensity = number(1.0);
        let mut environment_rotation_degrees = number(0.0);
        if let Some(object) = pbr_object {
            let values = self.scene3d_object_values(
                object,
                "Scene3D pbr",
                &["environment", "toneMapping", "exposure"],
            )?;

            if let Some(expression) = values.get("environment") {
                let Expression::ObjectExpression(object) = expression.without_parentheses() else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        "Scene3D environment must be { src, intensity?, rotation?, background? }",
                    );
                    return None;
                };
                let values = self.scene3d_object_values(
                    object,
                    "Scene3D environment",
                    &["src", "intensity", "rotation", "background"],
                )?;
                let source = values
                    .get("src")
                    .and_then(|v| self.eval_static(v))
                    .and_then(|v| v.as_str().map(str::to_owned));
                let control = source.as_deref().and_then(|v| v.strip_prefix("asset://"));
                let Some(control) = control.filter(|control| {
                    self.controls
                        .assets
                        .get(*control)
                        .is_some_and(|a| a.kind == AssetKind::Environment)
                }) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        "Scene3D environment src must be asset://<environment-control>",
                    );
                    return None;
                };
                let mut background = false;
                if let Some(value) = values.get("background") {
                    let Some(value) = self.eval_static(value).and_then(|v| v.as_bool()) else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            value.span(),
                            "environment background must be a static boolean",
                        );
                        return None;
                    };
                    background = value;
                }
                pbr.environment = Some(valle_motion::scene3d::EnvironmentSpec {
                    control: control.to_owned(),
                    background,
                });
                if let Some(value) = values.get("intensity") {
                    environment_intensity =
                        self.number_binding(value, "environment intensity", |v| {
                            (0.0..=16.0).contains(&v)
                        })?;
                }
                if let Some(value) = values.get("rotation") {
                    environment_rotation_degrees =
                        self.number_binding(value, "environment rotation", |v| {
                            v.abs() <= f64::from(valle_motion::scene3d::MAX_ABS_ROTATION_DEGREES)
                        })?;
                }
            }
            if let Some(expression) = values.get("toneMapping") {
                match self
                    .eval_static(expression)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .as_deref()
                {
                    Some("aces") => pbr.tone_mapping = valle_motion::scene3d::ToneMapping::Aces,
                    Some("none") => pbr.tone_mapping = valle_motion::scene3d::ToneMapping::None,
                    _ => self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        "Scene3D pbr.toneMapping must be `aces` or `none`",
                    ),
                }
            }
            exposure =
                self.scene3d_number(values.get("exposure").copied(), 1.0, "Scene3D exposure")?;
        }
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
                            "materials",
                            "nodes",
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
                    let (material, material_frame) = self.scene3d_material(&attrs, child.span())?;
                    let (material_overrides, material_frames) = self.scene3d_material_overrides(&attrs)?;
                    let transform = self.scene3d_mesh_transform(&attrs, child.span())?;
                    let node_frames = self.scene3d_nodes(&attrs)?;
                    let node_ids = node_frames.iter().map(|n| n.id).collect();
                    mesh_frames.push(Scene3DMeshBinding {
                        key: mesh_key.clone(),
                        material: material_frame,
                        material_overrides: material_frames,
                        transform,
                        nodes: node_frames,
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
                        material_overrides,
                        node_ids,
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
                "AmbientLight" | "DirectionalLight" | "HemisphereLight" => {
                    let binding = self.scene3d_light_binding(child, child_tag)?;
                    lights.push(binding.kind());
                    light_frames.push(binding);
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
            meshes,
            lights,
            anchors,
            pbr,
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
                    environment_intensity,
                    environment_rotation_degrees,
                    exposure,
                    camera: camera_binding,
                    meshes: mesh_frames,
                    lights: light_frames,
                },
            },
            class_names,
            class_conditions,
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

    fn scene3d_number(
        &mut self,
        value: Option<&'s Expression<'s>>,
        default: f64,
        label: &str,
    ) -> Option<NumberValue> {
        match value {
            Some(value) => self.number_binding(value, label, |_| true),
            None => Some(NumberValue::Static { value: default }),
        }
    }

    fn scene3d_frame_vec3(
        &mut self,
        value: Option<&'s Expression<'s>>,
        span: Span,
        label: &str,
    ) -> Option<[NumberValue; 3]> {
        let Some(value) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} is required"),
            );
            return None;
        };
        if let Some(value) = self.eval_static(value).and_then(|v| v.as_array().cloned()) {
            if value.len() == 3 && value.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) {
                return Some(std::array::from_fn(|i| NumberValue::Static {
                    value: value[i].as_f64().unwrap(),
                }));
            }
        }
        let Some(items) = array_items(value).filter(|a| a.len() == 3) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{label} requires a numeric [x,y,z] array"),
            );
            return None;
        };
        items
            .into_iter()
            .map(|v| self.number_binding(v, label, |_| true))
            .collect::<Option<Vec<_>>>()?
            .try_into()
            .ok()
    }

    fn scene3d_mesh_transform(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        span: Span,
    ) -> Option<Scene3DTransformBinding> {
        let mut vectors = Vec::new();
        for (name, axes, default) in [
            ("position", ["translateX", "translateY", "translateZ"], 0.0),
            ("rotation", ["rotateX", "rotateY", "rotateZ"], 0.0),
            ("scale", ["scaleX", "scaleY", "scaleZ"], 1.0),
        ] {
            let mut vector = self.scene3d_frame_attr_vec3(attrs, name, Some([default; 3]))?;
            for (i, axis) in axes.into_iter().enumerate() {
                let Some((value, at)) = attrs.get(axis) else {
                    continue;
                };
                let offset = self.scene3d_attr_number(value, *at, axis)?;
                vector[i] = match (&vector[i], &offset) {
                    (NumberValue::Static { value: a }, NumberValue::Static { value: b }) => {
                        NumberValue::Static {
                            value: if name == "scale" { a * b } else { a + b },
                        }
                    }
                    _ => {
                        let mut expr = |n: &NumberValue| match n {
                            NumberValue::Expr { expr } => *expr,
                            NumberValue::Static { value } => self.push(
                                Expr::Const {
                                    value: MotionValue::Number(*value),
                                },
                                span,
                            ),
                        };
                        let lhs = expr(&vector[i]);
                        let rhs = expr(&offset);
                        NumberValue::Expr {
                            expr: self.push(
                                if name == "scale" {
                                    Expr::Mul { lhs, rhs }
                                } else {
                                    Expr::Add { lhs, rhs }
                                },
                                span,
                            ),
                        }
                    }
                };
            }
            vectors.push(vector);
        }
        let [translation, rotation_degrees, scale] = <[_; 3]>::try_from(vectors).ok()?;
        Some(Scene3DTransformBinding {
            translation,
            rotation_degrees,
            scale,
        })
    }

    fn scene3d_nodes(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
    ) -> Option<Vec<Scene3DNodeBinding>> {
        let Some((value, span)) = attrs.get("nodes") else {
            return Some(Vec::new());
        };
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Mesh nodes must be an explicit array of {id, position?, rotation?, scale?}",
            );
            return None;
        };
        let Some(items) = container.expression.as_expression().and_then(array_items) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Mesh nodes requires an explicit array",
            );
            return None;
        };
        if items.len() > valle_motion::scene3d::MAX_MODEL_NODES {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Mesh node bindings exceed the node budget",
            );
            return None;
        }
        let mut result = Vec::new();
        for item in items {
            let Expression::ObjectExpression(object) = item.without_parentheses() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    item.span(),
                    "each Mesh node binding must be an object",
                );
                return None;
            };
            let values = self.scene3d_object_values(
                object,
                "Mesh node",
                &["id", "position", "rotation", "scale"],
            )?;
            let Some(id) = values
                .get("id")
                .and_then(|v| self.eval_static(v))
                .and_then(|v| v.as_u64())
                .filter(|v| *v < valle_motion::scene3d::MAX_MODEL_NODES as u64)
            else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    item.span(),
                    "Mesh node id must be a static GLB node index within the node budget",
                );
                return None;
            };
            let mut vectors = Vec::new();
            for (name, default) in [("position", 0.0), ("rotation", 0.0), ("scale", 1.0)] {
                vectors.push(match values.get(name) {
                    Some(v) => self.scene3d_frame_vec3(Some(v), v.span(), name)?,
                    None => std::array::from_fn(|_| NumberValue::Static { value: default }),
                });
            }
            let [translation, rotation_degrees, scale] = <[_; 3]>::try_from(vectors).ok()?;
            result.push(Scene3DNodeBinding {
                id: id as u32,
                transform: Scene3DTransformBinding {
                    translation,
                    rotation_degrees,
                    scale,
                },
            });
        }
        Some(result)
    }

    fn scene3d_light_binding(
        &mut self,
        child: &'s JSXElement<'s>,
        tag: &str,
    ) -> Option<Scene3DLightBinding> {
        let allowed: &[&str] = match tag {
            "AmbientLight" => &["key", "color", "intensity"],
            "DirectionalLight" => &["key", "color", "direction", "intensity"],
            _ => &["key", "skyColor", "groundColor", "direction", "intensity"],
        };
        let attrs = self.scene3d_attributes(child, tag, allowed)?;
        let intensity = match attrs.get("intensity") {
            Some((value, span)) => self.scene3d_attr_number(value, *span, "light intensity")?,
            None => NumberValue::Static { value: 1.0 },
        };
        let binding = match tag {
            "AmbientLight" => Scene3DLightBinding::Ambient {
                intensity,
                color: self.scene3d_light_color(&attrs, "color", Some("#ffffff"))?,
            },
            "DirectionalLight" => Scene3DLightBinding::Directional {
                intensity,
                direction: self.scene3d_frame_attr_vec3(&attrs, "direction", None)?,
                color: self.scene3d_light_color(&attrs, "color", Some("#ffffff"))?,
            },
            _ => Scene3DLightBinding::Hemisphere {
                intensity,
                direction: self.scene3d_frame_attr_vec3(
                    &attrs,
                    "direction",
                    Some([0.0, 1.0, 0.0]),
                )?,
                sky_color: self.scene3d_light_color(&attrs, "skyColor", None)?,
                ground_color: self.scene3d_light_color(&attrs, "groundColor", None)?,
            },
        };
        if let Some(value) = binding.constant() {
            if let Err(error) = value.validate() {
                self.illegal(DiagCode::GrammarForbidden, child.span(), error.to_string());
                return None;
            }
        }
        Some(binding)
    }

    fn scene3d_frame_attr_vec3(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        name: &str,
        default: Option<[f64; 3]>,
    ) -> Option<[NumberValue; 3]> {
        match attrs.get(name) {
            Some((Some(JSXAttributeValue::ExpressionContainer(container)), span)) => {
                self.scene3d_frame_vec3(container.expression.as_expression(), *span, name)
            }
            None if default.is_some() => {
                Some(default.unwrap().map(|value| NumberValue::Static { value }))
            }
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attrs.get(name).map_or(Span::default(), |(_, s)| *s),
                    format!("{name} requires a numeric [x,y,z] array"),
                );
                None
            }
        }
    }

    fn scene3d_light_color(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        name: &str,
        default: Option<&str>,
    ) -> Option<ColorValue> {
        let color = match attrs.get(name) {
            Some((Some(JSXAttributeValue::StringLiteral(value)), _)) => {
                Rgba::parse(&value.value).map(|value| ColorValue::Static { value })
            }
            Some((Some(JSXAttributeValue::ExpressionContainer(container)), _)) => {
                self.color_binding(container.expression.as_expression()?, name)
            }
            None => default
                .and_then(Rgba::parse)
                .map(|value| ColorValue::Static { value }),
            _ => None,
        };
        if color.is_none() || matches!(color,Some(ColorValue::Static {value}) if value.a!=255) {
            self.illegal(
                DiagCode::GrammarForbidden,
                attrs.get(name).map_or(Span::default(), |(_, s)| *s),
                format!("light {name} must be an opaque color expression"),
            );
            return None;
        }
        color
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

    fn scene3d_material(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
        _span: Span,
    ) -> Option<(valle_motion::scene3d::MaterialSpec, Scene3DMaterialBinding)> {
        let Some((value, span)) = attrs.get("material") else {
            return Some((Default::default(), Default::default()));
        };
        let object = self.attr_object_literal(value, *span, "Mesh material")?;
        self.scene3d_material_object(object, false)
    }

    fn scene3d_material_overrides(
        &mut self,
        attrs: &BTreeMap<String, (&'s Option<JSXAttributeValue<'s>>, Span)>,
    ) -> Option<(
        Vec<valle_motion::scene3d::MaterialOverrideSpec>,
        Vec<Scene3DMaterialOverrideBinding>,
    )> {
        let Some((value, span)) = attrs.get("materials") else {
            return Some((Vec::new(), Vec::new()));
        };
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Mesh materials requires an array of material overrides with original GLB indices",
            );
            return None;
        };
        let Some(items) = container
            .expression
            .as_expression()
            .and_then(array_items)
            .filter(|v| v.len() <= valle_motion::scene3d::MAX_MATERIALS)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Mesh materials requires a bounded explicit array",
            );
            return None;
        };
        let mut specs = Vec::new();
        let mut frames = Vec::new();
        for item in items {
            let Expression::ObjectExpression(object) = item.without_parentheses() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    item.span(),
                    "each material override must be an object",
                );
                return None;
            };
            let id = object
                .properties
                .iter()
                .find_map(|p| match p {
                    ObjectPropertyKind::ObjectProperty(p)
                        if p.key.static_name().as_deref() == Some("id") =>
                    {
                        Some(&p.value)
                    }
                    _ => None,
                })
                .and_then(|v| self.eval_static(v))
                .and_then(|v| v.as_u64());
            let Some(id) = id.filter(|&id| id < valle_motion::scene3d::MAX_MATERIALS as u64) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    item.span(),
                    "material id must be a static GLB material index within the material budget",
                );
                return None;
            };
            let (material, frame) = self.scene3d_material_object(object, true)?;
            specs.push(valle_motion::scene3d::MaterialOverrideSpec {
                id: id as u32,
                material,
            });
            frames.push(Scene3DMaterialOverrideBinding {
                id: id as u32,
                material: frame,
            });
        }
        Some((specs, frames))
    }

    fn scene3d_material_object(
        &mut self,
        object: &'s ObjectExpression<'s>,
        indexed: bool,
    ) -> Option<(valle_motion::scene3d::MaterialSpec, Scene3DMaterialBinding)> {
        use valle_motion::scene3d::{
            AlphaMode, MaterialKind, MaterialSpec, MaterialTexture, MaterialTextureSlot,
        };
        let mut allowed = vec![
            "type",
            "color",
            "metallic",
            "roughness",
            "emissive",
            "emissiveIntensity",
            "normalScale",
            "occlusionStrength",
            "alphaCutoff",
            "alphaMode",
            "doubleSided",
            "textures",
        ];
        if indexed {
            allowed.push("id");
        }
        let values = self.scene3d_object_values(object, "Mesh material", &allowed)?;
        let mut spec = MaterialSpec::default();
        if let Some(value) = values.get("type") {
            spec.kind = Some(
                match self
                    .eval_static(value)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .as_deref()
                {
                    Some("unlit") => MaterialKind::Unlit,
                    Some("lambert") => MaterialKind::Lambert,
                    Some("pbr") => MaterialKind::Pbr,
                    _ => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            value.span(),
                            "material type must be unlit, lambert or pbr",
                        );
                        return None;
                    }
                },
            );
        }
        if let Some(value) = values.get("alphaMode") {
            spec.alpha_mode = Some(
                match self
                    .eval_static(value)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .as_deref()
                {
                    Some("opaque") => AlphaMode::Opaque,
                    Some("mask") => AlphaMode::Mask,
                    _ => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            value.span(),
                            "material alphaMode must be opaque or mask",
                        );
                        return None;
                    }
                },
            );
        }
        if let Some(value) = values.get("doubleSided") {
            let Some(value) = self.eval_static(value).and_then(|v| v.as_bool()) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    value.span(),
                    "material doubleSided must be a static boolean",
                );
                return None;
            };
            spec.double_sided = Some(value);
        }
        if let Some(value) = values.get("textures") {
            let Some(textures) = self.eval_static(value).and_then(|v| v.as_object().cloned())
            else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    value.span(),
                    "material textures must be a static slot map",
                );
                return None;
            };
            for (name, value_json) in textures {
                let Ok(slot) = serde_json::from_value::<MaterialTextureSlot>(
                    serde_json::Value::String(name.clone()),
                ) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        value.span(),
                        format!("unknown material texture slot {name}"),
                    );
                    return None;
                };
                if value_json.is_null() {
                    spec.textures.insert(slot, None);
                    continue;
                }
                let mut texture = match value_json {
                    serde_json::Value::String(src) => {
                        serde_json::Map::from_iter([("src".into(), serde_json::Value::String(src))])
                    }
                    serde_json::Value::Object(value) => value,
                    _ => {
                        self.illegal(DiagCode::GrammarForbidden,value.span(),"texture requires an asset URI, {src, wrapU?, wrapV?, minFilter?, magFilter?, mipmap?}, or null");
                        return None;
                    }
                };
                if texture.contains_key("control") {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        value.span(),
                        "texture uses src, not control",
                    );
                    return None;
                }
                let Some(control) = texture.remove("src").and_then(|v| {
                    v.as_str()
                        .and_then(|v| v.strip_prefix("asset://"))
                        .map(str::to_owned)
                }) else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        value.span(),
                        "material texture src must be asset://<image-control>",
                    );
                    return None;
                };
                if !self
                    .controls
                    .assets
                    .get(&control)
                    .is_some_and(|v| v.kind == AssetKind::Image)
                {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        value.span(),
                        format!("material texture {control} must name an image control"),
                    );
                    return None;
                }
                texture.insert("control".into(), serde_json::Value::String(control));
                let texture = match serde_json::from_value::<MaterialTexture>(
                    serde_json::Value::Object(texture),
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        self.illegal(DiagCode::GrammarForbidden, value.span(), e.to_string());
                        return None;
                    }
                };
                spec.textures.insert(slot, Some(texture));
            }
        }
        let mut frame = Scene3DMaterialBinding::default();
        for (name, binding) in [
            ("color", &mut frame.color),
            ("emissive", &mut frame.emissive),
        ] {
            if let Some(value) = values.get(name) {
                *binding = Some(self.color_binding(value, name)?);
            }
        }
        for (name, binding) in [
            ("metallic", &mut frame.metallic),
            ("roughness", &mut frame.roughness),
            ("emissiveIntensity", &mut frame.emissive_intensity),
            ("normalScale", &mut frame.normal_scale),
            ("occlusionStrength", &mut frame.occlusion_strength),
            ("alphaCutoff", &mut frame.alpha_cutoff),
        ] {
            if let Some(value) = values.get(name) {
                *binding = Some(self.number_binding(value, name, |_| true)?);
            }
        }
        Some((spec, frame))
    }
}
