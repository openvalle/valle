//! Point/Rect/Path geometry, project3d, connectors, and camera bindings.

use super::*;
use valle_motion::geometry::MorphMethod;

struct MorphOptions {
    method: &'static str,
    anchors: Vec<Vec<Point>>,
    pairs: Option<Vec<Vec<Option<usize>>>>,
    allow_self_intersection: bool,
    contact_is_error: bool,
}

impl MorphOptions {
    fn method_kind(&self) -> MorphMethod {
        match self.method {
            "convex" => MorphMethod::Convex,
            "polar" => MorphMethod::Polar,
            "arcLength" => MorphMethod::ArcLength,
            _ => MorphMethod::Auto,
        }
    }
}

impl<'s> Compiler<'s> {
    pub(super) fn lower_point(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "point(x, y) requires exactly two arguments",
            );
            return None;
        }
        let x = self.lower_expr(arguments[0].as_expression()?)?;
        let y = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(Expr::MakePoint { x, y }, span))
    }

    pub(super) fn lower_rect(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 4 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "rect(x, y, width, height) requires exactly four arguments",
            );
            return None;
        }
        let x = self.lower_expr(arguments[0].as_expression()?)?;
        let y = self.lower_expr(arguments[1].as_expression()?)?;
        let width = self.lower_expr(arguments[2].as_expression()?)?;
        let height = self.lower_expr(arguments[3].as_expression()?)?;
        Some(self.push(
            Expr::MakeRect {
                x,
                y,
                width,
                height,
            },
            span,
        ))
    }

    pub(super) fn lower_line(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "line(points) requires one fixed-length Point array",
            );
            return None;
        }
        let points = self.collect_array_elements(arguments[0].as_expression()?, "line(points)")?;
        Some(self.push(Expr::PathLine { points }, span))
    }

    /// `pathTemplate` tagged template with fixed absolute SVG topology.
    ///
    /// v1 deliberately admits only M/L/Q/C/Z. Every interpolation occupies one numeric coordinate
    /// slot, so command topology is visible at compile time while coordinates may still be frame
    /// expressions. Relative commands, arcs and shorthand commands stay rejected instead of being
    /// reparsed differently by Native and Web.
    pub(super) fn lower_path_template(
        &mut self,
        tagged: &oxc::ast::ast::TaggedTemplateExpression<'_>,
    ) -> Option<ExprId> {
        let Expression::Identifier(tag) = &tagged.tag else {
            self.illegal(
                DiagCode::GrammarForbidden,
                tagged.tag.span(),
                "only the pathTemplate tagged template is admitted",
            );
            return None;
        };
        if tag.name != "pathTemplate" || tagged.type_arguments.is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                tagged.span,
                "only pathTemplate`M ... L ... Q ... C ... Z` is admitted",
            );
            return None;
        }

        #[derive(Clone, Copy)]
        enum Token {
            Command(char),
            Number(f64),
            Hole(usize),
        }

        let mut tokens = Vec::new();
        for (quasi_index, quasi) in tagged.quasi.quasis.iter().enumerate() {
            let Some(cooked) = quasi.value.cooked else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    quasi.span,
                    "pathTemplate contains an invalid escape",
                );
                return None;
            };
            let source = cooked.as_bytes();
            let mut at = 0;
            while at < source.len() {
                let byte = source[at];
                if byte.is_ascii_whitespace() || byte == b',' {
                    at += 1;
                    continue;
                }
                if byte.is_ascii_alphabetic() {
                    let command = byte as char;
                    if !matches!(command, 'M' | 'L' | 'Q' | 'C' | 'Z') {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            quasi.span,
                            format!(
                                "pathTemplate command `{command}` is not in the fixed M/L/Q/C/Z subset"
                            ),
                        );
                        return None;
                    }
                    tokens.push(Token::Command(command));
                    at += 1;
                    continue;
                }

                let start = at;
                if matches!(source[at], b'+' | b'-') {
                    at += 1;
                }
                let mut digits = 0usize;
                while at < source.len() && source[at].is_ascii_digit() {
                    digits += 1;
                    at += 1;
                }
                if at < source.len() && source[at] == b'.' {
                    at += 1;
                    while at < source.len() && source[at].is_ascii_digit() {
                        digits += 1;
                        at += 1;
                    }
                }
                if digits == 0 {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        quasi.span,
                        "pathTemplate expects commands, finite numbers, or numeric interpolations",
                    );
                    return None;
                }
                if at < source.len() && matches!(source[at], b'e' | b'E') {
                    at += 1;
                    if at < source.len() && matches!(source[at], b'+' | b'-') {
                        at += 1;
                    }
                    let exponent_start = at;
                    while at < source.len() && source[at].is_ascii_digit() {
                        at += 1;
                    }
                    if exponent_start == at {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            quasi.span,
                            "pathTemplate exponent must contain digits",
                        );
                        return None;
                    }
                }
                let number = core::str::from_utf8(&source[start..at])
                    .ok()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite());
                let Some(number) = number else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        quasi.span,
                        "pathTemplate coordinates must be finite numbers",
                    );
                    return None;
                };
                tokens.push(Token::Number(number));
            }
            if quasi_index < tagged.quasi.expressions.len() {
                tokens.push(Token::Hole(quasi_index));
            }
        }

        let mut verbs = Vec::new();
        let mut points = Vec::new();
        let mut command = None;
        let mut coordinates = Vec::new();
        let flush = |this: &mut Self,
                     command: char,
                     coordinates: &mut Vec<ExprId>,
                     verbs: &mut Vec<valle_draw::PathVerb>,
                     points: &mut Vec<ExprId>|
         -> Option<()> {
            let arity = match command {
                'M' | 'L' => 2,
                'Q' => 4,
                'C' => 6,
                _ => return None,
            };
            if coordinates.is_empty() || coordinates.len() % arity != 0 {
                return None;
            }
            for (group_index, group) in coordinates.chunks_exact(arity).enumerate() {
                let verb = match command {
                    'M' if group_index == 0 => valle_draw::PathVerb::Move,
                    'M' | 'L' => valle_draw::PathVerb::Line,
                    'Q' => valle_draw::PathVerb::Quad,
                    'C' => valle_draw::PathVerb::Cubic,
                    _ => unreachable!(),
                };
                verbs.push(verb);
                for pair in group.chunks_exact(2) {
                    points.push(this.push(
                        Expr::MakePoint {
                            x: pair[0],
                            y: pair[1],
                        },
                        tagged.span,
                    ));
                }
            }
            coordinates.clear();
            Some(())
        };

        for token in tokens {
            match token {
                Token::Command(next) => {
                    if let Some(active) = command.take()
                        && flush(self, active, &mut coordinates, &mut verbs, &mut points).is_none()
                    {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            tagged.span,
                            format!(
                                "pathTemplate command `{active}` has the wrong coordinate count"
                            ),
                        );
                        return None;
                    }
                    if next == 'Z' {
                        verbs.push(valle_draw::PathVerb::Close);
                    } else {
                        command = Some(next);
                    }
                }
                Token::Number(value) => coordinates.push(self.push(
                    Expr::Const {
                        value: MotionValue::Number(value),
                    },
                    tagged.span,
                )),
                Token::Hole(index) => {
                    coordinates.push(self.lower_expr(&tagged.quasi.expressions[index])?)
                }
            }
        }
        if let Some(active) = command
            && flush(self, active, &mut coordinates, &mut verbs, &mut points).is_none()
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                tagged.span,
                format!("pathTemplate command `{active}` has the wrong coordinate count"),
            );
            return None;
        }
        if !coordinates.is_empty() || verbs.is_empty() {
            self.illegal(
                DiagCode::GrammarForbidden,
                tagged.span,
                "pathTemplate must contain a non-empty valid M/L/Q/C/Z path",
            );
            return None;
        }
        if PathData::new(
            verbs.clone(),
            vec![valle_draw::Point::default(); points.len()],
        )
        .is_err()
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                tagged.span,
                "pathTemplate topology is invalid (start with M and close only an open subpath)",
            );
            return None;
        }
        Some(self.push(Expr::PathTemplate { verbs, points }, tagged.span))
    }

    /// Lower bounds(key) with a static string key so the post-layout dependency graph is fixed at
    /// compile time.
    pub(super) fn lower_bounds(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let key = self.static_key_argument(arguments, span, "bounds(\"key\")")?;
        Some(self.push(Expr::NodeBounds { key }, span))
    }

    /// Project a Scene3D anchor to a post-layout world-space point. Static scene and anchor names
    /// keep validation and dependencies fixed across frames.
    pub(super) fn lower_project3d(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "project3d(\"sceneKey\", \"objectKey::anchorKey\") requires two static keys",
            );
            return None;
        }
        let scene_key =
            self.static_key_argument(&arguments[..1], span, "project3d(\"sceneKey\", …)")?;
        let anchor_key = self.static_key_argument(
            &arguments[1..],
            span,
            "project3d(…, \"objectKey::anchorKey\")",
        )?;
        let mut segments = anchor_key.split("::");
        let valid = matches!(
            (segments.next(), segments.next(), segments.next()),
            (Some(parent), Some(anchor), None) if !parent.is_empty() && !anchor.is_empty()
        );
        if !valid {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "project3d anchor address must be exactly \"objectKey::anchorKey\"",
            );
            return None;
        }
        Some(self.push(
            Expr::Project3D {
                scene_key,
                anchor_key,
            },
            span,
        ))
    }

    /// Lower anchor(key, side) into bounds arithmetic for the nine supported box positions.
    pub(super) fn lower_anchor(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "anchor(\"key\", \"side\") requires a node key and a side",
            );
            return None;
        }
        let key = self.static_key_argument(&arguments[..1], span, "anchor(\"key\", …)")?;
        let Some(serde_json::Value::String(side)) = self.eval_static(arguments[1].as_expression()?)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "anchor side must be a static string",
            );
            return None;
        };
        let (fx, fy) = match side.as_str() {
            "left" => (0.0, 0.5),
            "right" => (1.0, 0.5),
            "top" => (0.5, 0.0),
            "bottom" => (0.5, 1.0),
            "center" => (0.5, 0.5),
            "top-left" => (0.0, 0.0),
            "top-right" => (1.0, 0.0),
            "bottom-left" => (0.0, 1.0),
            "bottom-right" => (1.0, 1.0),
            other => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "unknown anchor side `{other}`; use left/right/top/bottom/center or a corner"
                    ),
                );
                return None;
            }
        };
        let rect = self.push(Expr::NodeBounds { key }, span);
        let axis = |compiler: &mut Self, origin: GeometryField, size: GeometryField, f: f64| {
            let origin = compiler.push(
                Expr::GeometryField {
                    input: rect,
                    field: origin,
                },
                span,
            );
            if f == 0.0 {
                return origin;
            }
            let size = compiler.push(
                Expr::GeometryField {
                    input: rect,
                    field: size,
                },
                span,
            );
            let scaled = if f == 1.0 {
                size
            } else {
                let factor = compiler.push(
                    Expr::Const {
                        value: MotionValue::Number(f),
                    },
                    span,
                );
                compiler.push(
                    Expr::Mul {
                        lhs: size,
                        rhs: factor,
                    },
                    span,
                )
            };
            compiler.push(
                Expr::Add {
                    lhs: origin,
                    rhs: scaled,
                },
                span,
            )
        };
        let x = axis(self, GeometryField::X, GeometryField::Width, fx);
        let y = axis(self, GeometryField::Y, GeometryField::Height, fy);
        Some(self.push(Expr::MakePoint { x, y }, span))
    }

    /// Lower point connections into line or cubic path primitives with arithmetic control points.
    pub(super) fn lower_connect(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.is_empty() || arguments.len() > 3 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "connect(from, to, options?) requires two Points and an optional route",
            );
            return None;
        }
        let from = self.lower_expr(arguments[0].as_expression()?)?;
        let to = self.lower_expr(arguments[1].as_expression()?)?;
        let route = match arguments.get(2) {
            None => "line".to_owned(),
            Some(argument) => match self.eval_static(argument.as_expression()?) {
                Some(serde_json::Value::Object(options)) => match options.get("route") {
                    None => "line".to_owned(),
                    Some(serde_json::Value::String(route)) => route.clone(),
                    Some(_) => {
                        self.illegal(
                            DiagCode::GrammarForbidden,
                            span,
                            "connect route must be a static string",
                        );
                        return None;
                    }
                },
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        "connect options must be a static object literal",
                    );
                    return None;
                }
            },
        };
        match route.as_str() {
            "line" => Some(self.push(
                Expr::PathLine {
                    points: vec![from, to],
                },
                span,
            )),
            "cubic" => {
                // Place control points at the endpoints' horizontal midpoint with each endpoint's
                // own y coordinate.
                let mid = |compiler: &mut Self, a: ExprId, b: ExprId| {
                    let ax = compiler.push(
                        Expr::GeometryField {
                            input: a,
                            field: GeometryField::X,
                        },
                        span,
                    );
                    let bx = compiler.push(
                        Expr::GeometryField {
                            input: b,
                            field: GeometryField::X,
                        },
                        span,
                    );
                    let sum = compiler.push(Expr::Add { lhs: ax, rhs: bx }, span);
                    let two = compiler.push(
                        Expr::Const {
                            value: MotionValue::Number(2.0),
                        },
                        span,
                    );
                    compiler.push(Expr::Div { lhs: sum, rhs: two }, span)
                };
                let mid_x = mid(self, from, to);
                let from_y = self.push(
                    Expr::GeometryField {
                        input: from,
                        field: GeometryField::Y,
                    },
                    span,
                );
                let to_y = self.push(
                    Expr::GeometryField {
                        input: to,
                        field: GeometryField::Y,
                    },
                    span,
                );
                let control_1 = self.push(
                    Expr::MakePoint {
                        x: mid_x,
                        y: from_y,
                    },
                    span,
                );
                let control_2 = self.push(Expr::MakePoint { x: mid_x, y: to_y }, span);
                Some(self.push(
                    Expr::PathCubic {
                        from,
                        control_1,
                        control_2,
                        to,
                    },
                    span,
                ))
            }
            other => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("unknown connect route `{other}`; use \"line\" or \"cubic\""),
                );
                None
            }
        }
    }

    /// Read the first argument as a static node key.
    pub(super) fn static_key_argument(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        usage: &str,
    ) -> Option<String> {
        let Some(argument) = arguments.first() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{usage} requires a node key"),
            );
            return None;
        };
        match self.eval_static(argument.as_expression()?) {
            Some(serde_json::Value::String(key)) if !key.is_empty() => Some(key),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "{usage} needs a statically known node key — post-layout dependencies \
                         are a compile-time graph and cannot vary per frame"
                    ),
                );
                None
            }
        }
    }

    /// Lower scene camera center, zoom, and rotation as per-frame expressions. Bounds are available
    /// after layout; camera transforms do not feed back into layout.
    pub(super) fn lower_camera(&mut self, value: &'s Option<JSXAttributeValue<'s>>, span: Span) {
        // Keep child expression borrows tied to the JSX attribute lifetime.
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "camera must be an object literal: camera={{ center, zoom, rotation }}",
            );
            return;
        };
        let Some(expression) = container.expression.as_expression() else {
            self.illegal(DiagCode::GrammarForbidden, span, "camera must not be empty");
            return;
        };
        let Expression::ObjectExpression(object) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "camera must be an object literal with center / zoom / rotation",
            );
            return;
        };
        let mut center = None;
        let mut zoom = None;
        let mut rotation = None;
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "camera has no spread form",
                );
                return;
            };
            let Some(name) = property.key.static_name() else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "camera key must be static",
                );
                return;
            };
            match name.as_ref() {
                "center" => center = self.expr_point_value(&property.value, span),
                "zoom" => zoom = self.expr_number_value(&property.value),
                "rotation" => rotation = self.expr_number_value(&property.value),
                other => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!("unknown camera field `{other}`; use center / zoom / rotation"),
                    );
                    return;
                }
            }
        }
        let (Some(center), Some(zoom)) = (center, zoom) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "camera requires at least `center` and `zoom`",
            );
            return;
        };
        self.camera = Some(CameraBinding {
            center,
            zoom,
            rotation: rotation.unwrap_or(NumberValue::Static { value: 0.0 }),
        });
        self.extra_capabilities.insert(CAMERA_CAPABILITY.to_owned());
    }

    pub(super) fn lower_cubic(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 4 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "cubic(from, control1, control2, to) requires four Point arguments",
            );
            return None;
        }
        let points = arguments
            .iter()
            .map(|argument| self.lower_expr(argument.as_expression()?))
            .collect::<Option<Vec<_>>>()?;
        Some(self.push(
            Expr::PathCubic {
                from: points[0],
                control_1: points[1],
                control_2: points[2],
                to: points[3],
            },
            span,
        ))
    }

    pub(super) fn lower_arc(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 4 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "arc(center, radius, startAngle, endAngle) requires four arguments; angles are radians",
            );
            return None;
        }
        let radius_expression = arguments[1].as_expression()?;
        if let Some(radius) = self.fold_to_number(radius_expression)
            && radius <= 0.0
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                radius_expression.span(),
                "arc radius must be finite and greater than zero",
            );
            return None;
        }
        let start_expression = arguments[2].as_expression()?;
        let end_expression = arguments[3].as_expression()?;
        if let (Some(start), Some(end)) = (
            self.fold_to_number(start_expression),
            self.fold_to_number(end_expression),
        ) && (end - start).abs() > core::f64::consts::TAU
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                end_expression.span(),
                "arc sweep must not exceed TAU radians; use deg(value), rad(value), or TAU explicitly",
            );
            return None;
        }
        let center = self.lower_expr(arguments[0].as_expression()?)?;
        let radius = self.lower_expr(radius_expression)?;
        let start_angle = self.lower_expr(start_expression)?;
        let end_angle = self.lower_expr(end_expression)?;
        Some(self.push(
            Expr::PathArc {
                center,
                radius,
                start_angle,
                end_angle,
            },
            span,
        ))
    }

    pub(super) fn lower_area(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "area(pathOrPoints, baseline) requires exactly two arguments",
            );
            return None;
        }
        let input = arguments[0].as_expression()?;
        let path = if array_items(input).is_some() || as_map_call(input).is_some() {
            let points = self.collect_array_elements(input, "area(pathOrPoints)")?;
            self.push(Expr::PathLine { points }, input.span())
        } else {
            self.lower_expr(input)?
        };
        let baseline = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(Expr::PathArea { path, baseline }, span))
    }

    pub(super) fn lower_sector(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        let [argument] = arguments else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "sector({ center, inner?, outer, start, end, cornerRadius? }) requires one options object",
            );
            return None;
        };
        let Some(Expression::ObjectExpression(object)) = argument.as_expression().map(strip_parens)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                argument.span(),
                "sector takes an options object",
            );
            return None;
        };
        let mut center = None;
        let mut inner = None;
        let mut outer = None;
        let mut start = None;
        let mut end = None;
        let mut corner_radius = None;
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "sector options cannot spread",
                );
                return None;
            };
            let Some(name) = static_property_name(&property.key) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    property.span(),
                    "sector option names must be static identifiers",
                );
                return None;
            };
            match name.as_str() {
                "center" => center = self.lower_expr(&property.value),
                "inner" => inner = self.lower_expr(&property.value),
                "outer" => outer = self.lower_expr(&property.value),
                "start" => start = self.lower_expr(&property.value),
                "end" => end = self.lower_expr(&property.value),
                "cornerRadius" => corner_radius = self.lower_expr(&property.value),
                other => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        format!("unknown sector field `{other}`"),
                    );
                    return None;
                }
            }
        }
        let (Some(center), Some(outer), Some(start), Some(end)) = (center, outer, start, end)
        else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "sector requires center, outer, start and end",
            );
            return None;
        };
        let inner = inner.unwrap_or_else(|| {
            self.push(
                Expr::Const {
                    value: MotionValue::Number(0.0),
                },
                span,
            )
        });
        let corner_radius = corner_radius.unwrap_or_else(|| {
            self.push(
                Expr::Const {
                    value: MotionValue::Number(0.0),
                },
                span,
            )
        });
        if let (Some(inner_value), Some(outer_value)) =
            (self.const_number(inner), self.const_number(outer))
            && (inner_value < 0.0 || outer_value < inner_value)
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "sector needs 0 <= inner <= outer",
            );
            return None;
        }
        Some(self.push(
            Expr::PathSector {
                center,
                inner,
                outer,
                start,
                end,
                corner_radius,
            },
            span,
        ))
    }

    fn const_number(&self, id: ExprId) -> Option<f64> {
        match self.expr_arena.values.get(id.0 as usize) {
            Some(Expr::Const {
                value: MotionValue::Number(value),
            }) if value.is_finite() => Some(*value),
            _ => None,
        }
    }

    pub(super) fn lower_area_band(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "areaBand(upper, lower) requires exactly two PathData arguments",
            );
            return None;
        }
        let upper = self.lower_expr(arguments[0].as_expression()?)?;
        let lower = self.lower_expr(arguments[1].as_expression()?)?;
        if let (Some(MotionValue::PathData(upper_path)), Some(MotionValue::PathData(lower_path))) =
            (self.const_path(upper), self.const_path(lower))
            && let Err(error) = PathData::area_band(&upper_path, &lower_path)
        {
            self.illegal(DiagCode::BuiltinRejected, span, error.to_string());
            return None;
        }
        Some(self.push(Expr::PathAreaBand { upper, lower }, span))
    }

    fn const_path(&self, id: ExprId) -> Option<MotionValue> {
        match self.expr_arena.values.get(id.0 as usize) {
            Some(Expr::Const { value }) => Some(value.clone()),
            _ => None,
        }
    }

    pub(super) fn lower_offset_path(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "offsetPath(path, distance) requires exactly two arguments",
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        let distance = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(Expr::PathOffset { path, distance }, span))
    }

    pub(super) fn lower_resample_path(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "resamplePath(path, count) requires exactly two arguments",
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        let count_expr = self.lower_expr(arguments[1].as_expression()?)?;
        let Some(count) = self.const_number(count_expr) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "resamplePath count must be a static integer",
            );
            return None;
        };
        if count.fract() != 0.0
            || !(2.0..=valle_motion::geometry::MAX_FRAME_GEOMETRY_POINTS as f64).contains(&count)
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "resamplePath count must be an integer from 2 to the per-frame geometry point budget",
            );
            return None;
        }
        Some(self.push(
            Expr::PathResample {
                path,
                count: count as u16,
            },
            span,
        ))
    }

    pub(super) fn lower_reverse_path(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "reversePath(path) requires exactly one argument",
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        Some(self.push(Expr::PathReverse { path }, span))
    }

    pub(super) fn lower_round_corners(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "roundCorners(path, radius) requires two arguments",
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        let radius = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(Expr::PathRoundCorners { path, radius }, span))
    }

    pub(super) fn lower_zigzag(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "zigzag(path, { size, ridges }) requires two arguments",
            );
            return None;
        }
        let options_expr = arguments[1].as_expression()?;
        let Expression::ObjectExpression(object) = strip_parens(options_expr) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "zigzag options must be an object literal",
            );
            return None;
        };
        let mut size = None;
        let mut ridges = None;
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "zigzag does not accept spread options",
                );
                return None;
            };
            let Some(name) = property.key.static_name() else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "zigzag option names must be static",
                );
                return None;
            };
            match name.as_ref() {
                "size" => {
                    if self
                        .fold_to_number(&property.value)
                        .is_some_and(|value| value < 0.0)
                    {
                        self.illegal(
                            DiagCode::BuiltinRejected,
                            property.value.span(),
                            "zigzag size must be non-negative",
                        );
                        return None;
                    }
                    size = self.lower_expr(&property.value);
                }
                "ridges" => ridges = self.fold_to_number(&property.value),
                other => {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        format!("unknown zigzag option `{other}`"),
                    );
                    return None;
                }
            }
        }
        let Some(size) = size else {
            self.illegal(DiagCode::BuiltinRejected, span, "zigzag requires size");
            return None;
        };
        let Some(ridges) =
            ridges.filter(|value| value.fract() == 0.0 && (1.0..=1023.0).contains(value))
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "zigzag ridges must be a static integer from 1 to 1023",
            );
            return None;
        };
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        Some(self.push(
            Expr::PathZigzag {
                path,
                size,
                ridges: ridges as u16,
            },
            span,
        ))
    }

    pub(super) fn lower_noise_displace(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "noiseDisplace(path, { seed, amount, frequency, phase? }) requires two arguments",
            );
            return None;
        }
        let options_expr = arguments[1].as_expression()?;
        let Expression::ObjectExpression(object) = strip_parens(options_expr) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "noiseDisplace options must be an object literal",
            );
            return None;
        };
        let mut seed = None;
        let mut amount = None;
        let mut frequency = None;
        let mut phase = None;
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "noiseDisplace does not accept spread options",
                );
                return None;
            };
            let Some(name) = property.key.static_name() else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "noiseDisplace option names must be static",
                );
                return None;
            };
            match name.as_ref() {
                "seed" => seed = self.fold_to_number(&property.value),
                "amount" => {
                    if self
                        .fold_to_number(&property.value)
                        .is_some_and(|value| value < 0.0)
                    {
                        self.illegal(
                            DiagCode::BuiltinRejected,
                            property.value.span(),
                            "noiseDisplace amount must be non-negative",
                        );
                        return None;
                    }
                    amount = self.lower_expr(&property.value);
                }
                "frequency" => {
                    if self
                        .fold_to_number(&property.value)
                        .is_some_and(|value| value <= 0.0)
                    {
                        self.illegal(
                            DiagCode::BuiltinRejected,
                            property.value.span(),
                            "noiseDisplace frequency must be positive",
                        );
                        return None;
                    }
                    frequency = self.lower_expr(&property.value);
                }
                "phase" => phase = self.lower_expr(&property.value),
                other => {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        format!("unknown noiseDisplace option `{other}`"),
                    );
                    return None;
                }
            }
        }
        let Some(seed) = seed.filter(|value| {
            value.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(value)
        }) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "noiseDisplace seed must be a static non-negative safe integer",
            );
            return None;
        };
        let (Some(amount), Some(frequency)) = (amount, frequency) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "noiseDisplace requires amount and frequency",
            );
            return None;
        };
        let phase = phase.unwrap_or_else(|| {
            self.push(
                Expr::Const {
                    value: MotionValue::Number(0.0),
                },
                span,
            )
        });
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        Some(self.push(
            Expr::PathNoiseDisplace {
                path,
                seed: seed as u64,
                amount,
                frequency,
                phase,
            },
            span,
        ))
    }

    pub(super) fn lower_numeric_path_modifier(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        name: &str,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!("{name}(path, parameter) requires two arguments"),
            );
            return None;
        }
        let parameter_expr = arguments[1].as_expression()?;
        if let Some(value) = self.fold_to_number(parameter_expr) {
            let valid = match name {
                "puckerBloat" => (-1.0..=1.0).contains(&value),
                "twist" => true,
                "simplify" | "strokeToPath" => value >= 0.0,
                _ => false,
            };
            if !valid {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    parameter_expr.span(),
                    format!("{name} parameter is outside its admitted range"),
                );
                return None;
            }
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        let parameter = self.lower_expr(parameter_expr)?;
        let expr = match name {
            "puckerBloat" => Expr::PathPuckerBloat {
                path,
                amount: parameter,
            },
            "twist" => Expr::PathTwist {
                path,
                angle: parameter,
            },
            "simplify" => Expr::PathSimplify {
                path,
                tolerance: parameter,
            },
            "strokeToPath" => Expr::PathStrokeToPath {
                path,
                width: parameter,
            },
            _ => unreachable!(),
        };
        Some(self.push(expr, span))
    }

    pub(super) fn lower_morph_path(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 3 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "morphPath(from, to, progress) requires exactly three arguments",
            );
            return None;
        }
        let from = self.lower_expr(arguments[0].as_expression()?)?;
        let to = self.lower_expr(arguments[1].as_expression()?)?;
        let progress = self.lower_expr(arguments[2].as_expression()?)?;
        Some(self.push(Expr::PathMorph { from, to, progress }, span))
    }

    fn morph_options(
        &mut self,
        argument: Option<&Argument<'_>>,
        path_count: usize,
        span: Span,
    ) -> Option<MorphOptions> {
        let Some(argument) = argument else {
            return Some(MorphOptions {
                method: "auto",
                anchors: Vec::new(),
                pairs: None,
                allow_self_intersection: false,
                contact_is_error: false,
            });
        };
        let expression = argument.as_expression()?;
        let Some(options) = self
            .eval_static(expression)
            .and_then(|value| value.as_object().cloned())
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "morph options must be a prepare-time object",
            );
            return None;
        };
        if options.keys().any(|key| {
            !matches!(
                key.as_str(),
                "method" | "anchors" | "pairs" | "allowSelfIntersection" | "contactPolicy"
            )
        }) {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "morph options accept only `method`, `anchors`, `pairs`, `allowSelfIntersection`, and `contactPolicy`",
            );
            return None;
        }
        let method = match options
            .get("method")
            .map_or(Some("auto"), serde_json::Value::as_str)
        {
            Some("auto") => "auto",
            Some("convex") => "convex",
            Some("polar") => "polar",
            Some("compatible") => "compatible",
            Some("arcLength") => "arcLength",
            _ => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "morph method must be `auto`, `convex`, `polar`, `compatible`, or `arcLength`",
                );
                return None;
            }
        };
        let allow_self_intersection = match options.get("allowSelfIntersection") {
            None => false,
            Some(serde_json::Value::Bool(value)) => *value,
            _ => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "allowSelfIntersection must be a prepare-time boolean",
                );
                return None;
            }
        };
        let contact_is_error = match options.get("contactPolicy") {
            None => false,
            Some(serde_json::Value::String(value)) if value == "warn" => false,
            Some(serde_json::Value::String(value)) if value == "error" => true,
            _ => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "contactPolicy must be `warn` or `error`",
                );
                return None;
            }
        };
        let anchors = match options.get("anchors") {
            None => Vec::new(),
            Some(serde_json::Value::Array(rows)) => {
                let parsed = rows
                    .iter()
                    .map(|row| {
                        let entries = row.as_array()?;
                        if entries.len() != path_count {
                            return None;
                        }
                        entries
                            .iter()
                            .map(|entry| match motion_value_from_json(entry) {
                                Some(MotionValue::Point(point))
                                    if point.x.is_finite() && point.y.is_finite() =>
                                {
                                    Some(point)
                                }
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>()
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(parsed) = parsed.filter(|parsed| parsed.len() <= 32) else {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        "anchors must contain at most 32 rows of one point(...) per path",
                    );
                    return None;
                };
                parsed
            }
            _ => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "anchors must be a prepare-time array of point(...) rows",
                );
                return None;
            }
        };
        let pairs = match options.get("pairs") {
            None => None,
            Some(serde_json::Value::Array(rows)) => {
                let parsed = rows
                    .iter()
                    .map(|row| {
                        let cells = row.as_array()?;
                        if cells.len() != path_count {
                            return None;
                        }
                        cells
                            .iter()
                            .map(|cell| match cell {
                                serde_json::Value::Null => Some(None),
                                serde_json::Value::Number(number) => number
                                    .as_u64()
                                    .and_then(|index| usize::try_from(index).ok())
                                    .map(Some),
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>()
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(parsed) = parsed.filter(|parsed| !parsed.is_empty() && parsed.len() <= 8)
                else {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        "pairs must have 1–8 rows of one zero-based contour index or null per key shape",
                    );
                    return None;
                };
                Some(parsed)
            }
            _ => {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "pairs must be a prepare-time array of contour correspondence rows",
                );
                return None;
            }
        };
        if !anchors.is_empty() && !matches!(method, "auto" | "arcLength") {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "anchors require `auto` or `arcLength` correspondence",
            );
            return None;
        }
        Some(MorphOptions {
            method,
            anchors,
            pairs,
            allow_self_intersection,
            contact_is_error,
        })
    }

    pub(super) fn lower_morph(&mut self, arguments: &[Argument<'_>], span: Span) -> Option<ExprId> {
        if !(3..=4).contains(&arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "morph(from, to, progress, options?) requires three or four arguments",
            );
            return None;
        }
        let options = self.morph_options(arguments.get(3), 2, span)?;
        let from = self.lower_expr(arguments[0].as_expression()?)?;
        let to = self.lower_expr(arguments[1].as_expression()?)?;
        let progress = self.lower_expr(arguments[2].as_expression()?)?;
        let (Some(MotionValue::PathData(from_path)), Some(MotionValue::PathData(to_path))) =
            (self.const_path(from), self.const_path(to))
        else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "morph paths must be prepare-time PathData so their correspondence can be computed once",
            );
            return None;
        };
        if options.method == "compatible" {
            if !options.anchors.is_empty() || options.pairs.is_some() {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "compatible morph currently requires one simple closed contour without anchors or pairs",
                );
                return None;
            }
            let prepared = match from_path.prepare_compatible_barycentric_morph(&to_path) {
                Ok(Some(prepared)) => prepared,
                Ok(None) => {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        "compatible morph could not prepare a fixed-boundary mesh; use simple closed polygons within the mesh budget",
                    );
                    return None;
                }
                Err(reason) => {
                    self.illegal(DiagCode::BuiltinRejected, span, reason.to_string());
                    return None;
                }
            };
            return Some(self.push(Expr::PathCompatibleMorph { prepared, progress }, span));
        }
        let multiple_contours = [&from_path, &to_path].iter().any(|path| {
            path.verbs
                .iter()
                .filter(|verb| **verb == PathVerb::Move)
                .count()
                != 1
        });
        let aligned = if let Some(pairs) = &options.pairs {
            PathData::paired_morph_sequence_with_contacts(
                &[from_path.clone(), to_path.clone()],
                pairs,
                &options.anchors,
                options.method_kind(),
                options.allow_self_intersection,
                options.contact_is_error,
            )
            .map(|(mut paths, contacts)| ((paths.remove(0), paths.remove(0)), contacts))
        } else if multiple_contours {
            PathData::automatic_multi_morph_sequence_with_contacts(
                &[from_path.clone(), to_path.clone()],
                &options.anchors,
                options.method_kind(),
                options.allow_self_intersection,
                options.contact_is_error,
            )
            .map(|(mut paths, contacts)| ((paths.remove(0), paths.remove(0)), contacts))
        } else if !options.anchors.is_empty() {
            from_path.arc_length_morph_pair_with_contacts(
                &to_path,
                &options.anchors,
                options.allow_self_intersection,
                options.contact_is_error,
            )
        } else {
            match options.method {
                "convex" => from_path
                    .convex_morph_pair(&to_path)
                    .map(|pair| (pair, Vec::new())),
                "polar" => from_path
                    .polar_morph_pair(&to_path)
                    .map(|pair| (pair, Vec::new())),
                "arcLength" => from_path.arc_length_morph_pair_with_contacts(
                    &to_path,
                    &[],
                    options.allow_self_intersection,
                    options.contact_is_error,
                ),
                _ => from_path.automatic_morph_pair_with_contacts(
                    &to_path,
                    options.allow_self_intersection,
                    options.contact_is_error,
                ),
            }
        };
        let ((aligned_from, aligned_to), contacts) = match aligned {
            Ok(pair) => pair,
            Err(reason) => {
                if options.method == "auto"
                    && options.pairs.is_none()
                    && options.anchors.is_empty()
                    && !multiple_contours
                    && let Ok(Some(prepared)) =
                        from_path.prepare_compatible_barycentric_morph(&to_path)
                {
                    return Some(self.push(Expr::PathCompatibleMorph { prepared, progress }, span));
                }
                let message = if multiple_contours && options.pairs.is_none() {
                    format!("{reason}; specify `pairs` to control contour matching")
                } else {
                    reason.to_string()
                };
                self.illegal(DiagCode::BuiltinRejected, span, message);
                return None;
            }
        };
        for contact in contacts {
            self.warn(
                DiagCode::MorphContact,
                span,
                format!(
                    "morph touches without crossing between key shapes {} and {}: {}",
                    contact.segment,
                    contact.segment + 1,
                    contact.detail
                ),
            );
        }
        let from = self.push(
            Expr::Const {
                value: MotionValue::PathData(aligned_from),
            },
            span,
        );
        let to = self.push(
            Expr::Const {
                value: MotionValue::PathData(aligned_to),
            },
            span,
        );
        Some(self.push(Expr::PathMorph { from, to, progress }, span))
    }

    pub(super) fn lower_morph_sequence(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if !(3..=4).contains(&arguments.len()) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "morphSequence(paths, stops, progress, options?) requires three or four arguments",
            );
            return None;
        }
        let paths = self
            .eval_static(arguments[0].as_expression()?)
            .and_then(|value| value.as_array().cloned())
            .and_then(|items| {
                items
                    .iter()
                    .map(motion_value_from_json)
                    .map(|value| match value {
                        Some(MotionValue::PathData(path)) => Some(path),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            });
        let stops = self
            .eval_static(arguments[1].as_expression()?)
            .and_then(|value| value.as_array().cloned())
            .and_then(|items| {
                items
                    .iter()
                    .map(serde_json::Value::as_f64)
                    .collect::<Option<Vec<_>>>()
            });
        let (Some(paths), Some(stops)) = (paths, stops) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "morphSequence paths and stops must be prepare-time arrays of PathData and numbers",
            );
            return None;
        };
        if paths.len() < 2
            || paths.len() != stops.len()
            || stops.iter().any(|stop| !stop.is_finite())
            || stops.windows(2).any(|pair| pair[0] >= pair[1])
        {
            self.illegal(
                DiagCode::BuiltinRejected,
                span,
                "morphSequence needs matching path and finite strictly increasing stop arrays of length at least two",
            );
            return None;
        }
        let options = self.morph_options(arguments.get(3), paths.len(), span)?;
        if options.method == "compatible" {
            if !options.anchors.is_empty() || options.pairs.is_some() {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    span,
                    "compatible morph sequence currently requires one simple closed contour per key shape without anchors or pairs",
                );
                return None;
            }
            let segments = match PathData::prepare_compatible_barycentric_sequence(&paths) {
                Ok(Some(segments)) => segments,
                Ok(None) => {
                    self.illegal(
                        DiagCode::BuiltinRejected,
                        span,
                        "compatible morph sequence could not prepare simple closed polygons within the mesh budget",
                    );
                    return None;
                }
                Err(reason) => {
                    self.illegal(DiagCode::BuiltinRejected, span, reason.to_string());
                    return None;
                }
            };
            let progress = self.lower_expr(arguments[2].as_expression()?)?;
            return Some(self.push(
                Expr::PathCompatibleMorphSequence {
                    segments,
                    stops,
                    progress,
                },
                span,
            ));
        }
        let multiple_contours = paths.iter().any(|path| {
            path.verbs
                .iter()
                .filter(|verb| **verb == PathVerb::Move)
                .count()
                != 1
        });
        let alignment = if let Some(pairs) = &options.pairs {
            PathData::paired_morph_sequence_with_contacts(
                &paths,
                pairs,
                &options.anchors,
                options.method_kind(),
                options.allow_self_intersection,
                options.contact_is_error,
            )
        } else if multiple_contours {
            PathData::automatic_multi_morph_sequence_with_contacts(
                &paths,
                &options.anchors,
                options.method_kind(),
                options.allow_self_intersection,
                options.contact_is_error,
            )
        } else if !options.anchors.is_empty() {
            PathData::arc_length_morph_sequence_with_contacts(
                &paths,
                &options.anchors,
                options.allow_self_intersection,
                options.contact_is_error,
            )
        } else {
            match options.method {
                "convex" => {
                    PathData::convex_morph_sequence(&paths).map(|paths| (paths, Vec::new()))
                }
                "polar" => PathData::polar_morph_sequence(&paths).map(|paths| (paths, Vec::new())),
                "arcLength" => PathData::arc_length_morph_sequence_with_contacts(
                    &paths,
                    &[],
                    options.allow_self_intersection,
                    options.contact_is_error,
                ),
                _ => PathData::automatic_morph_sequence_with_contacts(
                    &paths,
                    options.allow_self_intersection,
                    options.contact_is_error,
                ),
            }
        };
        let (aligned, contacts) = match alignment {
            Ok(prepared) => prepared,
            Err(reason) => {
                if options.method == "auto"
                    && options.pairs.is_none()
                    && options.anchors.is_empty()
                    && !multiple_contours
                    && let Ok(Some(segments)) =
                        PathData::prepare_compatible_barycentric_sequence(&paths)
                {
                    let progress = self.lower_expr(arguments[2].as_expression()?)?;
                    return Some(self.push(
                        Expr::PathCompatibleMorphSequence {
                            segments,
                            stops,
                            progress,
                        },
                        span,
                    ));
                }
                let message = if multiple_contours && options.pairs.is_none() {
                    format!("{reason}; specify `pairs` to control contour matching")
                } else {
                    reason.to_string()
                };
                self.illegal(DiagCode::BuiltinRejected, span, message);
                return None;
            }
        };
        for contact in contacts {
            self.warn(
                DiagCode::MorphContact,
                span,
                format!(
                    "morphSequence touches without crossing between key shapes {} and {}: {}",
                    contact.segment,
                    contact.segment + 1,
                    contact.detail
                ),
            );
        }
        let input = self.lower_expr(arguments[2].as_expression()?)?;
        let stop_count = stops.len();
        Some(
            self.push(
                Expr::Interpolate {
                    input,
                    stops: stops
                        .into_iter()
                        .zip(aligned)
                        .map(|(input, path)| InterpolateStop {
                            input,
                            output: MotionValue::PathData(path),
                        })
                        .collect(),
                    easings: vec![MotionEasing::Linear; stop_count - 1],
                    color_space: valle_draw::program::GradientInterpolation::Srgb,
                    extrapolate_left: Extrapolation::Clamp,
                    extrapolate_right: Extrapolation::Clamp,
                },
                span,
            ),
        )
    }

    pub(super) fn lower_path_sample(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
        tangent: bool,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                if tangent {
                    "tangentAt(path, progress) requires exactly two arguments"
                } else {
                    "pointAt(path, progress) requires exactly two arguments"
                },
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        let progress = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(
            if tangent {
                Expr::PathTangentAt { path, progress }
            } else {
                Expr::PathPointAt { path, progress }
            },
            span,
        ))
    }

    pub(super) fn lower_path_length(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 1 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "pathLength(path) requires exactly one argument",
            );
            return None;
        }
        let path = self.lower_expr(arguments[0].as_expression()?)?;
        Some(self.push(Expr::PathLength { path }, span))
    }

    pub(super) fn lower_path_trajectory(
        &mut self,
        arguments: &[Argument<'_>],
        span: Span,
    ) -> Option<ExprId> {
        if arguments.len() != 2 {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "pathTrajectory(frames, frame) requires exactly two arguments",
            );
            return None;
        }
        let frames = self
            .eval_static(arguments[0].as_expression()?)
            .and_then(|value| value.as_array().cloned())
            .and_then(|values| {
                values
                    .iter()
                    .map(motion_value_from_json)
                    .map(|value| match value {
                        Some(MotionValue::PathData(value)) => Some(value),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .or_else(|| {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    arguments[0].span(),
                    "pathTrajectory frames must be a static array of path(svgD) values",
                );
                None
            })?;
        let frame = self.lower_expr(arguments[1].as_expression()?)?;
        Some(self.push(Expr::PathTrajectory { frames, frame }, span))
    }
}
