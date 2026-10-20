//! ShaderLayer package admission, texture inputs, and uniform lowering.

use super::*;
use valle_motion::shader::ShaderRole;

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
        // Copy the package contract before lowering expressions, which mutates `self`.
        let contract = ShaderContract::of(package);
        let authored_inputs = self.shader_object_values(inputs_object, "ShaderLayer inputs")?;
        let mut sources = BTreeMap::new();
        for (name, expression) in authored_inputs {
            match self.eval_static(expression) {
                Some(serde_json::Value::String(asset)) => {
                    sources.insert(name, (asset, expression.span()));
                }
                Some(serde_json::Value::Object(_)) => self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!(
                        "shader texture input `{name}` takes an asset:// reference; its descriptor \
                         sets how it is sampled"
                    ),
                ),
                _ => self.illegal(
                    DiagCode::GrammarForbidden,
                    expression.span(),
                    format!("shader texture input `{name}` must be a static asset:// reference"),
                ),
            }
        }
        if !self.diagnostics.is_empty() {
            return None;
        }
        let binding = self.lower_shader_contract(
            "ShaderLayer",
            &format!("shader package `{source}`"),
            contract,
            sources,
            uniforms_object,
            span,
        )?;
        Some(self.shader_layer_node(binding))
    }

    /// A ShaderLayer node drawing its children through a bound shader.
    pub(super) fn shader_layer_node(&mut self, binding: ShaderBinding) -> NodeKind {
        self.extra_capabilities
            .insert(SHADER_LAYER_CAPABILITY.to_owned());
        NodeKind::ShaderLayer {
            program: binding.program,
            uniforms: binding.uniforms,
            inputs: binding.inputs,
        }
    }

    /// A shader written with `code` on a ShaderLayer or a Transition, as `role` says: the code
    /// declares its uniforms and inputs, and `inputs` says how each input is sampled. `padding`
    /// is [left, top, right, bottom] local pixels.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_inline_shader(
        &mut self,
        role: ShaderRole,
        code: &str,
        code_span: Span,
        padding: [u32; 4],
        inputs_object: Option<&'s ObjectExpression<'s>>,
        uniforms_object: Option<&'s ObjectExpression<'s>>,
        span: Span,
    ) -> Option<ShaderBinding> {
        let owner = shader_owner(role);
        let inline = match valle_motion::shader::InlineCode::parse(code) {
            Ok(inline) => inline,
            Err(error) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    code_span,
                    shader_code_message(owner, &error),
                );
                return None;
            }
        };
        let authored_inputs =
            self.shader_object_values(inputs_object, &format!("{owner} inputs"))?;
        let mut sources = BTreeMap::new();
        let mut options = BTreeMap::new();
        for (name, expression) in authored_inputs {
            let (asset, input_options) = match self.eval_static(expression) {
                Some(serde_json::Value::String(source))
                    if source == valle_motion::shader::BACKDROP_INPUT =>
                {
                    (source, valle_motion::shader::InputOptions::backdrop())
                }
                Some(serde_json::Value::String(asset)) => {
                    (asset, valle_motion::shader::InputOptions::default())
                }
                Some(serde_json::Value::Object(fields)) => {
                    match self.inline_input_options(&name, &fields, expression.span()) {
                        Some(input) => input,
                        None => continue,
                    }
                }
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        expression.span(),
                        format!(
                            "shader texture input `{name}` must be a static asset:// reference, or \
                             {{ src, kind, sampling, wrap }} with one"
                        ),
                    );
                    continue;
                }
            };
            options.insert(name.clone(), input_options);
            sources.insert(name, (asset, expression.span()));
        }
        if !self.diagnostics.is_empty() {
            return None;
        }
        let package = match inline.package(
            role,
            |name| options.get(name).copied().unwrap_or_default(),
            padding,
        ) {
            Ok(package) => package,
            Err(error) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    code_span,
                    shader_code_message(owner, &error),
                );
                return None;
            }
        };
        self.inline_shaders
            .entry(package.uri().to_string())
            .or_insert_with(|| package.frozen());
        self.lower_shader_contract(
            owner,
            &format!("{owner} code"),
            ShaderContract::of(&package),
            sources,
            uniforms_object,
            span,
        )
    }

    /// `{ src, kind, sampling, wrap }`, how inline code samples one input.
    fn inline_input_options(
        &mut self,
        name: &str,
        fields: &serde_json::Map<String, serde_json::Value>,
        span: Span,
    ) -> Option<(String, valle_motion::shader::InputOptions)> {
        use valle_motion::shader::{InputKind, InputSampling, InputWrap};

        let mut src = None;
        let mut kind = InputKind::Color;
        let mut sampling = None;
        let mut wrap = None;
        for (key, value) in fields {
            let text = value.as_str();
            match (key.as_str(), text) {
                ("src", Some(text)) => src = Some(text.to_owned()),
                ("kind", Some("color")) => kind = InputKind::Color,
                ("kind", Some("data")) => kind = InputKind::Data,
                ("sampling", Some("nearest")) => sampling = Some(InputSampling::Nearest),
                ("sampling", Some("linear")) => sampling = Some(InputSampling::Linear),
                ("wrap", Some("clamp")) => wrap = Some(InputWrap::Clamp),
                ("wrap", Some("repeat")) => wrap = Some(InputWrap::Repeat),
                ("wrap", Some("mirror")) => wrap = Some(InputWrap::Mirror),
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!(
                            "shader texture input `{name}` takes src, an asset:// reference; kind, \
                             \"color\" or \"data\"; sampling, \"nearest\" or \"linear\"; and wrap, \
                             \"clamp\", \"repeat\" or \"mirror\" (not `{key}`: {value})"
                        ),
                    );
                    return None;
                }
            }
        }
        let Some(src) = src else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("shader texture input `{name}` needs `src`, an asset:// reference"),
            );
            return None;
        };
        if src == valle_motion::shader::BACKDROP_INPUT {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "shader texture input `{name}` reads the backdrop, which takes no options: \
                     write `{name}: \"{}\"`",
                    valle_motion::shader::BACKDROP_INPUT
                ),
            );
            return None;
        }
        Some((
            src,
            valle_motion::shader::InputOptions::new(kind, sampling, wrap),
        ))
    }

    /// Bind a package's texture inputs and uniforms. `sources` holds the authored asset of each
    /// input; `owner` names the element in diagnostics and `label` the package.
    fn lower_shader_contract(
        &mut self,
        owner: &str,
        label: &str,
        contract: ShaderContract,
        sources: BTreeMap<String, (String, Span)>,
        uniforms_object: Option<&'s ObjectExpression<'s>>,
        span: Span,
    ) -> Option<ShaderBinding> {
        let ShaderContract {
            program,
            inputs: declared_inputs,
            uniforms: declared_uniforms,
        } = contract;
        if let Some(unknown) = sources
            .keys()
            .find(|name| !declared_inputs.iter().any(|input| input.name == **name))
        {
            self.illegal(
                DiagCode::UnknownProp,
                span,
                format!("{label} has no texture input `{unknown}`"),
            );
        }
        let mut inputs = Vec::new();
        for input in &declared_inputs {
            let Some((asset, asset_span)) = sources.get(&input.name) else {
                if input.required {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("{label} requires texture input `{}`", input.name),
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
            let asset_span = *asset_span;
            let backdrop = input.kind == valle_motion::shader::InputKind::Backdrop;
            if backdrop != (asset == valle_motion::shader::BACKDROP_INPUT) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    asset_span,
                    if backdrop {
                        format!(
                            "shader texture input `{}` is the backdrop: bind it as \"{}\"",
                            input.name,
                            valle_motion::shader::BACKDROP_INPUT
                        )
                    } else {
                        format!(
                            "{label} declares `{}` as a texture, not the backdrop",
                            input.name
                        )
                    },
                );
                continue;
            }
            if backdrop {
                inputs.push(ShaderTextureInput {
                    kind: input.kind,
                    sampling: input.sampling,
                    wrap: input.wrap,
                    name: input.name.clone(),
                    source: Some(asset.clone()),
                });
                continue;
            }
            let Some(control_name) = asset.strip_prefix("asset://") else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    asset_span,
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
                        asset_span,
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
                        asset_span,
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
                    asset_span,
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
                source: Some(asset.clone()),
            });
        }

        let authored_uniforms =
            self.shader_object_values(uniforms_object, &format!("{owner} uniforms"))?;
        if let Some(unknown) = authored_uniforms.keys().find(|name| {
            !declared_uniforms
                .iter()
                .any(|uniform| uniform.name == **name)
        }) {
            self.illegal(
                DiagCode::UnknownProp,
                span,
                format!("{label} has no uniform `{unknown}`"),
            );
        }
        let mut uniforms = Vec::with_capacity(declared_uniforms.len());
        for uniform in &declared_uniforms {
            let value = if let Some(expression) = authored_uniforms.get(&uniform.name).copied() {
                self.shader_uniform_binding(uniform, expression)?
            } else if let Some(default) = &uniform.default {
                Self::shader_default_binding(default)
            } else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "{label} requires uniform `{}` of type {:?}",
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
        Some(ShaderBinding {
            program,
            uniforms,
            inputs,
        })
    }

    /// `outset`: a number of pixels, or one to four in CSS order (top, right, bottom, left), as
    /// the [left, top, right, bottom] padding of a package's output.
    pub(super) fn shader_outset(
        &mut self,
        value: &Option<JSXAttributeValue<'_>>,
        span: Span,
    ) -> Option<[u32; 4]> {
        let json = match value {
            Some(JSXAttributeValue::ExpressionContainer(container)) => container
                .expression
                .as_expression()
                .and_then(|expression| self.eval_static(expression)),
            _ => None,
        };
        let values = match json {
            Some(serde_json::Value::Array(items)) => items,
            Some(number @ serde_json::Value::Number(_)) => vec![number],
            _ => Vec::new(),
        };
        let pixels = values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .filter(|pixels| {
                        *pixels >= 0.0 && pixels.fract() == 0.0 && *pixels <= f64::from(u32::MAX)
                    })
                    .map(|pixels| pixels as u32)
            })
            .collect::<Option<Vec<_>>>();
        let [top, right, bottom, left] = match pixels.as_deref() {
            Some(&[all]) => [all; 4],
            Some(&[vertical, horizontal]) => [vertical, horizontal, vertical, horizontal],
            Some(&[top, horizontal, bottom]) => [top, horizontal, bottom, horizontal],
            Some(&[top, right, bottom, left]) => [top, right, bottom, left],
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "ShaderLayer outset must be a whole number of pixels, or one to four of them \
                     in CSS order (top, right, bottom, left), none negative",
                );
                return None;
            }
        };
        Some([left, top, right, bottom])
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
                        value: valle_draw::program::AuthorColor::from_srgb8(Rgba::new(
                            channel(*r),
                            channel(*g),
                            channel(*b),
                            channel(*a),
                        )),
                    },
                }
            }
            valle_motion::shader::UniformValue::Bool(value) => ShaderUniformValue::Bool {
                value: BoolValue::Static { value: *value },
            },
        }
    }
}

