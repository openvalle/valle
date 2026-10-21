//! `useCaption()`: the caption a captionPresenter draws. A template compiles once for a caption
//! track, so what varies from caption to caption, the text and the runs, is read like the time: a
//! name bound to the caption, its runs or one run keeps the path under it, and every read lowers
//! to a caption input where it is written. `caption.runs.map((run, index) => ...)` lowers its
//! callback once, as a row that repeats for each run of the caption the template is bound to.
//! The track style, the region and the alignment are prepared, like any static value.

use super::*;
use oxc::ast::ast::{ArrowFunctionExpression, CallExpression};
use valle_motion::CaptionInput;

/// The path segment that stands for the run of a `runs.map(...)` row.
const RUN: &str = "#";

/// What prepare-time code that reads the text or the runs is told.
pub(super) const CAPTION_CONTENT: &str = "the caption's text and runs are bound as each caption is drawn, like `useTime()`: read them in JSX and frame expressions, not in prepare-time code; change case with CSS `textTransform`, and style a keyword with its run's color or fontSize in the Timeline";

/// Whether `call` is `useCaption()`.
pub(super) fn is_use_caption(call: &CallExpression<'_>) -> bool {
    matches!(&call.callee, Expression::Identifier(callee) if callee.name == "useCaption")
}

/// Whether `field` of the caption is what the template is prepared with.
fn is_layout(field: &str) -> bool {
    matches!(field, "style" | "region" | "align")
}

/// Whether `path` reads what the template is prepared with rather than what each caption binds.
fn is_prepared(path: &[String]) -> bool {
    path.first().is_some_and(|field| is_layout(field))
}

/// Whether `path` names an object of the caption: the caption, its runs, or a run.
pub(super) fn is_caption_object(path: &[String]) -> bool {
    match path {
        [] => true,
        [runs] => runs == "runs",
        [runs, run] => runs == "runs" && run == RUN,
        _ => false,
    }
}

/// Whether `expression` reads what each caption binds: its text, its runs, or the caption as a
/// whole. Reads of the style, the region and the alignment evaluate while the scene is prepared.
pub(super) fn reads_caption_content(
    expression: &Expression<'_>,
    captions: &BTreeMap<String, Vec<String>>,
) -> bool {
    use oxc::ast_visit::{Visit, walk};

    struct Scan<'c> {
        captions: &'c BTreeMap<String, Vec<String>>,
        scopes: Vec<BTreeSet<String>>,
        found: bool,
    }
    impl Scan<'_> {
        fn bound(&self, name: &str) -> Option<&Vec<String>> {
            if self.scopes.iter().any(|scope| scope.contains(name)) {
                return None;
            }
            self.captions.get(name)
        }
    }
    impl<'a> Visit<'a> for Scan<'_> {
        fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
            self.scopes.push(BTreeSet::new());
            walk::walk_arrow_function_expression(self, it);
            self.scopes.pop();
        }
        fn visit_function(
            &mut self,
            it: &oxc::ast::ast::Function<'a>,
            flags: oxc::semantic::ScopeFlags,
        ) {
            self.scopes.push(BTreeSet::new());
            walk::walk_function(self, it, flags);
            self.scopes.pop();
        }
        fn visit_binding_identifier(&mut self, it: &oxc::ast::ast::BindingIdentifier<'a>) {
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(it.name.to_string());
            }
        }
        fn visit_static_member_expression(
            &mut self,
            it: &oxc::ast::ast::StaticMemberExpression<'a>,
        ) {
            let mut first = it.property.name.as_str();
            let mut object = peel_expr(&it.object);
            while let Expression::StaticMemberExpression(member) = object {
                first = member.property.name.as_str();
                object = peel_expr(&member.object);
            }
            if let Expression::Identifier(identifier) = object
                && let Some(path) = self.bound(identifier.name.as_str())
            {
                let field = path.first().map_or(first, String::as_str);
                self.found |= !is_layout(field);
                return;
            }
            walk::walk_static_member_expression(self, it);
        }
        fn visit_identifier_reference(&mut self, it: &oxc::ast::ast::IdentifierReference<'a>) {
            self.found |= self.bound(it.name.as_str()).is_some();
        }
        fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
            if is_use_caption(it) {
                self.found = true;
                return;
            }
            walk::walk_call_expression(self, it);
        }
    }

    let mut scan = Scan {
        captions,
        scopes: Vec::new(),
        found: false,
    };
    scan.visit_expression(expression);
    scan.found
}

