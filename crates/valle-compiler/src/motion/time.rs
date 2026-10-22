//! `useCurrentTime()` and `useComposition()`: the clock of the sample being drawn, and the values
//! that hold for the whole frame. Components take only `props`, as in React, and read the time
//! through these hooks. A name bound to either object keeps it; each number lowers where it is
//! read, on the scene clock of the component that called the hook.

use super::*;

/// The hook an object of the time comes from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum TimeHook {
    /// `useCurrentTime()`, called by a component drawn on this scene clock.
    Current(ClockId),
    Composition,
    /// The text unit a `perUnit` callback receives.
    Unit,
}

/// An object a time hook returns, or a path under it.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct TimePath {
    pub(super) hook: TimeHook,
    pub(super) path: Vec<String>,
}

impl TimePath {
    /// Whether this is the object itself rather than one of its numbers.
    pub(super) fn is_object(&self) -> bool {
        self.path.is_empty()
    }

    fn hook_name(&self) -> &'static str {
        match self.hook {
            TimeHook::Current(_) => "useCurrentTime()",
            TimeHook::Composition => "useComposition()",
            TimeHook::Unit => "unit",
        }
    }

    fn has_field(&self, field: &str) -> bool {
        match self.hook {
            TimeHook::Current(_) => {
                matches!(
                    field,
                    "seconds" | "progress" | "clipSeconds" | "clipProgress"
                )
            }
            TimeHook::Composition => {
                matches!(
                    field,
                    "width" | "height" | "duration" | "fps" | "clipDuration"
                )
            }
            TimeHook::Unit => matches!(field, "index" | "count" | "start" | "end"),
        }
    }

    fn fields(&self) -> &'static str {
        match self.hook {
            TimeHook::Current(_) => "`seconds`, `progress`, `clipSeconds` and `clipProgress`",
            TimeHook::Composition => "`width`, `height`, `duration`, `fps` and `clipDuration`",
            TimeHook::Unit => "`index`, `count`, `start` and `end`",
        }
    }
}

/// How an old field of `useTime()` is written now.
fn old_field_advice(field: &str) -> Option<&'static str> {
    Some(match field {
        "localFrame" => {
            "write the animation in seconds; to take items at a fixed rate, such as an image \
             sequence, write `list[clamp(Math.floor(seconds * rate), 0, list.length - 1)]`, with \
             `rate` the items per second"
        }
        "durationFrames" => {
            "the frames of a standalone render are `Math.round(duration * fps)`, from \
             `useComposition()`; an element count cannot depend on it"
        }
        "host" => {
            "the clip clock is `clipSeconds` and `clipProgress` in `useCurrentTime()`, and \
                   `clipDuration` in `useComposition()`"
        }
        "viewport" => "the layout size is `width` and `height` in `useComposition()`",
        "unit" => "per-unit styles take the unit as a parameter: `perUnit={(unit) => ({ … })}`",
        "instance" => "the item index is the second parameter of `.map((item, index) => …)`",
        _ => return None,
    })
}

impl<'s> Compiler<'s> {
    /// The hook `call` calls, when it is `useCurrentTime()` or `useComposition()`.
    pub(super) fn time_hook(&self, call: &oxc::ast::ast::CallExpression<'_>) -> Option<TimeHook> {
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if !call.arguments.is_empty() {
            return None;
        }
        match callee.name.as_str() {
            "useCurrentTime" => Some(TimeHook::Current(self.component_clock)),
            "useComposition" => Some(TimeHook::Composition),
            _ => None,
        }
    }

