//! JSX intrinsic dispatch and ordinary Scene node lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_jsx(
        &mut self,
        element: &'s JSXElement<'s>,
        path: &str,
    ) -> Option<PendingNode> {
        let tag = match &element.opening_element.name {
            JSXElementName::Identifier(id) => id.name.to_string(),
            JSXElementName::IdentifierReference(id) => id.name.to_string(),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.opening_element.name.span(),
                    "member and namespaced JSX tags are illegal",
                );
                return None;
            }
        };
        if tag == "ThemeProvider" {
            return self.lower_theme_provider(element, path);
        }
        if let Some((function, _)) = self.authored_fn(&tag) {
            return self.lower_component(element, &tag, function, path);
        }
        if tag == "Span" {
            self.unsupported(
                element.span(),
                "<Span> is only valid as a direct child of <Text>",
            );
            return None;
        }
        let svg_shape = match tag.as_str() {
            "Circle" | "circle" => Some("circle"),
            "Ellipse" | "ellipse" => Some("ellipse"),
            "Rect" | "rect" => Some("rect"),
            "Line" | "line" => Some("line"),
            "Polyline" | "polyline" => Some("polyline"),
            "Polygon" | "polygon" => Some("polygon"),
            _ => None,
        };
        let (kind_tag, implied_class) = match tag.as_str() {
            "Scene" | "Group" => ("group", None),
            // Coordinate spaces are tagged groups sharing the same node and layout model.
            "World" => ("world", None),
            "Screen" => ("screen", None),
            "View" | "div" | "section" | "main" | "article" | "svg" => ("box", None),
            "Flex" => ("box", Some("flex")),
            "Absolute" => ("box", Some("absolute")),
            "Text" | "span" | "p" | "strong" | "h1" | "h2" | "h3" => ("text", None),
            "Path" | "path" => ("path", None),
            "Circle" | "circle" | "Ellipse" | "ellipse" | "Rect" | "rect" | "Line" | "line"
            | "Polyline" | "polyline" | "Polygon" | "polygon" => ("path", None),
            "GeometryBatch" => ("geometry-batch", None),
            "ShaderLayer" => ("shader-layer", None),
            "Scene3D" => ("scene3d", None),
            "Image" | "img" => ("image", None),
            "MathFormula" => ("math-formula", None),
            "Video" | "video" => ("video", None),
            "Clip" => ("clip", None),
            "Mask" => ("mask", None),
            "MaskSource" => ("mask-source", None),
            "Glass" => ("glass", None),
            "GlassField" => ("glass-field", None),
            _ => {
                self.unsupported(element.opening_element.name.span(), format!("`<{tag}>` is not a declared Motion component or admitted primitive; use View/Text/Flex/Absolute or an HTML alias"));
                return None;
            }
        };

        let mut key = None;
        let mut class_names = implied_class
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut styles = Vec::new();
        let mut style_object: Option<&'s ObjectExpression<'s>> = None;
        let mut layout_id = None;
        let mut visibility = None;
        let mut glass_props = super::glass::GlassProps::default();
        let mut path_d = None;
        let mut shape_numbers = BTreeMap::<String, ExprId>::new();
        let mut shape_points = None;
        let mut path_fill = Some(PaintValue::Solid {
            color: ColorValue::Static {
                value: Rgba::rgb(0, 0, 0),
            },
        });
        let mut saw_explicit_fill = false;
        let mut saw_explicit_stroke = false;
        let mut path_stroke = None;
        let mut path_stroke_width = NumberValue::Static { value: 1.0 };
        let mut path_stroke_dash = None;
        let mut path_stroke_dash_offset = NumberValue::Static { value: 0.0 };
        let mut path_stroke_cap = Cap::Butt;
        let mut path_stroke_join = Join::Miter;
        let mut path_stroke_miter_limit = 4.0;
        let mut path_arrow_start = None;
        let mut path_arrow_end = None;
        let mut path_arrow_size = 8.0;
        let mut path_trim_start = NumberValue::Static { value: 0.0 };
        let mut path_trim_end = NumberValue::Static { value: 1.0 };
        let mut formula_latex = None;
        let mut formula_display = false;
        let mut formula_aria = None;
        let mut image_source = None;
        let mut video_source = None;
        let mut video_source_start = None;
        let mut video_speed = None;
        // Text split and per-unit style must be supplied together.
        let mut text_split: Option<TextSplit> = None;
        let mut text_per_unit: Option<UnitStyle> = None;
        let mut text_path: Option<PathValue> = None;
        let mut effect_fill_rule = FillRule::NonZero;
        let mut mask_paint = None;
        let mut mask_image = None;
        let mut mask_mode = MaskMode::Alpha;
        let mut mask_rect = None;
        let mut batch_geometry = None;
        let mut batch_positions = None;
        let mut batch_position_field = None;
        let mut batch_sizes = None;
        let mut batch_size_field = None;
        let mut batch_fills = None;
        let mut batch_fill_field = None;
        let mut batch_opacities = None;
        let mut batch_opacity_field = None;
        let mut batch_semantic_keys = None;
        let mut shader_source = None;
        let mut shader_inputs: Option<&'s ObjectExpression<'s>> = None;
        let mut shader_uniforms: Option<&'s ObjectExpression<'s>> = None;
        let mut scene3d_camera: Option<&'s ObjectExpression<'s>> = None;
        for attribute in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                self.illegal(DiagCode::GrammarForbidden, attribute.span(), "JSX spread attributes are illegal because property order and provenance must stay explicit");
                continue;
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    attribute.name.span(),
                    "namespaced attributes are illegal",
                );
                continue;
            };
            match name.name.as_str() {
                "key" => key = self.attr_static_string(&attribute.value, attribute.span(), "key"),
                // Allow the scene camera only on Scene, the final composition boundary.
                "camera" if kind_tag == "group" => {
                    self.lower_camera(&attribute.value, attribute.span());
                }
                "camera" if kind_tag == "scene3d" => {
                    scene3d_camera = self.attr_object_literal(
                        &attribute.value,
                        attribute.span(),
                        "Scene3D camera",
                    );
                }
                "latex" if kind_tag == "math-formula" => {
                    formula_latex =
                        self.attr_static_string(&attribute.value, attribute.span(), "latex");
                }
                "displayMode" if kind_tag == "math-formula" => {
                    let Some(mode) =
                        self.attr_static_string(&attribute.value, attribute.span(), "displayMode")
                    else {
                        continue;
                    };
                    match mode.as_str() {
                        "inline" => formula_display = false,
                        "display" => formula_display = true,
                        other => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                format!("displayMode must be \"inline\" or \"display\", got `{other}`"),
                            );
                        }
                    }
                }
                "ariaLabel" if kind_tag == "math-formula" => {
                    formula_aria =
                        self.attr_static_string(&attribute.value, attribute.span(), "ariaLabel");
                }
                "className" => {
                    if let Some(value) = self.attr_static_string(&attribute.value, attribute.span(), "className") {
                        for class_name in value.split_whitespace() {
                            match validate_tailwind_class(class_name) {
                                Ok(()) => class_names.push(class_name.to_string()),
                                Err(TailwindClassError::Forbidden) => self.illegal(
                                    DiagCode::TailwindForbidden,
                                    attribute.span(),
                                    format!("Tailwind class `{class_name}` is forbidden in frame-pure Motion"),
                                ),
                                Err(TailwindClassError::Unsupported) => self.push_diagnostic(
                                    diagnostic_at(
                                        self.source,
                                        DiagCode::TailwindUnsupported,
                                        attribute.span(),
                                        format!("Tailwind class `{class_name}` is not in {TAILWIND_CATALOG}"),
                                    ),
                                ),
                            }
                        }
                    }
                }
                "style" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                        self.illegal(DiagCode::GrammarForbidden, attribute.span(), "style must be an object literal");
                        continue;
                    };
                    let JSXExpression::ObjectExpression(object) = &container.expression else {
                        self.illegal(DiagCode::GrammarForbidden, container.span(), "style must be an object literal");
                        continue;
                    };
                    if style_object.replace(object).is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            attribute.span(),
                            "style may only be declared once on a node",
                        );
                    }
                }
                "layoutId" if kind_tag == "box" => {
                    layout_id = self.attr_static_string(
                        &attribute.value,
                        attribute.span(),
                        "layoutId",
                    );
                }
                "layoutId" => {
                    self.unsupported(
                        attribute.span(),
                        "layoutId is admitted only on View/Flex/Absolute box nodes",
                    );
                }
                "visible" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                        self.illegal(DiagCode::GrammarForbidden, attribute.span(), "visible must be a boolean expression");
                        continue;
                    };
                    if let Some(expression) = container.expression.as_expression() {
                        visibility = self.lower_expr(expression);
                    }
                }
                "d" if kind_tag == "path" => {
                    path_d = self.attr_path_value(&attribute.value, attribute.span());
                }
                name if matches!(
                    (svg_shape, name),
                    (Some("circle"), "cx" | "cy" | "r")
                        | (Some("ellipse"), "cx" | "cy" | "rx" | "ry")
                        | (Some("rect"), "x" | "y" | "width" | "height")
                        | (Some("line"), "x1" | "y1" | "x2" | "y2")
                ) =>
                {
                    if let Some(value) = self.attr_number_expr(
                        &attribute.value,
                        attribute.span(),
                        name,
                    ) {
                        shape_numbers.insert(name.to_owned(), value);
                    }
                }
                "points" if matches!(svg_shape, Some("polyline" | "polygon")) => {
                    shape_points = self.attr_shape_points(&attribute.value, attribute.span());
                }
                "geometry" if kind_tag == "geometry-batch" => {
                    batch_geometry = match self
                        .attr_static_string(&attribute.value, attribute.span(), "geometry")
                        .as_deref()
                    {
                        Some("circle") => Some(GeometryBatchGeometry::Circle),
                        Some("rect") => Some(GeometryBatchGeometry::Rect),
                        Some(_) => {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "GeometryBatch geometry must be circle or rect");
                            None
                        }
                        None => None,
                    };
                }
                "positions" if kind_tag == "geometry-batch" => {
                    if let Some((positions, field)) =
                        self.attr_batch_positions(&attribute.value, attribute.span())
                    {
                        batch_positions = Some(positions);
                        batch_position_field = field;
                    }
                }
                "sizes" if kind_tag == "geometry-batch" => {
                    if let Some((sizes, field)) =
                        self.attr_batch_sizes(&attribute.value, attribute.span())
                    {
                        batch_sizes = Some(sizes);
                        batch_size_field = field;
                    }
                }
                "fills" if kind_tag == "geometry-batch" => {
                    if let Some((fills, field)) =
                        self.attr_batch_fills(&attribute.value, attribute.span())
                    {
                        batch_fills = Some(fills);
                        batch_fill_field = field;
                    }
                }
                "opacities" if kind_tag == "geometry-batch" => {
                    if let Some((opacities, field)) =
                        self.attr_batch_opacities(&attribute.value, attribute.span())
                    {
                        batch_opacities = Some(opacities);
                        batch_opacity_field = field;
                    }
                }
                "semanticKeys" if kind_tag == "geometry-batch" => {
                    batch_semantic_keys = self.attr_static_strings(&attribute.value, attribute.span(), "semanticKeys");
                }
                "source" if kind_tag == "shader-layer" => {
                    let value = self.attr_static_string(
                        &attribute.value,
                        attribute.span(),
                        "source",
                    );
                    if shader_source.replace(value.unwrap_or_default()).is_some() {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            attribute.span(),
                            "ShaderLayer source may only be declared once",
                        );
                    }
                }
                "inputs" | "uniforms" if kind_tag == "shader-layer" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            attribute.span(),
                            format!("{0} must be an object literal", name.name),
                        );
                        continue;
                    };
                    let JSXExpression::ObjectExpression(object) = &container.expression else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            container.span(),
                            format!("{0} must be an object literal", name.name),
                        );
                        continue;
                    };
                    if name.name == "inputs" {
                        if shader_inputs.replace(object).is_some() {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "ShaderLayer inputs may only be declared once",
                            );
                        }
                    } else {
                        if shader_uniforms.replace(object).is_some() {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "ShaderLayer uniforms may only be declared once",
                            );
                        }
                    }
                }
                "path" | "d" if kind_tag == "clip" => {
                    path_d = self.attr_path_value(&attribute.value, attribute.span());
                }
                // Text paths share path parsing and static, prepare-time, and per-frame evaluation
                // policies.
                "path" if kind_tag == "text" => {
                    text_path = self.attr_path_value(&attribute.value, attribute.span());
                }
                "fillRule" if kind_tag == "clip" => {
                    effect_fill_rule = match self
                        .attr_static_string(&attribute.value, attribute.span(), "fillRule")
                        .as_deref()
                    {
                        Some("nonZero") => FillRule::NonZero,
                        Some("evenOdd") => FillRule::EvenOdd,
                        Some(_) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "Clip fillRule must be nonZero or evenOdd",
                            );
                            FillRule::NonZero
                        }
                        None => FillRule::NonZero,
                    };
                }
                "split" if kind_tag == "text" => {
                    if let Some(value) =
                        self.attr_static_string(&attribute.value, attribute.span(), "split")
                    {
                        match value.as_str() {
                            "char" => text_split = Some(TextSplit::Char),
                            "word" => text_split = Some(TextSplit::Word),
                            // Split on source newlines, not layout wrapping.
                            "line" => text_split = Some(TextSplit::Line),
                            _ => self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "Text split must be \"char\", \"word\" or \"line\"",
                            ),
                        }
                    }
                }
                "perUnit" if kind_tag == "text" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value
                    else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            attribute.span(),
                            "perUnit must be an object literal",
                        );
                        continue;
                    };
                    let JSXExpression::ObjectExpression(object) = &container.expression else {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            container.span(),
                            "perUnit must be an object literal",
                        );
                        continue;
                    };
                    text_per_unit = Some(self.lower_unit_style(object));
                }
                "trimStart" if kind_tag == "path" => {
                    if let Some(value) =
                        self.attr_number_value(&attribute.value, attribute.span(), "trimStart")
                    {
                        path_trim_start = value;
                    }
                }
                "trimEnd" if kind_tag == "path" => {
                    if let Some(value) =
                        self.attr_number_value(&attribute.value, attribute.span(), "trimEnd")
                    {
                        path_trim_end = value;
                    }
                }
                "fill" if kind_tag == "path" => {
                    saw_explicit_fill = true;
                    path_fill = self.attr_paint_value(&attribute.value, attribute.span(), "fill");
                }
                "stroke" if kind_tag == "path" => {
                    saw_explicit_stroke = true;
                    path_stroke = self.attr_paint_value(&attribute.value, attribute.span(), "stroke");
                }
                "strokeWidth" if kind_tag == "path" => {
                    if let Some(value) = self.attr_nonnegative_number_value(
                        &attribute.value,
                        attribute.span(),
                        "strokeWidth",
                    ) {
                        path_stroke_width = value;
                    }
                }
                "strokeLinecap" | "strokeLineCap" | "strokeCap" if kind_tag == "path" => {
                    if let Some(value) = self.attr_static_string(&attribute.value, attribute.span(), "strokeLinecap") {
                        path_stroke_cap = match value.as_str() {
                            "butt" => Cap::Butt,
                            "round" => Cap::Round,
                            "square" => Cap::Square,
                            _ => {
                                self.illegal(DiagCode::GrammarForbidden, attribute.span(), "strokeLinecap must be butt, round, or square");
                                Cap::Butt
                            }
                        };
                    }
                }
                "strokeLinejoin" | "strokeLineJoin" | "strokeJoin" if kind_tag == "path" => {
                    if let Some(value) = self.attr_static_string(&attribute.value, attribute.span(), "strokeLinejoin") {
                        path_stroke_join = match value.as_str() {
                            "miter" => Join::Miter,
                            "round" => Join::Round,
                            "bevel" => Join::Bevel,
                            _ => {
                                self.illegal(DiagCode::GrammarForbidden, attribute.span(), "strokeLinejoin must be miter, round, or bevel");
                                Join::Miter
                            }
                        };
                    }
                }
                "strokeDasharray" | "strokeDash" if kind_tag == "path" => {
                    let parsed = match &attribute.value {
                        Some(JSXAttributeValue::StringLiteral(value)) => value
                            .value
                            .split(|character: char| {
                                character == ',' || character.is_ascii_whitespace()
                            })
                            .filter(|part| !part.is_empty())
                            .map(str::parse::<f64>)
                            .collect::<Result<Vec<_>, _>>()
                            .ok(),
                        Some(JSXAttributeValue::ExpressionContainer(container)) => container
                            .expression
                            .as_expression()
                            .and_then(|expression| self.eval_static(expression))
                            .and_then(|value| {
                                value.as_array().map(|items| {
                                    items
                                        .iter()
                                        .map(serde_json::Value::as_f64)
                                        .collect::<Option<Vec<_>>>()
                                })
                            })
                            .flatten(),
                        _ => None,
                    };
                    match parsed {
                        Some(mut dash)
                            if !dash.is_empty()
                                && dash.iter().all(|value| value.is_finite() && *value >= 0.0)
                                && dash.iter().any(|value| *value > 0.0) =>
                        {
                            if dash.len() % 2 == 1 {
                                let repeated = dash.clone();
                                dash.extend(repeated);
                            }
                            path_stroke_dash = Some(dash);
                        }
                        _ => self.illegal(DiagCode::GrammarForbidden, attribute.span(), "strokeDasharray must be a static string/number array containing finite non-negative lengths and at least one positive length"),
                    }
                }
                "strokeDashoffset" | "strokeDashOffset" if kind_tag == "path" => {
                    path_stroke_dash_offset = match &attribute.value {
                        Some(JSXAttributeValue::StringLiteral(value)) => {
                            match value.value.parse::<f64>() {
                                Ok(value) if value.is_finite() => NumberValue::Static { value },
                                _ => {
                                    self.illegal(
                                        DiagCode::GrammarForbidden,
                                        attribute.span(),
                                        "strokeDashoffset must be a finite number or frame expression",
                                    );
                                    continue;
                                }
                            }
                        }
                        Some(JSXAttributeValue::ExpressionContainer(container)) => {
                            let Some(expression) = container.expression.as_expression() else {
                                self.illegal(
                                    DiagCode::GrammarForbidden,
                                    container.span(),
                                    "strokeDashoffset expression cannot be empty",
                                );
                                continue;
                            };
                            let Some(value) = self.number_binding(
                                expression,
                                "strokeDashoffset",
                                f64::is_finite,
                            ) else {
                                continue;
                            };
                            value
                        }
                        _ => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "strokeDashoffset must be a finite number or frame expression",
                            );
                            continue;
                        }
                    };
                }
                "strokeMiterlimit" | "strokeMiterLimit" if kind_tag == "path" => {
                    if let Some(value) = self.attr_static_string(&attribute.value, attribute.span(), "strokeMiterlimit") {
                        match value.parse::<f64>() {
                            Ok(value) if value.is_finite() && value > 0.0 => path_stroke_miter_limit = value,
                            _ => self.illegal(DiagCode::GrammarForbidden, attribute.span(), "strokeMiterlimit must be a finite positive static number"),
                        }
                    }
                }
                "arrowStart" if kind_tag == "path" => {
                    path_arrow_start = self.attr_arrow_kind(&attribute.value, attribute.span(), "arrowStart");
                }
                "arrowEnd" if kind_tag == "path" => {
                    path_arrow_end = self.attr_arrow_kind(&attribute.value, attribute.span(), "arrowEnd");
                }
                "arrowSize" if kind_tag == "path" => {
                    if let Some(value) = self.attr_static_string(&attribute.value, attribute.span(), "arrowSize") {
                        match value.parse::<f64>() {
                            Ok(value) if value.is_finite() && value > 0.0 => path_arrow_size = value,
                            _ => self.illegal(DiagCode::GrammarForbidden, attribute.span(), "arrowSize must be a finite positive static number"),
                        }
                    }
                }
                // Video sources use static asset controls; source start and playback speed accept
                // numeric expressions.
                "src" if kind_tag == "video" => {
                    video_source =
                        self.attr_static_string(&attribute.value, attribute.span(), "src");
                }
                "sourceStart" if kind_tag == "video" => {
                    video_source_start = self.attr_number_value(
                        &attribute.value,
                        attribute.span(),
                        "sourceStart",
                    );
                }
                "speed" if kind_tag == "video" => {
                    video_speed =
                        self.attr_number_value(&attribute.value, attribute.span(), "speed");
                }
                "src" if kind_tag == "image" => {
                    image_source = self.attr_static_string(
                        &attribute.value,
                        attribute.span(),
                        "src",
                    );
                }
                "paint" if kind_tag == "mask" => {
                    mask_paint = self.attr_paint_value(&attribute.value, attribute.span(), "paint");
                }
                "src" if kind_tag == "mask" => {
                    mask_image = self.attr_static_string(
                        &attribute.value,
                        attribute.span(),
                        "src",
                    );
                }
                "mode" if kind_tag == "mask" => {
                    mask_mode = match self
                        .attr_static_string(&attribute.value, attribute.span(), "mode")
                        .as_deref()
                    {
                        Some("alpha") => MaskMode::Alpha,
                        Some("luminance") => MaskMode::Luminance,
                        Some(_) => {
                            self.illegal(
                                DiagCode::GrammarForbidden,
                                attribute.span(),
                                "Mask mode must be alpha or luminance",
                            );
                            MaskMode::Alpha
                        }
                        None => MaskMode::Alpha,
                    };
                }
                "rect" if kind_tag == "mask" => {
                    mask_rect = self.attr_rect_value(&attribute.value, attribute.span(), "rect");
                }
                name if matches!(kind_tag, "glass" | "glass-field") => {
                    self.lower_glass_attribute(name, attribute, &mut glass_props);
                }
                other if other.starts_with("on") => self.illegal(DiagCode::GrammarForbidden, attribute.span(), format!("event handler `{other}` is a side effect and is illegal in frame-pure Motion JSX")),
                other => self.unsupported(attribute.span(), format!("attribute `{other}` is legal on some frontend elements but is not in the admitted capability set")),
            }
        }
        let has_layout_transition = style_object.is_some_and(|object| {
            object.properties.iter().any(|property| {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    return false;
                };
                let name = match &property.key {
                    PropertyKey::StaticIdentifier(name) => Some(name.name.as_str()),
                    PropertyKey::StringLiteral(name) => Some(name.value.as_str()),
                    _ => None,
                };
                name.is_some_and(|name| camel_to_kebab(name) == "layout-transition")
            })
        });
        if let Some(layout_id) = layout_id.as_deref() {
            let scoped = self.scoped_key(layout_id);
            if !self.layout_ids.insert(scoped) {
                self.illegal(
                    DiagCode::ArtifactInvalid,
                    element.opening_element.span(),
                    format!("duplicate layoutId `{layout_id}` in one component instance"),
                );
            }
            if !has_layout_transition {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.opening_element.span(),
                    format!(
                        "layoutId `{layout_id}` must be consumed by style.layoutTransition: flip(...)"
                    ),
                );
            }
        }
        if let Some(object) = style_object {
            styles.extend(self.lower_style(object, path, layout_id.as_deref()));
        }
        self.diagnose_border_paint(element.opening_element.span(), &class_names, &styles);
        if let Some(shape) = svg_shape {
            if path_d.is_some() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    format!(
                        "<{tag}> derives its path from shape attributes and cannot also declare d"
                    ),
                );
            } else {
                path_d = self.lower_svg_shape(shape, &shape_numbers, shape_points, element.span());
            }
        }
        if self.list_depth > 0 && key.is_none() {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.opening_element.span(),
                "every data-driven list item must declare a prepare-time stable `key`",
            );
        }
        let key = self.scoped_key(&key.unwrap_or_else(|| path.to_string()));
        if !self.keys.insert(key.clone()) {
            self.illegal(
                DiagCode::ArtifactInvalid,
                element.span(),
                format!("duplicate node key `{key}`"),
            );
        }

        if kind_tag == "text" {
            enum RichItem {
                Text(TextValue, Vec<StyleBinding>, Span),
                InlineImage(PendingNode),
            }
            let mut items = Vec::new();
            let mut saw_span = false;
            let mut saw_inline_image = false;
            for (index, child) in element.children.iter().enumerate() {
                match child {
                    JSXChild::Text(value) => {
                        if let Some(value) = Self::jsx_text_value(value.value.as_str()) {
                            items.push(RichItem::Text(
                                TextValue::Static { value },
                                Vec::new(),
                                child.span(),
                            ));
                        }
                    }
                    JSXChild::ExpressionContainer(container)
                        if matches!(container.expression, JSXExpression::EmptyExpression(_)) => {}
                    JSXChild::ExpressionContainer(container) => {
                        if let Some(expression) = container.expression.as_expression()
                            && let Some(text) = self.lower_text_value(expression)
                        {
                            items.push(RichItem::Text(text, Vec::new(), child.span()));
                        }
                    }
                    JSXChild::Element(child)
                        if matches!(
                            &child.opening_element.name,
                            JSXElementName::Identifier(name) if name.name == "Span"
                        ) || matches!(
                            &child.opening_element.name,
                            JSXElementName::IdentifierReference(name) if name.name == "Span"
                        ) =>
                    {
                        saw_span = true;
                        items.extend(
                            self.lower_span_runs(child, &format!("{path}.span.{index}"))
                                .into_iter()
                                .map(|(text, styles, span)| RichItem::Text(text, styles, span)),
                        );
                    }
                    JSXChild::Element(child)
                        if matches!(
                            &child.opening_element.name,
                            JSXElementName::Identifier(name) if matches!(name.name.as_str(), "Image" | "img")
                        ) || matches!(
                            &child.opening_element.name,
                            JSXElementName::IdentifierReference(name) if matches!(name.name.as_str(), "Image" | "img")
                        ) =>
                    {
                        if let Some(mut image) =
                            self.lower_jsx(child, &format!("{path}.inline-image.{index}"))
                        {
                            if !image.class_names.iter().any(|class_name| {
                                matches!(
                                    class_name.as_str(),
                                    "inline" | "inline-block" | "inline-flex" | "inline-grid"
                                )
                            }) {
                                image.class_names.push("inline-block".into());
                            }
                            saw_inline_image = true;
                            items.push(RichItem::InlineImage(image));
                        }
                    }
                    _ => self.unsupported(
                        child.span(),
                        "Text children must be static text, string/number expressions, <Span>, or <Image>",
                    ),
                }
            }
            // Adjacent plain Text children are one semantic string, not rich text. JSX written by
            // generators commonly mixes literals and expressions (`{team} · {owner}`); lowering
            // those pieces as independent inline runs needlessly enables the rich-text capability
            // and makes whitespace collapse/source addressing span several synthetic nodes.
            // Fuse them into the same Template expression used by a template literal. Explicit
            // <Span> or inline images still keep their authored run boundaries below.
            if !saw_span && !saw_inline_image && items.len() > 1 {
                let mut value = String::new();
                let mut parts = Vec::new();
                let mut dynamic = false;
                for item in std::mem::take(&mut items) {
                    let RichItem::Text(text, _, _) = item else {
                        unreachable!("plain Text fusion only contains text items")
                    };
                    match text {
                        TextValue::Static { value: text } => {
                            value.push_str(&text);
                            parts.push(valle_motion::TemplatePart::Text { value: text });
                        }
                        TextValue::Expr { expr } => {
                            dynamic = true;
                            parts.push(valle_motion::TemplatePart::Expr { expr });
                        }
                    }
                }
                let text = if dynamic {
                    TextValue::Expr {
                        expr: self.push(Expr::Template { parts }, element.span()),
                    }
                } else {
                    TextValue::Static { value }
                };
                items.push(RichItem::Text(text, Vec::new(), element.span()));
            }

            let rich = saw_span || saw_inline_image || items.len() > 1;
            let per_unit = self.close_per_unit(element.span(), text_split, text_per_unit);
            if rich && per_unit.is_some() {
                self.unsupported(
                    element.span(),
                    "multi-run Text cannot yet combine with split/perUnit because unit indices must span the whole paragraph",
                );
            }
            if rich && text_path.is_some() {
                self.unsupported(
                    element.span(),
                    "multi-run Text cannot yet combine with text-on-path because path distance must span the whole paragraph",
                );
            }
            if !rich {
                let text = items
                    .pop()
                    .and_then(|item| match item {
                        RichItem::Text(text, _, _) => Some(text),
                        RichItem::InlineImage(_) => None,
                    })
                    .unwrap_or(TextValue::Static {
                        value: String::new(),
                    });
                return Some(PendingNode {
                    space: None,
                    span: element.span(),
                    expansion_stack: self.component_stack.clone(),
                    key,
                    kind: NodeKind::Text {
                        text,
                        per_unit,
                        path: text_path,
                    },
                    class_names,
                    styles,
                    visibility,
                    semantic: None,
                    children: Vec::new(),
                    is_mask_source: false,
                });
            }

            self.extra_capabilities
                .insert(RICH_TEXT_CAPABILITY.to_owned());
            if saw_inline_image {
                self.extra_capabilities
                    .insert(valle_motion::RICH_TEXT_INLINE_IMAGE_CAPABILITY.to_owned());
            }
            let mut children = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                match item {
                    RichItem::Text(text, run_styles, span) => {
                        let run_key = format!("{key}/__run_{index}__");
                        if !self.keys.insert(run_key.clone()) {
                            self.illegal(
                                DiagCode::ArtifactInvalid,
                                span,
                                format!("duplicate rich-text run key `{run_key}`"),
                            );
                        }
                        children.push(PendingNode {
                            space: None,
                            span,
                            expansion_stack: self.component_stack.clone(),
                            key: run_key,
                            kind: NodeKind::Text {
                                text,
                                per_unit: None,
                                path: None,
                            },
                            class_names: Vec::new(),
                            styles: run_styles,
                            visibility: None,
                            semantic: None,
                            children: Vec::new(),
                            is_mask_source: false,
                        });
                    }
                    RichItem::InlineImage(image) => children.push(image),
                }
            }
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::Group,
                class_names,
                styles,
                visibility,
                semantic: None,
                children,
                is_mask_source: false,
            });
        }

        if kind_tag == "path" {
            let Some(d) = path_d else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "Path requires a typed d attribute",
                );
                return None;
            };
            self.diagnose_path_paint(
                element.opening_element.span(),
                tag.as_str(),
                svg_shape,
                &d,
                saw_explicit_fill,
                saw_explicit_stroke,
            );
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::Path {
                    d,
                    fill: path_fill,
                    stroke: path_stroke.map(|paint| {
                        Box::new(PathStroke {
                            paint,
                            width: path_stroke_width,
                            dash: path_stroke_dash,
                            dash_offset: path_stroke_dash_offset,
                            cap: path_stroke_cap,
                            join: path_stroke_join,
                            miter_limit: path_stroke_miter_limit,
                        })
                    }),
                    trim_start: path_trim_start,
                    trim_end: path_trim_end,
                    arrow_start: path_arrow_start.map(|kind| ArrowSpec {
                        kind,
                        size: path_arrow_size,
                    }),
                    arrow_end: path_arrow_end.map(|kind| ArrowSpec {
                        kind,
                        size: path_arrow_size,
                    }),
                },
                class_names,
                styles,
                visibility,
                semantic: None,
                children: Vec::new(),
                is_mask_source: false,
            });
        }

        if kind_tag == "geometry-batch" {
            if element
                .children
                .iter()
                .any(|child| !matches!(child, JSXChild::Text(text) if text.value.trim().is_empty()))
            {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "GeometryBatch is a leaf and cannot have children",
                );
            }
            let (Some(geometry), Some(positions), Some(sizes), Some(fills)) =
                (batch_geometry, batch_positions, batch_sizes, batch_fills)
            else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "GeometryBatch requires geometry, positions, sizes, and fills",
                );
                return None;
            };
            self.extra_capabilities
                .insert(GEOMETRY_BATCH_CAPABILITY.to_owned());
            let has_frame_fields = batch_position_field.is_some()
                || batch_size_field.is_some()
                || batch_fill_field.is_some()
                || batch_opacity_field.is_some();
            if matches!(positions, BatchPositions::Particles { .. }) {
                if has_frame_fields {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        "GeometryBatch field(...) cannot be combined with particles(...); particle life curves already own the batch fields",
                    );
                    return None;
                }
                self.extra_capabilities
                    .insert(PARTICLE_FIELD_CAPABILITY.to_owned());
            }
            if has_frame_fields {
                self.extra_capabilities
                    .insert(GEOMETRY_BATCH_FIELD_CAPABILITY.to_owned());
            }
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::GeometryBatch {
                    batch: GeometryBatchSpec {
                        geometry,
                        positions,
                        sizes,
                        fills,
                        opacities: batch_opacities.unwrap_or_default(),
                        semantic_keys: batch_semantic_keys.unwrap_or_default(),
                        position_field: batch_position_field,
                        size_field: batch_size_field,
                        fill_field: batch_fill_field,
                        opacity_field: batch_opacity_field,
                    },
                },
                class_names,
                styles,
                visibility,
                semantic: None,
                children: Vec::new(),
                is_mask_source: false,
            });
        }

        if kind_tag == "scene3d" {
            return self.lower_scene3d_node(
                element,
                key,
                class_names,
                styles,
                visibility,
                scene3d_camera,
            );
        }

        if kind_tag == "video" {
            // Declare video capability when used so unsupported consumers reject the artifact.
            self.extra_capabilities
                .insert(valle_motion::VIDEO_CAPABILITY.to_owned());
            let Some(source) = video_source else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "Video requires a static `src` asset reference (asset://<video-control>)",
                );
                return None;
            };
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::Video {
                    source,
                    // Default to source time zero at normal playback speed.
                    source_start: video_source_start.unwrap_or(NumberValue::Static { value: 0.0 }),
                    speed: video_speed.unwrap_or(NumberValue::Static { value: 1.0 }),
                },
                class_names,
                styles,
                visibility,
                semantic: None,
                children: Vec::new(),
                is_mask_source: false,
            });
        }

        if kind_tag == "math-formula" {
            if !element.children.is_empty() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "<MathFormula> cannot have children; latex is the only source of truth",
                );
                return None;
            }
            let Some(latex) = formula_latex else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "MathFormula requires a static `latex` string",
                );
                return None;
            };
            let policy = valle_motion::math_formula::AdmitPolicy::default();
            match valle_motion::math_formula::classify_formula(&latex, formula_display, &policy) {
                row if row.class == valle_motion::math_formula::Classification::Accepted => {}
                row => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        format!("MathFormula rejected: {}", row.detail),
                    );
                    return None;
                }
            }
            self.extra_capabilities
                .insert(MATH_FORMULA_CAPABILITY.to_owned());
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::MathFormula {
                    latex,
                    display: formula_display,
                    aria_label: formula_aria,
                },
                class_names,
                styles,
                visibility,
                semantic: None,
                children: Vec::new(),
                is_mask_source: false,
            });
        }

        if kind_tag == "image" {
            let Some(source) = image_source else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "Image requires a static `src` asset reference",
                );
                return None;
            };
            return Some(PendingNode {
                space: None,
                span: element.span(),
                expansion_stack: self.component_stack.clone(),
                key,
                kind: NodeKind::Image { source },
                class_names,
                styles,
                visibility,
                semantic: None,
                children: Vec::new(),
                is_mask_source: false,
            });
        }

        if kind_tag == "clip" && path_d.is_none() {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.span(),
                "Clip requires a typed path attribute",
            );
            return None;
        }
        let shader_kind = if kind_tag == "shader-layer" {
            let Some(source) = shader_source else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "ShaderLayer requires a static source=\"shader://<name>@<version>\"",
                );
                return None;
            };
            self.lower_shader_layer(
                &source,
                shader_inputs,
                shader_uniforms,
                element.opening_element.span(),
            )
        } else {
            None
        };
        if kind_tag == "shader-layer" && shader_kind.is_none() {
            return None;
        }

        let mut children = Vec::new();
        for (index, child) in element.children.iter().enumerate() {
            match child {
                JSXChild::Element(child) => {
                    if let Some(child) = self.lower_jsx(child, &format!("{path}.{index}")) {
                        children.push(child);
                    }
                }
                JSXChild::Text(value) if value.value.trim().is_empty() => {}
                JSXChild::ExpressionContainer(container)
                    if matches!(container.expression, JSXExpression::EmptyExpression(_)) => {}
                JSXChild::ExpressionContainer(container) => {
                    if let Some(expression) = container.expression.as_expression()
                        && let Some(mut lowered) =
                            self.lower_node_expression(expression, &format!("{path}.{index}"))
                    {
                        children.append(&mut lowered);
                    }
                }
                _ => self.unsupported(child.span(), "container child is not valid Motion JSX"),
            }
        }
        if kind_tag != "mask" && children.iter().any(|child| child.is_mask_source) {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.span(),
                "MaskSource must be a direct child of Mask",
            );
            return None;
        }
        let subtree_source = if kind_tag == "mask" {
            if mask_rect.is_none() {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "Mask requires a positive typed rect",
                );
                return None;
            }
            let source_children = children
                .iter()
                .enumerate()
                .filter(|(_, child)| child.is_mask_source)
                .collect::<Vec<_>>();
            let value_sources =
                usize::from(mask_paint.is_some()) + usize::from(mask_image.is_some());
            if value_sources + source_children.len() != 1 {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    element.span(),
                    "Mask requires exactly one source: paint, src, or one direct MaskSource child",
                );
                return None;
            }
            if let Some((index, source)) = source_children.first() {
                if mask_mode != MaskMode::Alpha {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        "subtree MaskSource currently requires mode=\"alpha\"",
                    );
                    return None;
                }
                if *index + 1 != children.len() {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        "MaskSource must be the final direct child so it composites over preceding mask content",
                    );
                    return None;
                }
                Some(source.key.clone())
            } else {
                None
            }
        } else {
            None
        };
        Some(PendingNode {
            span: element.span(),
            expansion_stack: self.component_stack.clone(),
            key,
            space: match kind_tag {
                "world" => Some(CoordinateSpace::World),
                "screen" => Some(CoordinateSpace::Screen),
                _ => None,
            },
            kind: match kind_tag {
                "group" | "world" | "screen" | "mask-source" => NodeKind::Group,
                "box" => NodeKind::Box,
                "clip" => NodeKind::Clip {
                    path: path_d.expect("checked above"),
                    fill_rule: effect_fill_rule,
                },
                "mask" => NodeKind::Mask {
                    source: match (mask_paint, mask_image, subtree_source) {
                        (Some(paint), None, None) => MaskValue::Paint { paint },
                        (None, Some(source), None) => MaskValue::Image { source },
                        (None, None, Some(source)) => MaskValue::Subtree { source },
                        _ => unreachable!("checked above"),
                    },
                    mode: mask_mode,
                    rect: mask_rect.expect("checked above"),
                },
                "shader-layer" => shader_kind.expect("lowered above"),
                "glass" | "glass-field" => {
                    match self.finish_glass_kind(kind_tag, &glass_props, element.span()) {
                        Some(kind) => kind,
                        None => return None,
                    }
                }
                _ => unreachable!("leaf kinds returned above"),
            },
            class_names,
            styles,
            visibility,
            semantic: None,
            children,
            is_mask_source: kind_tag == "mask-source",
        })
    }
}
