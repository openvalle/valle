//! ShaderLayer package admission, texture inputs, and uniform lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_shader_layer(
        &mut self,
        source: &str,
        inputs_object: Option<&'s ObjectExpression<'s>>,
        uniforms_object: Option<&'s ObjectExpression<'s>>,
        span: Span,
    ) -> Option<NodeKind> {
        let uri = source
            .parse::<valle_motion::shader::ShaderUri>()
            .unwrap_or_else(|error| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("invalid ShaderLayer source: {error}"),
                );
                // A sentinel keeps this branch expression-shaped; diagnostics make the result fail.
                "shader://invalid@1".parse().expect("sentinel URI")
            });
        if source != uri.to_string() {
            return None;
        }
        let Some(registry) = self.shader_registry.as_ref() else {
            self.unsupported(
                span,
                format!(
                    "ShaderLayer package `{source}` is not available: this compile entrypoint has \
                     no ShaderRegistryEnv"
                ),
            );
            return None;
        };
        let package = match registry.resolve(&uri) {
            Ok(package) => package,
            Err(error) => {
                self.unsupported(span, error.to_string());
                return None;
            }
        };
        // Clone the manifest contract before lowering expressions, which mutates `self`.
        let manifest_inputs = package.manifest.inputs.clone();
        let manifest_uniforms = package.manifest.uniforms.clone();
        let program = ShaderProgramRef {
            uri: uri.to_string(),
            content_hash: package.content_hash,
            abi_hash: package.manifest.abi_digest,
        };

        let authored_inputs = self.shader_object_values(inputs_object, "ShaderLayer inputs")?;
        let known_inputs = manifest_inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = authored_inputs
            .keys()
            .find(|name| !known_inputs.contains(name.as_str()))
        {
            self.illegal(
                DiagCode::UnknownProp,
                span,
                format!("shader package `{source}` has no texture input `{unknown}`"),
            );
        }
        let mut inputs = Vec::new();
        for input in &manifest_inputs {
            let Some(expression) = authored_inputs.get(&input.name).copied() else {
                if input.required {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!(
                            "shader package `{source}` requires texture input `{}`",
                            input.name
                        ),
                    );
                }
                continue;
            };
            let Some(serde_json::Value::String(asset)) = self.eval_static(expression) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!(
                        "shader texture input `{}` must be a static asset:// reference",
                        input.name
                    ),
                );
                continue;
            };
            let Some(control_name) = asset.strip_prefix("asset://") else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!(
                        "shader texture input `{}` must use asset://<image-control>",
                        input.name
                    ),
                );
                continue;
            };
            match self.controls.assets.get(control_name) {
                Some(control) if control.kind == AssetKind::Image => {}
                Some(_) => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        format!(
                            "shader texture input `{}` control `{control_name}` is not an image",
                            input.name
                        ),
                    );
                    continue;
                }
                None => {
                    self.illegal(
                        DiagCode::UnknownProp,
                        expression.span(),
                        format!(
                            "shader texture input `{}` references unknown control `{control_name}`",
                            input.name
                        ),
                    );
                    continue;
                }
            }
            if !self
                .resource_refs
                .iter()
                .any(|resource| resource.control == control_name)
            {
                self.unsupported(
                    expression.span(),
                    format!(
                        "shader texture input `{}` has no content-addressed resource binding for \
                         image control `{control_name}`",
                        input.name
                    ),
                );
                continue;
            }
            inputs.push(ShaderTextureInput {
                name: input.name.clone(),
                source: asset,
            });
        }

        let authored_uniforms =
            self.shader_object_values(uniforms_object, "ShaderLayer uniforms")?;
        let known_uniforms = manifest_uniforms
            .iter()
            .map(|uniform| uniform.name.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = authored_uniforms
            .keys()
            .find(|name| !known_uniforms.contains(name.as_str()))
        {
            self.illegal(
                DiagCode::UnknownProp,
                span,
                format!("shader package `{source}` has no uniform `{unknown}`"),
            );
        }
        let mut uniforms = Vec::with_capacity(manifest_uniforms.len());
        for uniform in &manifest_uniforms {
            let value = if let Some(expression) = authored_uniforms.get(&uniform.name).copied() {
                self.shader_uniform_binding(uniform, expression)?
            } else if let Some(default) = &uniform.default {
                Self::shader_default_binding(default)
            } else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "shader package `{source}` requires uniform `{}` of type {:?}",
                        uniform.name, uniform.uniform_type
                    ),
                );
                continue;
            };
            uniforms.push(ShaderUniformBinding {
                name: uniform.name.clone(),
                value,
            });
        }

        if !self.diagnostics.is_empty() {
            return None;
        }
        self.extra_capabilities
            .insert(SHADER_LAYER_CAPABILITY.to_owned());
        Some(NodeKind::ShaderLayer {
            program,
            uniforms,
            inputs,
        })
    }

    pub(super) fn shader_object_values(
        &mut self,
        object: Option<&'s ObjectExpression<'s>>,
        label: &str,
    ) -> Option<BTreeMap<String, &'s Expression<'s>>> {
        let mut values = BTreeMap::new();
        let Some(object) = object else {
            return Some(values);
        };
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    format!("{label} cannot spread; ABI names must stay explicit"),
                );
                continue;
            };
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("{label} keys must be static ABI names"),
                );
                continue;
            };
            if values.insert(name.clone(), &property.value).is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.key.span(),
                    format!("{label} declares `{name}` more than once"),
                );
            }
        }
        self.diagnostics.is_empty().then_some(values)
    }

    pub(super) fn shader_uniform_binding(
        &mut self,
        uniform: &valle_motion::shader::ShaderUniform,
        expression: &'s Expression<'s>,
    ) -> Option<ShaderUniformValue> {
        let label = format!("shader uniform `{}`", uniform.name);
        match uniform.uniform_type {
            valle_motion::shader::UniformType::Float => self
                .number_binding(expression, &label, |value| {
                    uniform.min.is_none_or(|min| value >= f64::from(min))
                        && uniform.max.is_none_or(|max| value <= f64::from(max))
                })
                .map(|value| ShaderUniformValue::Float { value }),
            valle_motion::shader::UniformType::Float2 => {
                let value = self.point_binding(expression, &label)?;
                if let PointValue::Static { value } = value
                    && !(uniform
                        .min
                        .is_none_or(|min| value.x >= f64::from(min) && value.y >= f64::from(min))
                        && uniform.max.is_none_or(|max| {
                            value.x <= f64::from(max) && value.y <= f64::from(max)
                        }))
                {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        format!("{label} is outside its declared range"),
                    );
                    return None;
                }
                Some(ShaderUniformValue::Float2 { value })
            }
            valle_motion::shader::UniformType::Color => self
                .color_binding(expression, &label)
                .map(|value| ShaderUniformValue::Color { value }),
            valle_motion::shader::UniformType::Bool => {
                let value = match self.eval_static(expression) {
                    Some(serde_json::Value::Bool(value)) => BoolValue::Static { value },
                    Some(_) => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            expression.span(),
                            format!("{label} must be bool"),
                        );
                        return None;
                    }
                    None => BoolValue::Expr {
                        expr: self.lower_expr(expression)?,
                    },
                };
                Some(ShaderUniformValue::Bool { value })
            }
        }
    }

    pub(super) fn shader_default_binding(
        value: &valle_motion::shader::UniformValue,
    ) -> ShaderUniformValue {
        match value {
            valle_motion::shader::UniformValue::Float(value) => ShaderUniformValue::Float {
                value: NumberValue::Static {
                    value: f64::from(*value),
                },
            },
            valle_motion::shader::UniformValue::Float2([x, y]) => ShaderUniformValue::Float2 {
                value: PointValue::Static {
                    value: Point::new(f64::from(*x), f64::from(*y)),
                },
            },
            valle_motion::shader::UniformValue::Color([r, g, b, a]) => {
                let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                ShaderUniformValue::Color {
                    value: ColorValue::Static {
                        value: Rgba::new(channel(*r), channel(*g), channel(*b), channel(*a)),
                    },
                }
            }
            valle_motion::shader::UniformValue::Bool(value) => ShaderUniformValue::Bool {
                value: BoolValue::Static { value: *value },
            },
        }
    }
}