impl<'s> Compiler<'s> {
    fn is_presenter(&self) -> bool {
        matches!(
            self.role,
            valle_timeline::MotionRole::CaptionPresenter { .. }
        )
    }

    /// `useCaption()` belongs to a captionPresenter, and `useData()` to every other template.
    pub(super) fn check_caption_hooks(&mut self, program: &Program<'_>) {
        let (hook, message) = if self.is_presenter() {
            (
                "useData",
                "a captionPresenter has no prepared data; read the caption with `const caption = useCaption()`",
            )
        } else {
            (
                "useCaption",
                "useCaption() reads the caption a captionPresenter draws; declare `export const role = captionPresenter(...)`",
            )
        };
        if let Some(span) = first_call(program, hook) {
            self.illegal(DiagCode::GrammarForbidden, span, message);
        }
    }

    /// Whether `expression` reads what each caption binds, which no prepare-time code can.
    pub(super) fn reads_caption(&self, expression: &Expression<'_>) -> bool {
        self.is_presenter() && reads_caption_content(expression, &self.bindings.captions)
    }

    /// The path under the caption that `expression` reads: `useCaption()`, a name or component
    /// prop bound to the caption, or a static member chain from either.
    pub(super) fn caption_path(&self, expression: &Expression<'_>) -> Option<Vec<String>> {
        let mut segments = Vec::new();
        let mut current = peel_expr(expression);
        let mut path = loop {
            match current {
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.to_string());
                    current = peel_expr(&member.object);
                }
                Expression::Identifier(identifier) => {
                    if let Some(path) = self.bindings.captions.get(identifier.name.as_str()) {
                        break path.clone();
                    }
                    if identifier.name != "props" {
                        return None;
                    }
                    let name = segments.pop()?;
                    match self.bindings.component_props.as_ref()?.get(&name)? {
                        AuthorValue::Caption(path) => break path.clone(),
                        _ => return None,
                    }
                }
                Expression::CallExpression(call) if is_use_caption(call) => break Vec::new(),
                _ => return None,
            }
        };
        path.extend(segments.into_iter().rev());
        Some(path)
    }

    /// The prepared value at `path`: the layout, or the track style, the region or the alignment.
    /// A whole caption, as `--data` gives one, prepares the template with its layout alone.
    fn prepared_caption_value(&self, path: &[String]) -> serde_json::Value {
        let mut value = match self.prepare_data.as_ref().map(|binding| &binding.value) {
            Some(serde_json::Value::Object(object)) => serde_json::Value::Object(
                object
                    .iter()
                    .filter(|(field, _)| is_layout(field))
                    .map(|(field, value)| (field.clone(), value.clone()))
                    .collect(),
            ),
            _ => serde_json::Value::Null,
        };
        for field in path {
            value = match value {
                serde_json::Value::Object(mut object) => {
                    object.remove(field).unwrap_or(serde_json::Value::Null)
                }
                serde_json::Value::Array(mut items) => match field.parse::<usize>() {
                    Ok(index) if index < items.len() => items.swap_remove(index),
                    _ => serde_json::Value::Null,
                },
                _ => serde_json::Value::Null,
            };
        }
        value
    }

    /// Lower one value of the caption, at the place it is read.
    pub(super) fn lower_caption_path(&mut self, path: &[String], span: Span) -> Option<ExprId> {
        if is_prepared(path) {
            let value = self.prepared_caption_value(path);
            let Some(value) = motion_value_from_json(&value) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "`caption.{}` holds several values; read one of them",
                        path.join(".")
                    ),
                );
                return None;
            };
            return Some(self.push(Expr::Const { value }, span));
        }
        let input = match path {
            [text] if text == "text" => CaptionInput::Text,
            [runs, length] if runs == "runs" && length == "length" => CaptionInput::RunCount,
            [runs, run, field] if runs == "runs" && run == RUN => match field.as_str() {
                "text" => CaptionInput::RunText,
                "start" => CaptionInput::RunStart,
                "end" => CaptionInput::RunEnd,
                "fontSize" => CaptionInput::RunFontSize,
                "color" => CaptionInput::RunColor,
                _ => {
                    self.illegal(
                        DiagCode::UnknownContextPath,
                        span,
                        format!("a run has text, start, end, fontSize and color, not `{field}`"),
                    );
                    return None;
                }
            },
            [] => {
                self.illegal(
                    DiagCode::UnknownContextPath,
                    span,
                    "`useCaption()` holds several values; read one, such as `caption.text`",
                );
                return None;
            }
            [runs] if runs == "runs" => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "draw the runs with `caption.runs.map((run, index) => ...)`, or read `caption.runs.length`",
                );
                return None;
            }
            [runs, run] if runs == "runs" && run == RUN => {
                self.illegal(
                    DiagCode::UnknownContextPath,
                    span,
                    "a run holds several values; read one, such as `run.text`",
                );
                return None;
            }
            _ => {
                self.illegal(
                    DiagCode::UnknownContextPath,
                    span,
                    format!(
                        "`caption.{}` is not part of the caption; it has text, runs, style, region and align",
                        path.join(".")
                    ),
                );
                return None;
            }
        };
        Some(self.push(Expr::Caption { input }, span))
    }

    /// Bind `name` to the caption, its runs or one run. The caption keeps its prepared parts as a
    /// static value, so prepare-time code reads the style, the region and the alignment.
    pub(super) fn bind_caption(&mut self, name: String, path: Vec<String>) {
        self.bindings.unbind(&name);
        if path.is_empty() {
            let layout = self.prepared_caption_value(&[]);
            self.bindings.statics.insert(name.clone(), layout);
        }
        self.bindings.captions.insert(name, path);
    }

    /// Bind one name to what it reads from the caption.
    pub(super) fn bind_caption_name(&mut self, name: String, path: Vec<String>, span: Span) {
        if is_prepared(&path) {
            let value = self.prepared_caption_value(&path);
            self.bind_static(name, value);
        } else if is_caption_object(&path) {
            self.bind_caption(name, path);
        } else if let Some(expr) = self.lower_caption_path(&path, span) {
            self.bind_dynamic(name, expr);
        }
    }

    /// Bind what an initializer reads from the caption: a name keeps an object of the caption,
    /// and a value is read once, where it is bound.
    pub(super) fn bind_caption_value(
        &mut self,
        pattern: &BindingPattern<'_>,
        path: Vec<String>,
        span: Span,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                self.bind_caption_name(identifier.name.to_string(), path, span);
            }
            BindingPattern::AssignmentPattern(assignment) => {
                self.bind_caption_value(&assignment.left, path, span);
            }
            BindingPattern::ObjectPattern(object) => {
                if let Some(rest) = &object.rest {
                    self.unsupported(
                        rest.span(),
                        "bind the fields of the caption that you read; it has no rest",
                    );
                }
                for property in &object.properties {
                    let Some(field) = self.destructured_name(property) else {
                        continue;
                    };
                    let mut field_path = path.clone();
                    field_path.push(field);
                    self.bind_caption_value(&property.value, field_path, property.span());
                }
            }
            BindingPattern::ArrayPattern(_) if is_prepared(&path) => {
                let value = self.prepared_caption_value(&path);
                self.bind_static_pattern(pattern, value);
            }
            BindingPattern::ArrayPattern(array) => self.illegal(
                DiagCode::GrammarForbidden,
                array.span(),
                "draw the runs with `caption.runs.map((run, index) => ...)`; they are bound as each caption is drawn and cannot be destructured",
            ),
        }
    }

    /// Whether `call` is `caption.runs.map(...)`.
    pub(super) fn is_caption_map_call(&self, call: &CallExpression<'_>) -> bool {
        matches!(&call.callee, Expression::StaticMemberExpression(member)
            if member.property.name == "map"
                && self
                    .caption_path(&member.object)
                    .is_some_and(|path| matches!(path.as_slice(), [runs] if runs == "runs")))
    }

    /// The `(run, index) => ...` callback of `caption.runs.map(...)`.
    fn caption_map_callback(
        &mut self,
        call: &'s CallExpression<'s>,
    ) -> Option<&'s ArrowFunctionExpression<'s>> {
        let arrow = match &call.arguments[..] {
            [argument] => match argument.as_expression().map(peel_expr) {
                Some(Expression::ArrowFunctionExpression(arrow)) => Some(arrow.as_ref()),
                _ => None,
            },
            _ => None,
        };
        let Some(arrow) = arrow.filter(|arrow| {
            !arrow.r#async && arrow.params.rest.is_none() && arrow.params.items.len() <= 2
        }) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "`caption.runs.map` takes one arrow callback, `(run, index) => JSX`",
            );
            return None;
        };
        if self.caption_row.is_some() {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "a `runs.map(...)` row cannot draw the runs again; each run has one row",
            );
            return None;
        }
        Some(arrow)
    }

    /// Lower `f` as the row of a `runs.map(...)` callback: its run and index are bound, and every
    /// node it lowers is drawn once for each run.
    fn in_caption_row<T>(
        &mut self,
        arrow: &'s ArrowFunctionExpression<'s>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> (u32, T) {
        let saved = self.bindings.snapshot_frame();
        if let Some(parameter) = arrow.params.items.first() {
            self.bind_caption_value(
                &parameter.pattern,
                vec!["runs".into(), RUN.into()],
                parameter.span(),
            );
        }
        if let Some(parameter) = arrow.params.items.get(1) {
            match &parameter.pattern {
                BindingPattern::BindingIdentifier(identifier) => {
                    let index = self.push(
                        Expr::Caption {
                            input: CaptionInput::RunIndex,
                        },
                        parameter.span(),
                    );
                    self.bind_dynamic(identifier.name.to_string(), index);
                }
                other => self.unsupported(
                    other.span(),
                    "name the index of a run, as in `(run, index) => ...`",
                ),
            }
        }
        let row = self.caption_row_count;
        self.caption_row_count += 1;
        self.caption_row = Some(row);
        let result = f(self);
        self.caption_row = None;
        self.bindings.restore_frame(saved);
        (row, result)
    }

    /// `caption.runs.map((run, index) => JSX)` among the children of an element.
    pub(super) fn lower_caption_rows(
        &mut self,
        call: &'s CallExpression<'s>,
        path: &str,
    ) -> Option<Vec<PendingNode>> {
        let arrow = self.caption_map_callback(call)?;
        let (row, nodes) = self.in_caption_row(arrow, |this| {
            this.lower_static_map_callback(arrow, &format!("{path}.run"))
        });
        let mut nodes: Vec<_> = nodes?
            .into_iter()
            .flat_map(PendingNode::into_nodes)
            .collect();
        for node in &mut nodes {
            node.caption_row = Some(row);
        }
        Some(nodes)
    }

    /// The `<Span>` a map callback inside a `<Text>` returns.
    fn span_callback(
        &mut self,
        arrow: &'s ArrowFunctionExpression<'s>,
    ) -> Option<&'s JSXElement<'s>> {
        let element = match arrow.get_expression().map(peel_expr) {
            Some(Expression::JSXElement(element))
                if matches!(&element.opening_element.name,
                    JSXElementName::Identifier(name) if name.name == "Span")
                    || matches!(&element.opening_element.name,
                    JSXElementName::IdentifierReference(name) if name.name == "Span") =>
            {
                Some(element.as_ref())
            }
            _ => None,
        };
        if element.is_none() {
            self.illegal(
                DiagCode::GrammarForbidden,
                arrow.span(),
                "a map inside <Text> returns one <Span> for each item, as in `(run) => <Span>{run.text}</Span>`",
            );
        }
        element
    }

    /// A map among the children of a `<Text>` whose callback returns a `<Span>`: one run for each
    /// item of a prepared array, or a row of runs for `caption.runs`.
    pub(super) fn lower_span_map(
        &mut self,
        call: &'s CallExpression<'s>,
        path: &str,
    ) -> Option<Vec<(TextValue, Vec<StyleBinding>, Span, Option<u32>)>> {
        if self.is_caption_map_call(call) {
            let arrow = self.caption_map_callback(call)?;
            let element = self.span_callback(arrow)?;
            let (row, runs) = self.in_caption_row(arrow, |this| {
                this.lower_span_runs(element, &format!("{path}.run"))
            });
            return Some(
                runs.into_iter()
                    .map(|(text, styles, span)| (text, styles, span, Some(row)))
                    .collect(),
            );
        }
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let Some(serde_json::Value::Array(items)) = self.eval_static(&member.object) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                "a map inside <Text> maps a prepare-time array, or `caption.runs`",
            );
            return None;
        };
        let arrow = match &call.arguments[..] {
            [argument] => match argument.as_expression().map(peel_expr) {
                Some(Expression::ArrowFunctionExpression(arrow))
                    if !arrow.r#async
                        && arrow.params.rest.is_none()
                        && arrow.params.items.len() <= 2 =>
                {
                    arrow.as_ref()
                }
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        call.span(),
                        "array map callback must be synchronous `(item, index) => JSX`",
                    );
                    return None;
                }
            },
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    call.span(),
                    "array map requires one arrow callback",
                );
                return None;
            }
        };
        let element = self.span_callback(arrow)?;
        if items.len() > MAX_STATIC_MAP_ITEMS
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                format!(
                    "prepare-time map expands {} items; the per-map limit is {MAX_STATIC_MAP_ITEMS} and the module budget is {MAX_TOTAL_EXPANDED_LIST_ITEMS}",
                    items.len()
                ),
            );
            return None;
        }
        self.expanded_list_items += items.len();
        let saved = self.bindings.snapshot_frame();
        let mut runs = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            self.bindings.restore_frame(saved.clone());
            if let Some(parameter) = arrow.params.items.first() {
                self.bind_static_pattern(&parameter.pattern, item);
            }
            if let Some(parameter) = arrow.params.items.get(1) {
                self.bind_static_pattern(&parameter.pattern, serde_json::json!(index));
            }
            runs.extend(
                self.lower_span_runs(element, &format!("{path}.{index}"))
                    .into_iter()
                    .map(|(text, styles, span)| (text, styles, span, None)),
            );
        }
        self.bindings.restore_frame(saved);
        Some(runs)
    }
}