    /// The path under a time hook that `expression` reads: a hook call, a name or component prop
    /// bound to its object, or a static member chain from any of them.
    pub(super) fn time_path(&self, expression: &Expression<'_>) -> Option<TimePath> {
        let mut segments = Vec::new();
        let mut current = peel_expr(expression);
        let mut time = loop {
            match current {
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.to_string());
                    current = peel_expr(&member.object);
                }
                Expression::Identifier(identifier) => {
                    if let Some(time) = self.bindings.times.get(identifier.name.as_str()) {
                        break time.clone();
                    }
                    if identifier.name != "props" {
                        return None;
                    }
                    let name = segments.pop()?;
                    match self.bindings.component_props.as_ref()?.get(&name)? {
                        AuthorValue::Time(time) => break time.clone(),
                        _ => return None,
                    }
                }
                Expression::CallExpression(call) => {
                    break TimePath {
                        hook: self.time_hook(call)?,
                        path: Vec::new(),
                    };
                }
                _ => return None,
            }
        };
        time.path.extend(segments.into_iter().rev());
        Some(time)
    }

    /// A number of `useComposition()` known while the scene compiles: the duration, and outside
    /// a template the size. A template takes its size from the place it is drawn.
    pub(super) fn static_time_value(&self, time: &TimePath) -> Option<f64> {
        let (TimeHook::Composition, [field]) = (time.hook, time.path.as_slice()) else {
            return None;
        };
        let composition = self.composition.as_ref()?;
        let clip = matches!(self.role, valle_timeline::MotionRole::Clip);
        match field.as_str() {
            "duration" => composition
                .duration()
                .ok()
                .map(|duration| duration.as_f64()),
            "width" if clip => Some(f64::from(composition.width)),
            "height" if clip => Some(f64::from(composition.height)),
            _ => None,
        }
    }

    /// Lower one number of a time hook, at the place it is read.
    pub(super) fn lower_time_path(&mut self, time: &TimePath, span: Span) -> Option<ExprId> {
        let hook = time.hook_name();
        let Some(field) = time.path.first().map(String::as_str) else {
            self.illegal(
                DiagCode::UnknownContextPath,
                span,
                format!(
                    "`{hook}` holds several numbers; read one of them: {}",
                    time.fields()
                ),
            );
            return None;
        };
        if !time.has_field(field) {
            let mut message = format!("`{field}` is not in `{hook}`, which has {}", time.fields());
            if let Some(advice) = old_field_advice(field) {
                message.push_str("; ");
                message.push_str(advice);
            }
            self.illegal(DiagCode::UnknownContextPath, span, message);
            return None;
        }
        if time.path.len() > 1 {
            self.illegal(
                DiagCode::UnknownContextPath,
                span,
                format!("`{field}` of `{hook}` is a number and has no fields"),
            );
            return None;
        }
        if let Some(value) = self.static_time_value(time) {
            return Some(self.push(
                Expr::Const {
                    value: MotionValue::Number(value),
                },
                span,
            ));
        }
        let input = match (time.hook, field) {
            (TimeHook::Current(clock), "seconds") => {
                return Some(self.scene_clock_expr(ContextInput::CompositionSeconds, clock, span));
            }
            (TimeHook::Current(clock), "progress") => {
                return Some(self.scene_clock_expr(ContextInput::LocalProgress, clock, span));
            }
            (TimeHook::Current(_), "clipSeconds") => ContextInput::HostSeconds,
            (TimeHook::Current(_), "clipProgress") => ContextInput::HostProgress,
            (TimeHook::Composition, "duration") => {
                self.illegal(
                    DiagCode::UnknownContextPath,
                    span,
                    "`duration` is the duration of the delivery contract; declare it with \
                     `export const composition = { width, height, fps, duration }`",
                );
                return None;
            }
            (TimeHook::Composition, "fps") => {
                self.warn(
                    DiagCode::FrameRateDependent,
                    span,
                    "`fps` is the output frame rate, which `--fps` can change, and a value \
                     computed from it changes with it; write the animation in seconds",
                );
                let numerator = self.push(
                    Expr::Context {
                        input: ContextInput::FpsNum,
                    },
                    span,
                );
                let denominator = self.push(
                    Expr::Context {
                        input: ContextInput::FpsDen,
                    },
                    span,
                );
                return Some(self.push(
                    Expr::Div {
                        lhs: numerator,
                        rhs: denominator,
                    },
                    span,
                ));
            }
            (TimeHook::Composition, "width") => ContextInput::ViewportWidth,
            (TimeHook::Composition, "height") => ContextInput::ViewportHeight,
            (TimeHook::Composition, "clipDuration") => ContextInput::HostDuration,
            (TimeHook::Unit, "index") => ContextInput::UnitIndex,
            (TimeHook::Unit, "count") => ContextInput::UnitCount,
            (TimeHook::Unit, "start") => ContextInput::UnitStart,
            (TimeHook::Unit, "end") => ContextInput::UnitEnd,
            _ => unreachable!("the field is checked above"),
        };
        // Declare the viewport capability only when used, so consumers without it reject the
        // artifact.
        if matches!(
            input,
            ContextInput::ViewportWidth | ContextInput::ViewportHeight
        ) {
            self.extra_capabilities
                .insert(VIEWPORT_CAPABILITY.to_owned());
        }
        Some(self.push(Expr::Context { input }, span))
    }

    /// Bind `name` to the object of a time hook.
    pub(super) fn bind_time(&mut self, name: String, time: TimePath) {
        self.bindings.unbind(&name);
        self.bindings.times.insert(name, time);
    }

    /// Bind one name to what it reads from a time hook. A number known while the scene compiles
    /// is a constant, which can decide how many elements there are.
    pub(super) fn bind_time_name(&mut self, name: String, time: TimePath, span: Span) {
        if time.is_object() {
            self.bind_time(name, time);
        } else if let Some(value) = self.static_time_value(&time) {
            self.bind_static(name, json_number(value));
        } else if let Some(expr) = self.lower_time_path(&time, span) {
            self.bind_dynamic(name, expr);
        }
    }

    /// Bind what an initializer reads from a time hook: a name keeps an object, and a number is
    /// read once, where it is bound, like any other local.
    pub(super) fn bind_time_value(
        &mut self,
        pattern: &BindingPattern<'_>,
        time: TimePath,
        span: Span,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                self.bind_time_name(identifier.name.to_string(), time, span);
            }
            // Every field of the hooks exists, so a default never applies.
            BindingPattern::AssignmentPattern(assignment) => {
                self.bind_time_value(&assignment.left, time, span);
            }
            BindingPattern::ObjectPattern(object) => {
                if let Some(rest) = &object.rest {
                    self.unsupported(
                        rest.span(),
                        format!(
                            "bind the fields of `{}` that you read; it has no rest",
                            time.hook_name()
                        ),
                    );
                }
                for property in &object.properties {
                    let Some(field) = self.destructured_name(property) else {
                        continue;
                    };
                    let mut field_time = time.clone();
                    field_time.path.push(field);
                    self.bind_time_value(&property.value, field_time, property.span());
                }
            }
            BindingPattern::ArrayPattern(array) => self.unsupported(
                array.span(),
                format!(
                    "`{}` is an object; destructure it by name, such as `{{ {} }}`",
                    time.hook_name(),
                    match time.hook {
                        TimeHook::Current(_) => "seconds, progress",
                        TimeHook::Composition => "width, height",
                        TimeHook::Unit => "index, count",
                    }
                ),
            ),
        }
    }
}

