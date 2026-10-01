//! JSX intrinsic dispatch and ordinary Scene node lowering.

use super::*;

impl<'s> Compiler<'s> {
    pub(super) fn lower_jsx(
        &mut self,
        element: &'s JSXElement<'s>,
        path: &str,
    ) -> Option<PendingNode> {
        let list_depth = self.list_depth;
        if list_depth > 0 && !element.opening_element.attributes.iter().any(|attribute| {
            matches!(attribute, JSXAttributeItem::Attribute(attribute)
                if matches!(&attribute.name, JSXAttributeName::Identifier(name) if name.name == "key"))
        }) {
            self.illegal(DiagCode::GrammarForbidden, element.opening_element.span(),
                "every data-driven list item must declare a prepare-time stable `key`");
        }
        self.list_depth = 0;
        let result = self.lower_jsx_node(element, path, list_depth > 0);
        self.list_depth = list_depth;
        result
    }

    fn lower_jsx_node(
        &mut self,
        element: &'s JSXElement<'s>,
        path: &str,
        list_root: bool,
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
            "Transition" => ("transition", None),
            "Shutter" => ("shutter", None),
            "Echo" => ("echo", None),
            "TimeScope" => ("time-scope", None),
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
        let mut class_conditions = BTreeMap::new();
        let mut styles = Vec::new();
        let mut style_object = None;
        let mut layout_id = None;
        let mut visibility = None;
        let mut glass_props = super::glass::GlassProps::default();
        let mut path_d = None;
        let mut shape_numbers = BTreeMap::<String, ExprId>::new();
        let mut shape_points = None;
        let mut path_fill = Some(PaintValue::Solid {
            color: ColorValue::Static {
                value: valle_draw::program::AuthorColor::from_srgb8(Rgba::rgb(0, 0, 0)),
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
        let mut mask_invert = false;
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
        let mut batch_rotations = None;
        let mut batch_rotation_field = None;
        let mut batch_skew_xs = None;
        let mut batch_skew_x_field = None;
        let mut batch_stroke_widths = None;
        let mut batch_stroke_width_field = None;
        let mut batch_semantic_keys = None;
        let mut transition_kind: Option<valle_draw::transition::TransitionKind> = None;
        let mut transition_progress = None;
        let mut transition_params = BTreeMap::new();
        let mut shutter_samples = None;
        let mut shutter_angle = None;
        let mut echo_count = None;
        let mut echo_interval = None;
        let mut echo_decay = None;
        let mut time_offset = None;
        let mut time_speed = None;
        let mut shader_source = None;
        let mut shader_inputs: Option<&'s ObjectExpression<'s>> = None;
        let mut shader_uniforms: Option<&'s ObjectExpression<'s>> = None;
        let mut scene3d_camera: Option<&'s ObjectExpression<'s>> = None;
        let mut scene3d_pbr: Option<&'s ObjectExpression<'s>> = None;
        let mut scene_bloom = None;
        let mut saw_scene_bloom = false;
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
                "key" => {
                    key = if self.instance_scope.as_ref().is_some_and(|scope| {
                        matches!(&attribute.value,
                            Some(JSXAttributeValue::ExpressionContainer(container))
                            if matches!(container.expression.as_expression().map(peel_expr),
                                Some(Expression::StaticMemberExpression(member))
                                if matches!(&member.object, Expression::Identifier(id) if id.name.as_str() == scope.parameter)
                                    && member.property.name.as_str() == scope.key_field))
                    }) {
                        self.instance_scope.as_ref().map(|scope| scope.template_key.clone())
                    } else {
                        self.attr_static_string(&attribute.value, attribute.span(), "key")
                    };
                }
                // Allow the scene camera only on Scene, the final composition boundary.
                "camera" if kind_tag == "group" => {
                    self.lower_camera(&attribute.value, attribute.span());
                }
                "bloom" if tag == "Scene" => {
                    if saw_scene_bloom {
                        self.illegal(DiagCode::GrammarForbidden, attribute.span(),
                            "Scene bloom may only be declared once");
                        continue;
                    }
                    saw_scene_bloom = true;
                    let Some(object) = self.attr_object_literal(&attribute.value, attribute.span(), "Scene bloom") else {
                        continue;
                    };
                    let Some(fields) = self.scene3d_object_values(
                        object,
                        "Scene bloom",
                        &["threshold", "knee", "intensity", "radius"],
                    ) else {
                        continue;
                    };
                    let mut numbers = [None; 4];
                    for (index, (name, max)) in [
                        ("threshold", 1.0),
                        ("knee", 1.0),
                        ("intensity", 4.0),
                        ("radius", 128.0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let Some(expression) = fields.get(name) else { continue };
                        numbers[index] = self.eval_static(expression).and_then(|value| value.as_f64())
                            .filter(|value| value.is_finite() && (0.0..=max).contains(value));
                        if numbers[index].is_none() {
                            self.illegal(DiagCode::GrammarForbidden, expression.span(),
                                format!("Scene bloom {name} must be a static number in 0..={max}"));
                        }
                    }
                    if let (Some(threshold), Some(intensity), Some(radius)) =
                        (numbers[0], numbers[2], numbers[3])
                    {
                        scene_bloom = Some([threshold, numbers[1].unwrap_or(0.1), intensity, radius]);
                    } else {
                        self.illegal(DiagCode::GrammarForbidden, object.span(),
                            "Scene bloom requires threshold, intensity, and radius");
                    }
                }
                "camera" if kind_tag == "scene3d" => {
                    scene3d_camera = self.attr_object_literal(
                        &attribute.value,
                        attribute.span(),
                        "Scene3D camera",
                    );
                }
                "pbr" if kind_tag == "scene3d" => {
                    scene3d_pbr = self.attr_object_literal(&attribute.value,attribute.span(),"Scene3D pbr");
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
                "className" => self.lower_classes(
                    &attribute.value, attribute.span(), path, &mut class_names, &mut class_conditions,
                ),
                "style" => {
                    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                        self.illegal(DiagCode::GrammarForbidden, attribute.span(), "style must be a fixed-shape object expression");
                        continue;
                    };
                    let Some(expression) = container.expression.as_expression() else {
                        self.illegal(DiagCode::GrammarForbidden, container.span(), "style must be a fixed-shape object expression");
                        continue;
                    };
                    if style_object.replace(self.resolve_style(expression)).is_some() {
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
                    batch_geometry = self.attr_batch_geometry(&attribute.value, attribute.span());
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
                "rotations" if kind_tag == "geometry-batch" => {
                    if let Some((rotations, field)) =
                        self.attr_batch_rotations(&attribute.value, attribute.span())
                    {
                        batch_rotations = Some(rotations);
                        batch_rotation_field = field;
                    }
                }
                "skewXs" if kind_tag == "geometry-batch" => {
                    if let Some((values, field)) =
                        self.attr_batch_skew_xs(&attribute.value, attribute.span())
                    {
                        batch_skew_xs = Some(values);
                        batch_skew_x_field = field;
                    }
                }
                "strokeWidths" if kind_tag == "geometry-batch" => {
                    if let Some((values, field)) =
                        self.attr_batch_stroke_widths(&attribute.value, attribute.span())
                    {
                        batch_stroke_widths = Some(values);
                        batch_stroke_width_field = field;
                    }
                }
                "semanticKeys" if kind_tag == "geometry-batch" => {
                    batch_semantic_keys = self.attr_static_strings(&attribute.value, attribute.span(), "semanticKeys");
                }
                "kind" if kind_tag == "transition" => {
                    if let Some(name) = self.attr_static_string(&attribute.value, attribute.span(), "kind") {
                        match serde_json::from_value(serde_json::Value::String(name.clone())) {
                            Ok(kind) => transition_kind = Some(kind),
                            Err(_) => self.illegal(DiagCode::GrammarForbidden, attribute.span(), format!("unknown Transition kind `{name}`")),
                        }
                    }
                }
                "progress" if kind_tag == "transition" => {
                    transition_progress = self.attr_number_value(&attribute.value, attribute.span(), "progress");
                }
                "samples" if kind_tag == "shutter" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "Shutter samples") {
                        if value.fract() == 0.0 && (1.0..=32.0).contains(&value) {
                            shutter_samples = Some(value as u8);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Shutter samples must be an integer in 1..=32");
                        }
                    }
                }
                "angle" if kind_tag == "shutter" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "Shutter angle") {
                        if value.fract() == 0.0 && (0.0..=360.0).contains(&value) {
                            shutter_angle = Some(value as u16);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Shutter angle must be an integer in 0..=360 degrees");
                        }
                    }
                }
                "count" if kind_tag == "echo" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "Echo count") {
                        if value.fract() == 0.0 && (1.0..=32.0).contains(&value) {
                            echo_count = Some(value as u8);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Echo count must be an integer in 1..=32");
                        }
                    }
                }
                "interval" if kind_tag == "echo" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "Echo interval") {
                        if value.fract() == 0.0 && (1.0..=u32::MAX as f64).contains(&value) {
                            echo_interval = Some(value as u32);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Echo interval must be a positive integer frame count");
                        }
                    }
                }
                "decay" if kind_tag == "echo" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "Echo decay") {
                        if value.is_finite() && (0.0..=1.0).contains(&value) {
                            echo_decay = Some(value);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Echo decay must be a finite number in [0,1]");
                        }
                    }
                }
                "offset" if kind_tag == "time-scope" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "TimeScope offset") {
                        if value.is_finite() {
                            time_offset = Some(value);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "TimeScope offset must be finite seconds");
                        }
                    }
                }
                "speed" if kind_tag == "time-scope" => {
                    if let Some(value) = self.attr_static_number(&attribute.value, attribute.span(), "TimeScope speed") {
                        if value.is_finite() {
                            time_speed = Some(value);
                        } else {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "TimeScope speed must be finite");
                        }
                    }
                }
                "params" if kind_tag == "transition" => {
                    let Some(object) = self.attr_object_literal(&attribute.value, attribute.span(), "Transition params") else { continue; };
                    for entry in &object.properties {
                        let ObjectPropertyKind::ObjectProperty(property) = entry else {
                            self.illegal(DiagCode::GrammarForbidden, entry.span(), "Transition params spread is not admitted");
                            continue;
                        };
                        let Some(name) = static_property_name(&property.key) else {
                            self.illegal(DiagCode::GrammarForbidden, property.span(), "Transition parameter names must be static");
                            continue;
                        };
                        if let Some(value) = self.expr_number_value(&property.value) {
                            if transition_params.insert(name.clone(), value).is_some() {
                                self.illegal(DiagCode::GrammarForbidden, property.span(), format!("duplicate Transition parameter `{name}`"));
                            }
                        }
                    }
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
                    video_source_start = self.attr_finite_number_value(
                        &attribute.value,
                        attribute.span(),
                        "sourceStart",
                    );
                }
                "speed" if kind_tag == "video" => {
                    video_speed =
                        self.attr_finite_number_value(&attribute.value, attribute.span(), "speed");
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
                "invert" if kind_tag == "mask" => {
                    mask_invert = match &attribute.value {
                        None => true,
                        Some(JSXAttributeValue::ExpressionContainer(container)) => {
                            match container.expression.as_expression() {
                                Some(Expression::BooleanLiteral(value)) => value.value,
                                _ => {
                                    self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Mask invert must be a static boolean");
                                    false
                                }
                            }
                        }
                        _ => {
                            self.illegal(DiagCode::GrammarForbidden, attribute.span(), "Mask invert must be a static boolean");
                            false
                        }
                    };
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
        let has_layout_transition = style_object
            .as_ref()
            .is_some_and(|object| object.contains("layout-transition"));
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
            styles.extend(self.lower_style(&object, path, layout_id.as_deref()));
        }
        if class_names
            .iter()
            .any(|class| valle_motion::tailwind::advanced_filter(class).is_some())
        {
            self.extra_capabilities
                .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
            if !styles
                .iter()
                .any(|style| style.property == "motion-filter-frame")
            {
                let expr = self.scoped_context_expr(ContextInput::LocalFrame, element.span());
                styles.push(StyleBinding {
                    property: "motion-filter-frame".into(),
                    value: StyleValue::Expr { expr },
                });
            }
        }
        if let Some([threshold, knee, intensity, radius]) = scene_bloom {
            self.extra_capabilities
                .insert(valle_motion::NODE_ADVANCED_FILTER_CAPABILITY.to_owned());
            for (property, value) in [
                ("motion-bloom-threshold", threshold),
                ("motion-bloom-knee", knee),
                ("motion-bloom-intensity", intensity),
                ("motion-bloom-radius", radius),
            ] {
                styles.push(StyleBinding {
                    property: property.into(),
                    value: StyleValue::Static {
                        value: MotionValue::Number(value),
                    },
                });
            }
        }
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
        let key = self.scoped_key(&key.unwrap_or_else(|| path.to_string()));
        // Static descendants are stable when keyed list items are reordered.
        let path = if list_root { key.as_str() } else { path };
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
                            // Contextual primitive default, below the author cascade. Media
                            // conditions and finite class choices can override it independently.
                            image.styles.push(StyleBinding {
                                property: "motion-inline-image".into(),
                                value: StyleValue::Static { value: MotionValue::Bool(true) },
                            });
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
            if rich && text_path.is_some() {
                self.unsupported(
                    element.span(),
                    "a text path lays out one run, and a multi-run Text spans several runs; keep the path text in its own single-run Text, or drop the path",
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
                    class_conditions,
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
                                per_unit: per_unit.as_ref().map(|binding| {
                                    let mut binding = (**binding).clone();
                                    binding.group_key = Some(key.clone());
                                    Box::new(binding)
                                }),
                                path: None,
                            },
                            class_names: Vec::new(),
                            class_conditions: Default::default(),
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
                class_conditions,
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
                class_conditions,
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
                || batch_opacity_field.is_some()
                || batch_rotation_field.is_some()
                || batch_skew_x_field.is_some()
                || batch_stroke_width_field.is_some();
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
                        rotations: batch_rotations.unwrap_or_default(),
                        skew_xs: batch_skew_xs.unwrap_or_default(),
                        stroke_widths: batch_stroke_widths.unwrap_or_default(),
                        semantic_keys: batch_semantic_keys.unwrap_or_default(),
                        position_field: batch_position_field,
                        size_field: batch_size_field,
                        fill_field: batch_fill_field,
                        opacity_field: batch_opacity_field,
                        rotation_field: batch_rotation_field,
                        skew_x_field: batch_skew_x_field,
                        stroke_width_field: batch_stroke_width_field,
                    },
                },
                class_names,
                class_conditions,
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
                class_conditions,
                styles,
                visibility,
                scene3d_camera,
                scene3d_pbr,
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
                class_conditions,
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
                class_conditions,
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
                class_conditions,
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
                    "ShaderLayer requires a static source=\"asset://<shader-control>\"",
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

        let scoped_time = if kind_tag == "time-scope" {
            time_offset
                .zip(time_speed)
                .map(|(offset_seconds, speed)| TimeTransform {
                    offset_seconds,
                    speed,
                })
        } else {
            None
        };
        if let Some(transform) = scoped_time {
            self.time_scopes.push(transform);
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
        if scoped_time.is_some() {
            self.time_scopes.pop();
        }
        if kind_tag == "transition" {
            if transition_kind.is_none()
                || transition_progress.is_none()
                || children.len() != 2
                || children
                    .iter()
                    .any(|child| matches!(child.kind, NodeKind::GlassField(_)))
            {
                self.illegal(DiagCode::GrammarForbidden, element.span(),
                    "Transition requires a static kind, numeric progress, and exactly two child subtrees with layout boxes");
                return None;
            }
            if let Some(NumberValue::Expr { expr }) = transition_progress.as_ref() {
                self.diagnose_parameter_range(*expr, "Transition progress", "", 0.0, 1.0);
            }
            for spec in transition_kind.unwrap().parameter_specs() {
                if let Some(NumberValue::Expr { expr }) = transition_params.get(spec.name) {
                    self.diagnose_parameter_range(
                        *expr,
                        &format!("Transition {}", spec.name),
                        "",
                        f64::from(spec.min),
                        f64::from(spec.max),
                    );
                }
            }
            self.extra_capabilities
                .insert(valle_motion::TRANSITION_CAPABILITY.into());
        }
        if kind_tag == "shutter" {
            if shutter_samples.is_none()
                || shutter_angle.is_none()
                || children.is_empty()
                || children
                    .iter()
                    .any(|child| matches!(child.kind, NodeKind::GlassField(_)))
                || !styles.is_empty()
                || !class_names.is_empty()
                || !class_conditions.is_empty()
                || visibility.is_some()
            {
                self.illegal(DiagCode::GrammarForbidden, element.span(),
                    "Shutter requires static samples and angle plus child layout boxes; put styles and visibility on a surrounding View");
                return None;
            }
            self.extra_capabilities
                .insert(valle_motion::SHUTTER_CAPABILITY.into());
        }
        if kind_tag == "echo" {
            if echo_count.is_none()
                || echo_interval.is_none()
                || echo_decay.is_none()
                || children.is_empty()
                || children
                    .iter()
                    .any(|child| matches!(child.kind, NodeKind::GlassField(_)))
                || !styles.is_empty()
                || !class_names.is_empty()
                || !class_conditions.is_empty()
                || visibility.is_some()
            {
                self.illegal(DiagCode::GrammarForbidden, element.span(),
                    "Echo requires static count, interval and decay plus child layout boxes; put styles and visibility on a surrounding View");
                return None;
            }
            self.extra_capabilities
                .insert(valle_motion::ECHO_CAPABILITY.into());
        }
        if kind_tag == "time-scope" {
            if time_offset.is_none()
                || time_speed.is_none()
                || children.is_empty()
                || children
                    .iter()
                    .any(|child| matches!(child.kind, NodeKind::GlassField(_)))
                || !styles.is_empty()
                || !class_names.is_empty()
                || !class_conditions.is_empty()
                || visibility.is_some()
            {
                self.illegal(DiagCode::GrammarForbidden, element.span(),
                    "TimeScope requires static offset and speed plus child layout boxes; put styles and visibility on a surrounding View");
                return None;
            }
            self.extra_capabilities
                .insert(valle_motion::TIME_SCOPE_CAPABILITY.into());
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
            if let Some((_, source)) = source_children.first() {
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
                    mode: if mask_invert {
                        mask_mode.inverted()
                    } else {
                        mask_mode
                    },
                    rect: mask_rect.expect("checked above"),
                },
                "transition" => NodeKind::Transition {
                    effect: transition_kind.expect("checked above"),
                    progress: transition_progress.expect("checked above"),
                    params: transition_params,
                },
                "shutter" => NodeKind::Shutter {
                    samples: shutter_samples.expect("checked above"),
                    angle_degrees: shutter_angle.expect("checked above"),
                },
                "echo" => NodeKind::Echo {
                    count: echo_count.expect("checked above"),
                    interval_frames: echo_interval.expect("checked above"),
                    decay: echo_decay.expect("checked above"),
                },
                "time-scope" => NodeKind::TimeScope {
                    offset_seconds: time_offset.expect("checked above"),
                    speed: time_speed.expect("checked above"),
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
            class_conditions,
            styles,
            visibility,
            semantic: None,
            children,
            is_mask_source: kind_tag == "mask-source",
        })
    }
}
