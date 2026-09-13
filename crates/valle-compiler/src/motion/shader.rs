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
        let Some(control) = source.strip_prefix("asset://") else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "ShaderLayer source must use asset://<shader-control>",
            );
            return None;
        };
        if !self
            .controls
            .assets
            .get(control)
            .is_some_and(|asset| asset.kind == AssetKind::Shader)
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("ShaderLayer source `{control}` must be a declared shader asset"),
            );
            return None;
        }
        let Some(package) = self
            .shader_registry
            .as_ref()
            .and_then(|registry| registry.asset(control))
        else {
            self.unsupported(
                span,
                format!(
                    "shader asset `{control}` is not bound to an admitted .shader.json descriptor"
                ),
            );
            return None;
        };
        if !self.resource_refs.iter().any(|resource| {
            resource.control == control && resource.content_hash == package.content_hash
        }) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("shader asset `{control}` does not match its frozen resource binding"),
            );
            return None;
        }
        let uri = package.uri();
        // Clone the manifest contract before lowering expressions, which mutates `self`.
        let manifest_inputs = package.manifest.inputs.clone();
        let manifest_uniforms = package.manifest.uniforms.clone();
        let program = ShaderProgramRef {
            work_per_pixel: package.work_per_pixel(),
            uri: uri.to_string(),
            content_hash: package.content_hash,
            abi_hash: package.abi_hash,
            padding: package.manifest.output.padding,
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
                inputs.push(ShaderTextureInput {
                    kind: input.kind,
                    name: input.name.clone(),
                    source: None,
                    sampling: input.sampling,
                    wrap: input.wrap,
                });
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
                kind: input.kind,
                sampling: input.sampling,
                wrap: input.wrap,
                name: input.name.clone(),
                source: Some(asset),
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
                range: uniform.min.zip(uniform.max).map(|(min, max)| [min, max]),
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
            valle_motion::shader::UniformType::Float3 => {
                let value = self.shader_components::<3>(uniform, expression, &label)?;
                Some(ShaderUniformValue::Float3 { value })
            }
            valle_motion::shader::UniformType::Float4 => {
                let value = self.shader_components::<4>(uniform, expression, &label)?;
                Some(ShaderUniformValue::Float4 { value })
            }
            valle_motion::shader::UniformType::Float2x2 => {
                let value = self.shader_components::<4>(uniform, expression, &label)?;
                Some(ShaderUniformValue::Float2x2 { value })
            }
            valle_motion::shader::UniformType::Float3x3 => {
                let value = self.shader_components::<9>(uniform, expression, &label)?;
                Some(ShaderUniformValue::Float3x3 { value })
            }
            valle_motion::shader::UniformType::Float4x4 => {
                let value = self.shader_components::<16>(uniform, expression, &label)?;
                Some(ShaderUniformValue::Float4x4 { value })
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

    fn shader_components<const N: usize>(
        &mut self,
        uniform: &valle_motion::shader::ShaderUniform,
        expression: &'s Expression<'s>,
        label: &str,
    ) -> Option<[NumberValue; N]> {
        if let Some(serde_json::Value::Array(items)) = self.eval_static(expression) {
            let values = items
                .iter()
                .map(|item| {
                    item.as_f64()
                        .filter(|v| {
                            v.is_finite()
                                && uniform.min.is_none_or(|min| *v >= f64::from(min))
                                && uniform.max.is_none_or(|max| *v <= f64::from(max))
                        })
                        .map(|value| NumberValue::Static { value })
                })
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values.filter(|values| values.len() == N) {
                return values.try_into().ok();
            }
            self.illegal(
                DiagCode::GrammarForbidden,
                expression.span(),
                format!("{label} requires {N} finite numeric components within its declared range"),
            );
            return None;
        }
        let items = array_items(expression);
        let Some(items) = items.filter(|items| items.len() == N) else {
            self.illegal(DiagCode::GrammarForbidden, expression.span(), format!("{label} requires an array of {N} numeric components (matrices are column-major)"));
            return None;
        };
        let values = items
            .into_iter()
            .map(|item| {
                self.number_binding(item, label, |value| {
                    uniform.min.is_none_or(|min| value >= f64::from(min))
                        && uniform.max.is_none_or(|max| value <= f64::from(max))
                })
            })
            .collect::<Option<Vec<_>>>()?;
        values.try_into().ok()
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
            valle_motion::shader::UniformValue::Float3(value) => ShaderUniformValue::Float3 {
                value: value.map(|v| NumberValue::Static {
                    value: f64::from(v),
                }),
            },
            valle_motion::shader::UniformValue::Float4(value) => ShaderUniformValue::Float4 {
                value: value.map(|v| NumberValue::Static {
                    value: f64::from(v),
                }),
            },
            valle_motion::shader::UniformValue::Float2x2(value) => ShaderUniformValue::Float2x2 {
                value: value.map(|v| NumberValue::Static {
                    value: f64::from(v),
                }),
            },
            valle_motion::shader::UniformValue::Float3x3(value) => ShaderUniformValue::Float3x3 {
                value: value.map(|v| NumberValue::Static {
                    value: f64::from(v),
                }),
            },
            valle_motion::shader::UniformValue::Float4x4(value) => ShaderUniformValue::Float4x4 {
                value: value.map(|v| NumberValue::Static {
                    value: f64::from(v),
                }),
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