impl<'s> Compiler<'s> {
    /// Reject a template that reads the size its module exports. A template takes its size from
    /// the place it is drawn; the exported size is only the size it renders at alone.
    pub(super) fn check_template_size(&mut self, program: &'s Program<'s>) {
        if matches!(self.role, valle_timeline::MotionRole::Clip) || self.composition.is_none() {
            return;
        }
        if let Some(span) = exported_size_read(program) {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "a template takes its size from the place it is drawn, and the exported \
                 `composition` holds only the size it renders at alone; read the size with \
                 `const { width, height } = useComposition()`",
            );
        }
    }
}

/// The first read of the exported composition's `width` or `height`.
fn exported_size_read<'a>(program: &'a Program<'a>) -> Option<Span> {
    use oxc::ast::ast::{ComputedMemberExpression, StaticMemberExpression, VariableDeclarator};
    use oxc::ast_visit::{Visit, walk};
    use oxc::semantic::{Scoping, SemanticBuilder, SymbolId};

    struct Scan<'b> {
        scoping: &'b Scoping,
        composition: SymbolId,
        found: Option<Span>,
    }
    impl Scan<'_> {
        fn is_composition(&self, expression: &Expression<'_>) -> bool {
            let Expression::Identifier(identifier) = peel_expr(expression) else {
                return false;
            };
            identifier
                .reference_id
                .get()
                .and_then(|reference| self.scoping.get_reference(reference).symbol_id())
                == Some(self.composition)
        }
    }
    fn is_size(name: &str) -> bool {
        matches!(name, "width" | "height")
    }
    impl<'a> Visit<'a> for Scan<'_> {
        fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
            if is_size(it.property.name.as_str()) && self.is_composition(&it.object) {
                self.found.get_or_insert(it.span);
            }
            walk::walk_static_member_expression(self, it);
        }
        fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
            if let Expression::StringLiteral(name) = &it.expression
                && is_size(name.value.as_str())
                && self.is_composition(&it.object)
            {
                self.found.get_or_insert(it.span);
            }
            walk::walk_computed_member_expression(self, it);
        }
        fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
            if let (BindingPattern::ObjectPattern(pattern), Some(initializer)) = (&it.id, &it.init)
                && self.is_composition(initializer)
                && pattern.properties.iter().any(|property| {
                    static_property_name(&property.key).is_some_and(|name| is_size(&name))
                })
            {
                self.found.get_or_insert(it.span);
            }
            walk::walk_variable_declarator(self, it);
        }
    }

    let semantic = SemanticBuilder::new()
        .with_build_nodes(false)
        .build(program)
        .semantic;
    let scoping = semantic.scoping();
    let composition = scoping.get_root_binding("composition".into())?;
    let mut scan = Scan {
        scoping,
        composition,
        found: None,
    };
    scan.visit_program(program);
    scan.found
}

/// A JSON number for a finite value bound as a constant.
fn json_number(value: f64) -> serde_json::Value {
    serde_json::Number::from_f64(value).map_or(serde_json::Value::Null, serde_json::Value::Number)
}