/// A shader's program with its uniforms and texture inputs bound.
pub(super) struct ShaderBinding {
    pub(super) program: ShaderProgramRef,
    pub(super) uniforms: Vec<ShaderUniformBinding>,
    pub(super) inputs: Vec<ShaderTextureInput>,
}

/// What lowering a shader needs from its package, copied out so lowering can mutate the
/// compiler.
struct ShaderContract {
    program: ShaderProgramRef,
    inputs: Vec<valle_motion::shader::ShaderInput>,
    uniforms: Vec<valle_motion::shader::ShaderUniform>,
}

impl ShaderContract {
    fn of(package: &valle_motion::shader::ShaderPackage) -> Self {
        Self {
            program: ShaderProgramRef {
                work_per_pixel: package.work_per_pixel(),
                uri: package.uri().to_string(),
                content_hash: package.content_hash,
                abi_hash: package.abi_hash,
                padding: package.manifest.output.padding,
            },
            inputs: package.manifest.inputs.clone(),
            uniforms: package.manifest.uniforms.clone(),
        }
    }
}

/// The element that writes a shader of this role.
fn shader_owner(role: ShaderRole) -> &'static str {
    match role {
        ShaderRole::Layer => "ShaderLayer",
        ShaderRole::Transition => "Transition",
    }
}

/// A shader diagnostic about inline code, placed by line and column in the code when the shader
/// language reports where it stopped, as `source:line:column`.
fn shader_code_message(owner: &str, error: &valle_motion::shader::ShaderDiagnostic) -> String {
    match error
        .path
        .strip_prefix("source:")
        .and_then(|place| place.split_once(':'))
    {
        Some((line, column)) => format!(
            "{owner} code, line {line}, column {column}: {}",
            error.message
        ),
        None => format!("{owner} code: {}", error.message),
    }
}