/// Whether `callback` is an arrow function that returns JSX.
pub(super) fn returns_jsx(callback: &Expression<'_>) -> bool {
    use oxc::ast_visit::Visit;

    struct Scan {
        found: bool,
    }
    impl<'a> Visit<'a> for Scan {
        fn visit_jsx_element(&mut self, _: &JSXElement<'a>) {
            self.found = true;
        }
        fn visit_jsx_fragment(&mut self, _: &oxc::ast::ast::JSXFragment<'a>) {
            self.found = true;
        }
    }

    let Expression::ArrowFunctionExpression(arrow) = peel_expr(callback) else {
        return false;
    };
    let mut scan = Scan { found: false };
    scan.visit_arrow_function_expression(arrow);
    scan.found
}

/// The first call of `name` in `program`.
fn first_call(program: &Program<'_>, name: &str) -> Option<Span> {
    use oxc::ast_visit::{Visit, walk};

    struct Scan<'n> {
        name: &'n str,
        hit: Option<Span>,
    }
    impl<'a> Visit<'a> for Scan<'_> {
        fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
            if self.hit.is_none()
                && matches!(&call.callee, Expression::Identifier(callee) if callee.name == self.name)
            {
                self.hit = Some(call.span);
            }
            walk::walk_call_expression(self, call);
        }
    }

    let mut scan = Scan { name, hit: None };
    scan.visit_program(program);
    scan.hit
}
