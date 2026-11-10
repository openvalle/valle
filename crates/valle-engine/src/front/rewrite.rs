//! Rewriting for the tracing frontend. oxc strips TypeScript and turns JSX into
//! `__h(type, props, ...children)`; the printed JavaScript is parsed again and every operation a
//! traced value can reach becomes a runtime call, by inserting text at node spans (JavaScript has
//! no operator overloading):
//!
//! | Source | Rewritten |
//! |---|---|
//! | `a op b`, `x op= v` | `__b("op", a, b)`, `x = __b("op", x, v)` |
//! | `op a` | `__u("op", a)` |
//! | `c ? a : b`, `a && b`, `a \|\| b`, `a ?? b` | `__sel(c, () => (a), () => (b), site)`, `__and(a, () => (b), site)`, … |
//! | `` `s${x}` `` | `__tpl(["s", ""], x)` |
//! | `o.p`, `o[k]` | `__get(o, "p")`, `__idx(o, k, site)` |
//! | `o.m(...a)` | `__call(o, "m", site, ...a)` |
//! | `__h(type, …)` (JSX) | `__hs(site, type, …)` |
//! | `if (c)`, loop tests | `if (__if(c, site))` |
//! | `switch (d)` | `switch (__sw(d, site))` |
//! | `...x` | `...__spread(x)` |
//!
//! Assignment targets, optional chains and tagged templates are left alone.
//!
//! Imports of relative modules, `import D, { A, B as C } from "./x"`, become
//! `const { default: D, A, B: C } = __import("dir/x")` on the same line: the module's path joined
//! to the importer's directory, without its extension, which the loader resolves. A module
//! evaluates to an object of its exports.
//!
//! `async` and `await` are the one syntax rule: a frame is computed synchronously, so there is
//! nothing to wait for. Everything else a scene cannot do comes from what its environment lacks.
//!
//! Every class a `className` may take is checked where it is written, as Tailwind finds classes
//! in source, before a frame reaches it.

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    ArrowFunctionExpression, AssignmentExpression, AssignmentTarget, AwaitExpression,
    BindingPattern, Declaration, DoWhileStatement, Expression, ForOfStatement, ForStatement,
    Function, IfStatement, JSXAttribute, JSXAttributeValue, ObjectProperty, ObjectPropertyKind,
    Program, PropertyKey, SpreadElement, Statement, SwitchStatement, WhileStatement,
};
use oxc::ast_visit::{Visit, walk};
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::{GetSpan, SourceType};
use oxc::str::JSStr;
use oxc::syntax::scope::ScopeFlags;
use oxc::transformer::{JsxRuntime, TransformOptions, Transformer};

use super::Composition;
use super::controls::{self, Prop};
use super::role::{self, Role};
use crate::time::{Rate, Tick};

/// A place in the source where the runtime may record a guard or an element.
pub struct Site {
    /// 1-based line and column (UTF-16 units) in the original file; 0 where unknown.
    pub line: u32,
    pub col: u32,
}

pub struct Rewritten {
    /// An expression that evaluates to the module's exports, `default` included.
    pub code: String,
    /// Sites numbered from `first_site`, as given to `rewrite`.
    pub sites: Vec<Site>,
    /// Relative modules this one imports, as module keys, in import order.
    pub imports: Vec<String>,
    /// Names the module reads without declaring: built-in components and functions.
    pub free: Vec<String>,
    /// Original line of each line of `code`, 0 where unknown.
    pub lines: Vec<u32>,
    /// The module's `composition` export, read without running it.
    pub composition: Option<Composition>,
    /// The props its `controls` export declares, read without running it.
    pub props: Vec<Prop>,
    /// The data it declares.
    pub data: Vec<Prop>,
    /// The resources it declares.
    pub resources: Vec<Slot>,
    /// Its role in a Timeline.
    pub role: Role,
}

/// A resource `controls.resources` declares: its name, what it holds, and the file it reads, as
/// written, unless another is bound to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub name: String,
    pub kind: Holds,
    /// Its default files: none, one, or for a font, the faces of its family.
    pub default: Vec<String>,
}

/// The names of the built-in font families, which no font resource takes.
pub const BUILT_IN_FAMILIES: [&str; 4] = ["sans-serif", "serif", "monospace", "emoji"];

/// What a resource holds: an image, an SVG file among them, a font, whose family is the
/// resource's name, a shader's WGSL code, or a video, which the host decodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Holds {
    Image,
    Font,
    Shader,
    Video,
}

impl Holds {
    pub fn name(self) -> &'static str {
        match self {
            Holds::Image => "image",
            Holds::Font => "font",
            Holds::Shader => "shader",
            Holds::Video => "video",
        }
    }
}

/// What `code` starts with before the first line of the module body.
const PREFIX: &str = "(function () {\n";

pub fn rewrite(path: &str, source: &str, first_site: usize) -> Result<Rewritten, String> {
    let path = Path::new(path);
    // TypeScript and JSX → plain JavaScript, with a source map back to the file.
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|e| format!("{e:?}"))?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(diagnostics(&parsed.diagnostics, source));
    }
    let mut program = parsed.program;
    let source_starts = line_starts(source);
    let place = |offset: u32| -> (u32, u32) {
        let line = source_starts.partition_point(|&s| s <= offset as usize) - 1;
        let col = source[source_starts[line]..offset as usize]
            .encode_utf16()
            .count();
        (line as u32 + 1, col as u32 + 1)
    };
    let mut asynchronous = Async(None);
    asynchronous.visit_program(&program);
    if let Some(offset) = asynchronous.0 {
        let (line, col) = place(offset);
        return Err(format!(
            "{line}:{col}: async functions and await are not supported: a frame is computed \
             synchronously, so there is nothing to wait for"
        ));
    }
    let at = |(offset, e): (u32, String)| {
        let (line, col) = place(offset);
        format!("{line}:{col}: {e}")
    };
    let composition = composition(&program).map_err(at)?;
    let role = match exported(&program, "role") {
        None => Role::Clip,
        Some((offset, e)) => {
            let e = e.ok_or((offset, "role is declared with its value".to_string()));
            e.and_then(|e| role::read(e).map_err(|m| (offset, m)))
                .map_err(at)?
        }
    };
    let Declared {
        props,
        data,
        resources,
    } = controls(&program).map_err(at)?;
    let mut classes = Classes(None);
    classes.visit_program(&program);
    if let Some(e) = classes.0 {
        return Err(at(e));
    }
    // Where each JSX element and fragment opens, in source order. The transform keeps their
    // order, so the k-th `__h` call of the printed code is the k-th of them; the source map only
    // places a call near some token of it.
    let mut jsx = Jsx(Vec::new());
    jsx.visit_program(&program);
    jsx.0.sort_unstable();
    let jsx_at: Vec<(u32, u32)> = jsx.0.iter().map(|&offset| place(offset)).collect();
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let mut options = TransformOptions::default();
    options.jsx.runtime = JsxRuntime::Classic;
    options.jsx.pragma = Some("__h".into());
    options.jsx.pragma_frag = Some("__F".into());
    options.jsx.display_name_plugin = false;
    options.jsx.pure = false;
    options.jsx.use_spread = Some(true);
    // Only `import type` goes, as with `verbatimModuleSyntax`: an import the module does not use
    // still names a file that must exist.
    options.typescript.only_remove_type_imports = true;
    let transformed =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.has_errors() {
        return Err(diagnostics(&transformed.diagnostics, source));
    }
    let printed = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let map = printed.map.ok_or("codegen returned no source map")?;
    let table = map.generate_lookup_table();
    let dir = path
        .parent()
        .map_or(String::new(), |d| d.to_string_lossy().into_owned());
    let (js, exports, imports) = strip_module(&printed.code, &dir)?;
    let js_starts = line_starts(&js);
    let original = |offset: u32| -> (u32, u32) {
        let line = js_starts.partition_point(|&s| s <= offset as usize) - 1;
        let col = js[js_starts[line]..offset as usize].encode_utf16().count() as u32;
        map.lookup_token_approx(&table, line as u32, col)
            .map_or((0, 0), |t| (t.get_src_line() + 1, t.get_src_col() + 1))
    };

    // Rewrite the printed JavaScript.
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &js, SourceType::mjs()).parse();
    if parsed.diagnostics.has_errors() {
        return Err(diagnostics(&parsed.diagnostics, &js));
    }
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let mut free: Vec<String> = semantic
        .scoping()
        .root_unresolved_references()
        .keys()
        .map(|name| name.to_string())
        .collect();
    free.sort();
    let mut collector = Collector {
        first_site,
        ..Collector::default()
    };
    collector.visit_program(&parsed.program);
    // Element sites by their call's rank in the printed code, when the counts agree.
    let mut calls: Vec<(u32, usize)> = collector
        .sites
        .iter()
        .enumerate()
        .filter(|(_, s)| s.1 == "element")
        .map(|(i, s)| (s.0, i))
        .collect();
    calls.sort_unstable();
    let mut exact = vec![None; collector.sites.len()];
    if calls.len() == jsx_at.len() {
        for (rank, &(_, i)) in calls.iter().enumerate() {
            exact[i] = Some(jsx_at[rank]);
        }
    }
    let sites = collector
        .sites
        .iter()
        .zip(exact)
        .map(|(&(offset, _), exact)| {
            let (line, col) = exact.unwrap_or_else(|| original(offset));
            Site { line, col }
        })
        .collect();
    let (body, runs) = apply(&js, collector.edits);

    let code = format!(
        "{PREFIX}{body}\nreturn {{ {} }};\n}})()",
        exports.join(", ")
    );
    // Original line of each output line, through the copied runs back into `js`.
    let mut lines = vec![0];
    for start in line_starts(&body) {
        let at = runs.partition_point(|&(out, _)| out <= start);
        let line = match at.checked_sub(1).map(|i| runs[i]) {
            Some((out, input)) => original((input + (start - out)).min(js.len()) as u32).0,
            None => 0,
        };
        lines.push(line);
    }
    Ok(Rewritten {
        code,
        sites,
        imports,
        free,
        lines,
        composition,
        props,
        data,
        resources,
        role,
    })
}

fn diagnostics(list: &[oxc::diagnostics::OxcDiagnostic], source: &str) -> String {
    let mut out = String::new();
    for d in list {
        out.push_str(&d.clone().render_with_source_code(source.to_string()));
        out.push('\n');
    }
    out
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// The earliest `async` function or `await`.
struct Async(Option<u32>);

impl Async {
    fn found(&mut self, offset: u32) {
        self.0 = Some(self.0.map_or(offset, |o| o.min(offset)));
    }
}

impl<'a> Visit<'a> for Async {
    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        if f.r#async {
            self.found(f.span.start);
        }
        walk::walk_function(self, f, flags);
    }

    fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
        if f.r#async {
            self.found(f.span.start);
        }
        walk::walk_arrow_function_expression(self, f);
    }

    fn visit_await_expression(&mut self, e: &AwaitExpression<'a>) {
        self.found(e.span.start);
        walk::walk_await_expression(self, e);
    }

    fn visit_for_of_statement(&mut self, s: &ForOfStatement<'a>) {
        if s.r#await {
            self.found(s.span.start);
        }
        walk::walk_for_of_statement(self, s);
    }
}

/// The `composition` export: an object literal of literals, which tools read without running the
/// scene. Errors carry the source offset they point at.
fn composition(program: &Program<'_>) -> Result<Option<Composition>, (u32, String)> {
    let Some((at, init)) = exported(program, "composition") else {
        return Ok(None);
    };
    let not_static = || {
        (
            at,
            "composition must be an object literal of numbers, strings and booleans: tools read \
             it without running the scene"
                .to_string(),
        )
    };
    let Some(Expression::ObjectExpression(object)) = init else {
        return Err(not_static());
    };
    let mut fields = Vec::new();
    for p in &object.properties {
        let ObjectPropertyKind::ObjectProperty(p) = p else {
            return Err(not_static());
        };
        let key = key(p).ok_or_else(not_static)?;
        let value = match &p.value {
            Expression::NumericLiteral(n) => Literal::Number(n.value),
            Expression::StringLiteral(s) => {
                Literal::Text(utf8(s.value).map_err(|e| (s.span.start, e))?.to_owned())
            }
            Expression::BooleanLiteral(b) => Literal::Bool(b.value),
            _ => return Err(not_static()),
        };
        fields.push((key, value, p.span.start));
    }
    composition_fields(&fields, at).map(Some)
}

/// The value of `export const name = …`, past `as`, `satisfies` and parentheses, and where the
/// declaration starts.
fn exported<'b, 'a>(
    program: &'b Program<'a>,
    name: &str,
) -> Option<(u32, Option<&'b Expression<'a>>)> {
    for statement in &program.body {
        let Statement::ExportDeclaration(export) = statement else {
            continue;
        };
        let Declaration::VariableDeclaration(declaration) = &export.declaration else {
            continue;
        };
        for d in &declaration.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            if id.name.as_str() == name {
                return Some((d.span.start, d.init.as_ref().map(bare)));
            }
        }
    }
    None
}

pub(super) fn bare<'b, 'a>(mut e: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        e = match e {
            Expression::TSAsExpression(x) => &x.expression,
            Expression::TSSatisfiesExpression(x) => &x.expression,
            Expression::ParenthesizedExpression(x) => &x.expression,
            _ => return e,
        };
    }
}

/// A property's name, when it is written out.
pub(super) fn key<'b>(p: &'b ObjectProperty<'_>) -> Option<&'b str> {
    match &p.key {
        PropertyKey::StaticIdentifier(n) if !p.computed => Some(n.name.as_str()),
        PropertyKey::StringLiteral(s) => s.value.as_str(),
        _ => None,
    }
}

/// Metadata crosses into Rust/JSON, whose strings cannot contain lone UTF-16 surrogates.
pub(super) fn utf8(value: JSStr<'_>) -> Result<&str, String> {
    value
        .as_str()
        .ok_or_else(|| "a string cannot contain an unpaired UTF-16 surrogate".into())
}

/// The `controls` export's props, data and resources, `resources: { logo: resource({ kind:
/// "image", default: "./logo.png" }) }`, which tools read without running the scene.
fn controls(program: &Program<'_>) -> Result<Declared, (u32, String)> {
    let Some((at, init)) = exported(program, "controls") else {
        return Ok(Declared::default());
    };
    let literal = |at: u32, what: &str| {
        (
            at,
            format!("{what} must be an object literal: tools read it without running the scene"),
        )
    };
    let Some(Expression::ObjectExpression(object)) = init else {
        return Err(literal(at, "controls"));
    };
    let mut out = Declared::default();
    for p in &object.properties {
        let ObjectPropertyKind::ObjectProperty(p) = p else {
            return Err(literal(p.span().start, "controls"));
        };
        match key(p) {
            Some("resources") => {}
            Some(k @ ("props" | "data")) => {
                let Expression::ObjectExpression(o) = bare(&p.value) else {
                    return Err(literal(p.span.start, &format!("controls.{k}")));
                };
                if k == "props" {
                    out.props = controls::props(o)?;
                } else {
                    out.data = controls::data(o)?;
                }
                continue;
            }
            _ => {
                return Err((
                    p.span.start,
                    "controls takes props, data and resources".into(),
                ));
            }
        }
        let Expression::ObjectExpression(resources) = bare(&p.value) else {
            return Err(literal(p.span.start, "controls.resources"));
        };
        for r in &resources.properties {
            let ObjectPropertyKind::ObjectProperty(r) = r else {
                return Err(literal(r.span().start, "controls.resources"));
            };
            let at = r.span.start;
            let name = key(r).ok_or_else(|| literal(at, "controls.resources"))?;
            let named = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c));
            if !named || !(1..=64).contains(&name.len()) {
                return Err((
                    at,
                    format!(
                        "resource name {name:?} is not 1 to 64 ASCII letters, digits, dots, \
                         underscores and hyphens"
                    ),
                ));
            }
            let slot =
                slot(name, bare(&r.value)).map_err(|e| (at, format!("resource {name}: {e}")))?;
            out.resources.push(slot);
        }
    }
    if let Some(e) = controls::clash(&out.props, &out.data) {
        return Err((at, e));
    }
    Ok(out)
}

/// What a `controls` export declares.
#[derive(Default)]
struct Declared {
    props: Vec<Prop>,
    data: Vec<Prop>,
    resources: Vec<Slot>,
}

/// `resource({ kind: "image", default: "…" })`; a font's default may be a list of files.
fn slot(name: &str, e: &Expression<'_>) -> Result<Slot, String> {
    let written = || "write resource({ kind: \"image\", default: \"./file.png\" })".to_string();
    let Expression::CallExpression(call) = e else {
        return Err(written());
    };
    let named = matches!(&call.callee, Expression::Identifier(f) if f.name.as_str() == "resource");
    let spec = call
        .arguments
        .first()
        .and_then(|a| a.as_expression())
        .map(bare);
    let (true, 1, Some(Expression::ObjectExpression(spec))) = (named, call.arguments.len(), spec)
    else {
        return Err(written());
    };
    let (mut kind, mut default, mut list) = (None, Vec::new(), false);
    for p in &spec.properties {
        let ObjectPropertyKind::ObjectProperty(p) = p else {
            return Err(written());
        };
        let text = |e: &Expression<'_>| match bare(e) {
            Expression::StringLiteral(v) => v.value.as_str().map(str::to_owned),
            _ => None,
        };
        match (key(p), bare(&p.value)) {
            (Some("default"), Expression::ArrayExpression(files)) => {
                list = true;
                for file in &files.elements {
                    let file = file.as_expression().and_then(text);
                    default.push(file.ok_or("default is a string literal or a list of them")?);
                }
            }
            (Some(k @ ("kind" | "default")), v) => {
                let v = text(v).ok_or_else(|| format!("{k} is a string literal"))?;
                match k {
                    "kind" => kind = Some(v),
                    _ => default.push(v),
                }
            }
            _ => return Err("resource() takes kind and default".into()),
        }
    }
    let kind = match kind.as_deref() {
        Some("image") => Holds::Image,
        Some("font") => Holds::Font,
        Some("shader") => Holds::Shader,
        Some("video") => Holds::Video,
        Some(k @ ("model3d" | "environment")) => {
            return Err(format!("{k} resources are not supported yet"));
        }
        Some(k) => {
            return Err(format!(
                "{k:?} is not a kind of resource: image, font, shader or video"
            ));
        }
        None => {
            return Err(
                "resource() needs a kind: \"image\", \"font\", \"shader\" or \"video\"".into(),
            );
        }
    };
    // Its name is a family name, which the built-in ones would hide.
    if kind == Holds::Font
        && BUILT_IN_FAMILIES
            .iter()
            .any(|g| name.eq_ignore_ascii_case(g))
    {
        return Err(format!(
            "a font resource cannot be named {name}: that is a built-in family"
        ));
    }
    match (kind, default.len()) {
        (Holds::Image, _) if list => {
            return Err("an image resource has one file, not a list".into());
        }
        (Holds::Shader | Holds::Video, _) if list => {
            return Err(format!(
                "a {} resource has one file, not a list",
                kind.name()
            ));
        }
        (_, 0) if list => return Err("default lists no file".into()),
        _ => {}
    }
    for d in &default {
        located(d)?;
    }
    Ok(Slot {
        name: name.to_string(),
        kind,
        default,
    })
}

enum Literal {
    Number(f64),
    Text(String),
    Bool(bool),
}

fn composition_fields(
    fields: &[(&str, Literal, u32)],
    at: u32,
) -> Result<Composition, (u32, String)> {
    let (mut width, mut height, mut fps, mut duration, mut looped) =
        (None, None, None, None, false);
    for (key, value, offset) in fields {
        let bad = |what: &str| (*offset, format!("composition {key} must be {what}"));
        let size = |v: &Literal| match v {
            Literal::Number(x) if x.fract() == 0.0 && (1.0..=16384.0).contains(x) => Ok(*x as u32),
            _ => Err(bad("a whole number of pixels from 1 to 16384")),
        };
        match *key {
            "width" => width = Some(size(value)?),
            "height" => height = Some(size(value)?),
            "fps" => {
                let rate = match value {
                    Literal::Number(x) => Rate::from_f64(*x),
                    Literal::Text(t) => Rate::parse(t),
                    Literal::Bool(_) => Err(String::new()),
                };
                let what = "a number such as 30, or a ratio such as \"30000/1001\"";
                fps = Some(rate.map_err(|_| bad(what))?);
            }
            "duration" => {
                let seconds = match value {
                    Literal::Number(x) if *x > 0.0 => Tick::from_seconds(*x).ok(),
                    _ => None,
                };
                duration = Some(seconds.ok_or_else(|| bad("a positive number of seconds"))?);
            }
            "loop" => match value {
                Literal::Bool(b) => looped = *b,
                _ => return Err(bad("true or false")),
            },
            other => {
                return Err((
                    *offset,
                    format!(
                        "composition has no field {other}; it takes width, height, fps, duration \
                         and loop"
                    ),
                ));
            }
        }
    }
    let missing = |name: &str| (at, format!("composition needs {name}"));
    Ok(Composition {
        width: width.ok_or_else(|| missing("width"))?,
        height: height.ok_or_else(|| missing("height"))?,
        fps: fps.ok_or_else(|| missing("fps"))?,
        duration: duration.ok_or_else(|| missing("duration"))?,
        looped,
    })
}

/// Turns the module into a function body: `export` keywords become spaces, so offsets and columns
/// keep matching the source map, and each import becomes a `const` on its own line. Returns the
/// body, the exported names (`default: Name` for the default export) and the imported modules.
fn strip_module(code: &str, dir: &str) -> Result<(String, Vec<String>, Vec<String>), String> {
    let ident = |rest: &str| -> String {
        rest.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect()
    };
    let mut out = String::with_capacity(code.len());
    let mut exports = Vec::new();
    let mut imports = Vec::new();
    for line in code.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("export default function ") {
            let name = ident(rest);
            if name.is_empty() {
                return Err("the default export must be a named function, such as \
                     `export default function Scene()`"
                    .into());
            }
            exports.push(format!("default: {name}"));
            out.push_str(&" ".repeat("export default ".len()));
            out.push_str("function ");
            out.push_str(rest);
        } else if let Some(name) = line
            .strip_prefix("export default ")
            .map(|rest| rest.trim_end().trim_end_matches(';'))
            .filter(|name| !name.is_empty() && ident(name) == *name)
        {
            exports.push(format!("default: {name}"));
            out.push('\n');
        } else if let Some(rest) = line.strip_prefix("export ")
            && let Some(keyword) = ["const ", "let ", "function "]
                .iter()
                .find(|k| rest.starts_with(**k))
        {
            exports.push(ident(&rest[keyword.len()..]));
            out.push_str(&" ".repeat("export ".len()));
            out.push_str(rest);
        } else if line.starts_with("import ") {
            let (names, key) = import(line.trim_end(), dir)?;
            out.push_str(&format!(
                "const {{ {names} }} = __import({});\n",
                json(&key)
            ));
            imports.push(key);
        } else if line.starts_with("export ") {
            return Err(format!(
                "unsupported export: {}; export declarations instead, such as `export const x = …`",
                line.trim_end()
            ));
        } else {
            out.push_str(line);
        }
    }
    Ok((out, exports, imports))
}

/// `import D, { A, B as C } from "./x";` → (`default: D, A, B: C`, the module key of `./x`).
fn import(line: &str, dir: &str) -> Result<(String, String), String> {
    let unsupported = || {
        format!(
            "unsupported import: {line}; import names from a project file, such as \
             `import Title, {{ COLORS }} from \"./title\"`"
        )
    };
    let rest = line.strip_prefix("import ").ok_or_else(unsupported)?;
    let (clause, spec) = rest.rsplit_once(" from ").ok_or_else(unsupported)?;
    let spec = spec.trim().trim_end_matches(';').trim_matches(['"', '\'']);
    if spec == "d3" || spec.starts_with("d3-") {
        return Err(format!(
            "cannot import {spec:?}: d3's scales, shapes and projections are built in; call \
             them without importing"
        ));
    }
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return Err(format!(
            "cannot import {spec:?}: scenes import only project files (./ or ../), not packages, \
             which may use randomness, time or I/O; the built-in functions cover d3's scales and \
             shapes"
        ));
    }
    let clause = clause.trim();
    let (default, named) = match clause.split_once('{') {
        Some((default, named)) => (
            default.trim().trim_end_matches(',').trim(),
            named.strip_suffix('}').ok_or_else(unsupported)?,
        ),
        None => (clause, ""),
    };
    let is_name = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    let mut names = Vec::new();
    if !default.is_empty() {
        if !is_name(default) {
            return Err(unsupported());
        }
        names.push(format!("default: {default}"));
    }
    for n in named.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        names.push(match n.split_once(" as ") {
            Some((from, to)) => format!("{}: {}", from.trim(), to.trim()),
            None => n.to_string(),
        });
    }
    if names.is_empty() {
        return Err(unsupported());
    }
    Ok((names.join(", "), join(dir, spec)))
}

/// Whether a resource's file is on the web.
pub(crate) fn web(location: &str) -> bool {
    location.starts_with("http://") || location.starts_with("https://")
}

/// Checks that a resource's file is a path or an HTTP(S) address.
pub(super) fn located(location: &str) -> Result<(), String> {
    match location.contains("://") && !web(location) {
        true => Err(format!(
            "{location}: write a path, or an http:// or https:// address"
        )),
        false => Ok(()),
    }
}

/// `spec` relative to `dir`, with `.` and `..` segments resolved and `/` separators.
pub fn join(dir: &str, spec: &str) -> String {
    let mut parts: Vec<&str> = dir.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let absolute = dir.starts_with('/');
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Text inserted or replaced at a position of the printed JavaScript.
struct Edit {
    pos: u32,
    /// Closings, then openings, then a replacement at the same position.
    rank: u8,
    /// Closings deepest first, openings outermost first.
    depth: u32,
    end: u32,
    text: String,
}

const CLOSE: u8 = 0;
const OPEN: u8 = 1;
const REPLACE: u8 = 2;

/// Applies the edits; also returns (output offset, input offset) at the start of each copied run.
fn apply(src: &str, mut edits: Vec<Edit>) -> (String, Vec<(usize, usize)>) {
    edits.sort_by_key(|e| {
        let order = if e.rank == CLOSE {
            u32::MAX - e.depth
        } else {
            e.depth
        };
        (e.pos, e.rank, order)
    });
    let mut out = String::with_capacity(src.len() * 2);
    let mut runs = Vec::new();
    let mut cur = 0usize;
    for e in edits {
        let pos = e.pos as usize;
        if pos < cur {
            // Inside a replaced gap: nothing of the source is left there to edit.
            continue;
        }
        if pos > cur {
            runs.push((out.len(), cur));
            out.push_str(&src[cur..pos]);
            cur = pos;
        }
        out.push_str(&e.text);
        if e.rank == REPLACE {
            cur = e.end as usize;
        }
    }
    runs.push((out.len(), cur));
    out.push_str(&src[cur..]);
    (out, runs)
}

/// Opening offsets of JSX elements and fragments.
struct Jsx(Vec<u32>);

impl<'a> Visit<'a> for Jsx {
    fn visit_jsx_element(&mut self, e: &oxc::ast::ast::JSXElement<'a>) {
        self.0.push(e.span.start);
        walk::walk_jsx_element(self, e);
    }

    fn visit_jsx_fragment(&mut self, f: &oxc::ast::ast::JSXFragment<'a>) {
        self.0.push(f.span.start);
        walk::walk_jsx_fragment(self, f);
    }
}

/// The first class a `className` may take that is not one, and where it is written. A class is a
/// word of a string literal, or of a template literal's text, the value may be: through `? :`,
/// `&&`, `||` and `??`, as Tailwind finds classes in source. A word a template literal runs into a
/// value of is part of a class the frame makes, checked then.
struct Classes(Option<(u32, String)>);

impl Classes {
    fn value(&mut self, e: &Expression<'_>) {
        match bare(e) {
            Expression::StringLiteral(s) => self.literal(s.span.start + 1, s.value, false, false),
            Expression::TemplateLiteral(t) => {
                for (i, q) in t.quasis.iter().enumerate() {
                    if let Some(text) = q.value.cooked {
                        self.literal(q.span.start, text, i > 0, !q.tail);
                    } else {
                        self.words(q.span.start, &q.value.raw, i > 0, !q.tail);
                    }
                }
                t.expressions.iter().for_each(|e| self.value(e));
            }
            Expression::ConditionalExpression(c) => {
                self.value(&c.consequent);
                self.value(&c.alternate);
            }
            Expression::LogicalExpression(l) => {
                self.value(&l.left);
                self.value(&l.right);
            }
            _ => {}
        }
    }

    fn literal(&mut self, at: u32, value: JSStr<'_>, after: bool, before: bool) {
        match utf8(value) {
            Ok(text) => self.words(at, text, after, before),
            Err(e) if self.0.is_none() => self.0 = Some((at, e)),
            Err(_) => {}
        }
    }

    /// Checks the words of `text`, written from `at`, but a first one a value comes right
    /// before, `after` one, and a last one it comes right after, `before` one.
    fn words(&mut self, at: u32, text: &str, after: bool, before: bool) {
        let space = |c: Option<char>| c.is_none_or(|c| c.is_ascii_whitespace());
        let words: Vec<&str> = text.split_ascii_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let cut = (i == 0 && after && !space(text.chars().next()))
                || (i + 1 == words.len() && before && !space(text.chars().last()));
            if cut || self.0.is_some() {
                continue;
            }
            if let Err(e) = crate::style::tailwind::class(word) {
                let offset = word.as_ptr() as usize - text.as_ptr() as usize;
                self.0 = Some((at + offset as u32, e));
            }
        }
    }
}

impl<'a> Visit<'a> for Classes {
    fn visit_jsx_attribute(&mut self, a: &JSXAttribute<'a>) {
        if a.is_identifier("className") {
            match &a.value {
                Some(JSXAttributeValue::StringLiteral(s)) => {
                    self.literal(s.span.start + 1, s.value, false, false);
                }
                Some(JSXAttributeValue::ExpressionContainer(c)) => {
                    if let Some(e) = c.expression.as_expression() {
                        self.value(e);
                    }
                }
                _ => {}
            }
        }
        walk::walk_jsx_attribute(self, a);
    }
}

#[derive(Default)]
struct Collector {
    edits: Vec<Edit>,
    sites: Vec<(u32, &'static str)>,
    first_site: usize,
    depth: u32,
}

fn json(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

impl Collector {
    fn open(&mut self, pos: u32, text: impl Into<String>) {
        self.edit(pos, OPEN, pos, text);
    }

    fn close(&mut self, pos: u32, text: impl Into<String>) {
        self.edit(pos, CLOSE, pos, text);
    }

    fn replace(&mut self, start: u32, end: u32, text: impl Into<String>) {
        self.edit(start, REPLACE, end, text);
    }

    fn edit(&mut self, pos: u32, rank: u8, end: u32, text: impl Into<String>) {
        self.edits.push(Edit {
            pos,
            rank,
            depth: self.depth,
            end,
            text: text.into(),
        });
    }

    fn site(&mut self, offset: u32, kind: &'static str) -> usize {
        self.sites.push((offset, kind));
        self.first_site + self.sites.len() - 1
    }

    /// `__h(type, …)` → `__hs(site, type, …)`: the element's place in the source.
    fn element_site(&mut self, callee: oxc::span::Span, c: &oxc::ast::ast::CallExpression<'_>) {
        let site = self.site(c.span.start, "element");
        self.replace(callee.start, callee.end, "__hs");
        self.replace(
            callee.end,
            c.arguments[0].span().start,
            format!("({site}, "),
        );
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self)) {
        self.depth += 1;
        f(self);
        self.depth -= 1;
    }

    /// `__call(object, key, site, ...arguments)`, where `key` is the computed key's expression
    /// or a property name.
    fn call<'a>(
        &mut self,
        call: &oxc::ast::ast::CallExpression<'a>,
        object: &Expression<'a>,
        key: Result<&Expression<'a>, &str>,
    ) {
        let site = self.site(call.span.start, "call");
        self.open(object.span().start, "__call(");
        let after_key = match key {
            Ok(computed) => {
                self.replace(object.span().end, computed.span().start, ", ");
                Some(computed.span().end)
            }
            Err(name) => {
                let head = format!(", {}", json(name));
                match call.arguments.first() {
                    Some(first) => self.replace(
                        object.span().end,
                        first.span().start,
                        format!("{head}, {site}, "),
                    ),
                    None => {
                        self.replace(object.span().end, call.span.end, format!("{head}, {site})"))
                    }
                }
                None
            }
        };
        if let Some(after_key) = after_key {
            match call.arguments.first() {
                Some(first) => self.replace(after_key, first.span().start, format!(", {site}, ")),
                None => self.replace(after_key, call.span.end, format!(", {site})")),
            }
        }
        if let Some(last) = call.arguments.last() {
            self.replace(last.span().end, call.span.end, ")");
        }
        self.nested(|s| {
            s.visit_expression(object);
            if let Ok(computed) = key {
                s.visit_expression(computed);
            }
            s.visit_arguments(&call.arguments);
        });
    }

    /// `prefix test, site)` around `test`.
    fn wrap(&mut self, test: &Expression<'_>, prefix: &str, kind: &'static str) {
        let site = self.site(test.span().start, kind);
        self.open(test.span().start, prefix);
        self.close(test.span().end, format!(", {site})"));
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_expression(&mut self, e: &Expression<'a>) {
        match e {
            Expression::BinaryExpression(b)
                if !matches!(b.operator.as_str(), "in" | "instanceof") =>
            {
                self.open(
                    b.left.span().start,
                    format!("__b({}, ", json(b.operator.as_str())),
                );
                self.replace(b.left.span().end, b.right.span().start, ", ");
                self.close(b.right.span().end, ")");
                self.nested(|s| {
                    s.visit_expression(&b.left);
                    s.visit_expression(&b.right);
                });
            }
            Expression::LogicalExpression(l) => {
                let (f, kind) = match l.operator.as_str() {
                    "&&" => ("__and", "and"),
                    "||" => ("__or", "or"),
                    _ => ("__nc", "nullish"),
                };
                let site = self.site(l.span.start, kind);
                self.open(l.left.span().start, format!("{f}("));
                self.replace(l.left.span().end, l.right.span().start, ", () => (");
                self.close(l.right.span().end, format!("), {site})"));
                self.nested(|s| {
                    s.visit_expression(&l.left);
                    s.visit_expression(&l.right);
                });
            }
            Expression::ConditionalExpression(c) => {
                let site = self.site(c.span.start, "select");
                self.open(c.test.span().start, "__sel(");
                self.replace(c.test.span().end, c.consequent.span().start, ", () => (");
                self.replace(
                    c.consequent.span().end,
                    c.alternate.span().start,
                    "), () => (",
                );
                self.close(c.alternate.span().end, format!("), {site})"));
                self.nested(|s| {
                    s.visit_expression(&c.test);
                    s.visit_expression(&c.consequent);
                    s.visit_expression(&c.alternate);
                });
            }
            Expression::UnaryExpression(u) if !matches!(u.operator.as_str(), "void" | "delete") => {
                self.replace(
                    u.span.start,
                    u.argument.span().start,
                    format!("__u({}, ", json(u.operator.as_str())),
                );
                self.close(u.argument.span().end, ")");
                self.nested(|s| s.visit_expression(&u.argument));
            }
            Expression::TemplateLiteral(t) if !t.expressions.is_empty() => {
                // Keep each chunk's JS escapes: JSON/Rust strings cannot represent lone
                // surrogates, which JavaScript can join into a valid pair at run time.
                let strings: Vec<String> = t
                    .quasis
                    .iter()
                    .map(|q| match q.value.cooked.and_then(JSStr::as_str) {
                        Some(text) => json(text),
                        None => format!("`{}`", q.value.raw),
                    })
                    .collect();
                let n = t.expressions.len();
                self.replace(
                    t.span.start,
                    t.expressions[0].span().start,
                    format!("__tpl([{}], ", strings.join(",")),
                );
                for i in 1..n {
                    self.replace(
                        t.expressions[i - 1].span().end,
                        t.expressions[i].span().start,
                        ", ",
                    );
                }
                self.replace(t.expressions[n - 1].span().end, t.span.end, ")");
                self.nested(|s| {
                    for x in &t.expressions {
                        s.visit_expression(x);
                    }
                });
            }
            Expression::StaticMemberExpression(m) if !m.optional => {
                self.open(m.object.span().start, "__get(");
                self.replace(
                    m.object.span().end,
                    m.span.end,
                    format!(", {})", json(m.property.name.as_str())),
                );
                self.nested(|s| s.visit_expression(&m.object));
            }
            Expression::ComputedMemberExpression(m) if !m.optional => {
                let site = self.site(m.span.start, "index");
                self.open(m.object.span().start, "__idx(");
                self.replace(m.object.span().end, m.expression.span().start, ", ");
                self.replace(m.expression.span().end, m.span.end, format!(", {site})"));
                self.nested(|s| {
                    s.visit_expression(&m.object);
                    s.visit_expression(&m.expression);
                });
            }
            Expression::CallExpression(c) if !c.optional => match &c.callee {
                Expression::StaticMemberExpression(m) if !m.optional => {
                    self.call(c, &m.object, Err(m.property.name.as_str()));
                }
                Expression::ComputedMemberExpression(m) if !m.optional => {
                    self.call(c, &m.object, Ok(&m.expression));
                }
                Expression::Identifier(h) if h.name == "__h" && !c.arguments.is_empty() => {
                    self.element_site(h.span, c);
                    walk::walk_expression(self, e);
                }
                _ => walk::walk_expression(self, e),
            },
            _ => walk::walk_expression(self, e),
        }
    }

    fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
        if let (Some(op), AssignmentTarget::AssignmentTargetIdentifier(id)) =
            (a.operator.to_binary_operator(), &a.left)
        {
            self.replace(
                a.left.span().end,
                a.right.span().start,
                format!(" = __b({}, {}, ", json(op.as_str()), id.name.as_str()),
            );
            self.close(a.right.span().end, ")");
            self.nested(|s| s.visit_expression(&a.right));
        } else {
            walk::walk_assignment_expression(self, a);
        }
    }

    fn visit_spread_element(&mut self, it: &SpreadElement<'a>) {
        self.open(it.argument.span().start, "__spread(");
        self.close(it.argument.span().end, ")");
        self.nested(|s| s.visit_expression(&it.argument));
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.wrap(&it.test, "__if(", "if");
        self.nested(|s| walk::walk_if_statement(s, it));
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.wrap(&it.test, "__if(", "loop");
        self.nested(|s| walk::walk_while_statement(s, it));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.wrap(&it.test, "__if(", "loop");
        self.nested(|s| walk::walk_do_while_statement(s, it));
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        if let Some(test) = &it.test {
            self.wrap(test, "__if(", "loop");
        }
        self.nested(|s| walk::walk_for_statement(s, it));
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.wrap(&it.discriminant, "__sw(", "switch");
        self.nested(|s| walk::walk_switch_statement(s, it));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(source: &str) -> String {
        rewrite("scene.motion.tsx", source, 0)
            .err()
            .expect("an error")
    }

    #[test]
    fn async_and_await_are_rejected_with_their_place() {
        let e = err("const a = 1;\nasync function f() {}\n");
        assert!(e.starts_with("2:1: async functions and await"), "{e}");
        let e = err("function f() {\n  const g = async () => 1;\n}\n");
        assert!(e.starts_with("2:13:"), "{e}");
    }

    #[test]
    fn roles_are_read_statically() {
        let role = |code: &str| rewrite("s.motion.tsx", code, 0).map(|r| r.role);
        let s = |x: f64| Tick::from_seconds(x).unwrap();
        assert_eq!(role("export const a = 1;\n"), Ok(Role::Clip));
        assert_eq!(
            role("export const role = overlay({ intro: 0.5, outro: 1, hold: \"loop\" });\n"),
            Ok(Role::Overlay {
                intro: s(0.5),
                outro: s(1.0),
                hold: role::Hold::Loop
            })
        );
        assert_eq!(
            role("export const role = overlay();\n"),
            Ok(Role::Overlay {
                intro: Tick::ZERO,
                outro: Tick::ZERO,
                hold: role::Hold::Once
            })
        );
        assert_eq!(
            role("export const role = transition();\n"),
            Ok(Role::Transition)
        );
        assert_eq!(role("export const role = effect();\n"), Ok(Role::Effect));
        assert_eq!(
            role("export const role = captionPresenter({ intro: 0.25 });\n"),
            Ok(Role::CaptionPresenter {
                intro: s(0.25),
                outro: Tick::ZERO
            })
        );
        let e = err("const r = effect();\nexport const role = r;\n");
        assert!(e.starts_with("2:14: role must be one of overlay"), "{e}");
        let e = err("export const role = overlay({ hold: \"freeze\" });\n");
        assert!(
            e.ends_with("hold is \"once\", \"loop\" or \"stretch\", not \"freeze\""),
            "{e}"
        );
        let e = err("export const role = overlay({ intro: -1 });\n");
        assert!(e.ends_with("intro is seconds, 0 or more, not -1"), "{e}");
        let e = err("export const role = effect({ amount: 1 });\n");
        assert!(
            e.ends_with("role effect() has no option amount; it takes none"),
            "{e}"
        );
    }

    #[test]
    fn composition_is_read_statically() {
        let r = rewrite(
            "s.motion.tsx",
            "export const composition = { width: 1920, height: 1080, fps: \"30000/1001\", \
             duration: 4.5, loop: true } as const;\n",
            0,
        )
        .unwrap();
        let c = r.composition.unwrap();
        assert_eq!((c.width, c.height), (1920, 1080));
        assert_eq!(c.fps, Rate::new(30000, 1001).unwrap());
        assert_eq!(c.duration, Tick::parse_seconds("4.5").unwrap());
        assert!(c.looped);
        let e = err("const W = 1920;\nexport const composition = { width: W };\n");
        assert!(
            e.starts_with("2:14: composition must be an object literal"),
            "{e}"
        );
        let e = err("export const composition = { width: 10, height: 10, fps: 30 };\n");
        assert!(e.contains("composition needs duration"), "{e}");
        let e = err("export const composition = { width: 10.5 };\n");
        assert!(
            e.contains("composition width must be a whole number"),
            "{e}"
        );
        let e = err("export const composition = { size: 1 };\n");
        assert!(e.contains("no field size"), "{e}");
    }

    #[test]
    fn controls_declare_resources_read_statically() {
        let r = rewrite(
            "s.motion.tsx",
            "export const controls = { resources: { logo: resource({ kind: \"image\", default: \
             \"../media/logo.png\" }), \"hero-2.b\": resource({ kind: \"image\" }), brand: \
             resource({ kind: \"font\", default: \"Brand.ttf\" }), faces: resource({ kind: \
             \"font\", default: [\"A.ttf\", \"https://x.org/B.otf\"] }) } };\n",
            0,
        )
        .unwrap();
        let slot = |name: &str, kind: Holds, default: &[&str]| Slot {
            name: name.into(),
            kind,
            default: default.iter().map(|d| d.to_string()).collect(),
        };
        assert_eq!(
            r.resources,
            [
                slot("logo", Holds::Image, &["../media/logo.png"]),
                slot("hero-2.b", Holds::Image, &[]),
                slot("brand", Holds::Font, &["Brand.ttf"]),
                slot("faces", Holds::Font, &["A.ttf", "https://x.org/B.otf"]),
            ]
        );
        for (source, says) in [
            (
                "const c = {};\nexport const controls = c;",
                "2:14: controls must be an object",
            ),
            (
                "export const controls = { fonts: {} };",
                "controls takes props, data and resources",
            ),
            (
                "export const controls = { resources: { a: { kind: \"image\" } } };",
                "resource a: write resource({ kind: \"image\"",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"model3d\" }) } };",
                "resource a: model3d resources are not supported yet",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"photo\" }) } };",
                "\"photo\" is not a kind of resource: image, font, shader or video",
            ),
            (
                "export const controls = { resources: { Serif: resource({ kind: \"font\" }) } };",
                "resource Serif: a font resource cannot be named Serif: that is a built-in family",
            ),
            (
                "export const controls = { resources: { emoji: resource({ kind: \"font\" }) } };",
                "resource emoji: a font resource cannot be named emoji",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"image\", default: \
                 [\"a.png\"] }) } };",
                "resource a: an image resource has one file, not a list",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"font\", default: [] \
                 }) } };",
                "resource a: default lists no file",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"font\", default: \
                 [\"a.ttf\", 1] }) } };",
                "resource a: default is a string literal or a list of them",
            ),
            (
                "export const controls = { resources: { a: resource({ default: \"a.png\" }) } };",
                "resource() needs a kind",
            ),
            (
                "export const controls = { resources: { a: resource({ kind: \"image\", default: \
                 \"file:///a.png\" }) } };",
                "resource a: file:///a.png: write a path, or an http:// or https:// address",
            ),
            (
                "export const controls = { resources: { \"a b\": resource({ kind: \"image\" }) } };",
                "resource name \"a b\" is not 1 to 64",
            ),
        ] {
            let e = err(source);
            assert!(e.contains(says), "{source}: {e}");
        }
    }

    #[test]
    fn classes_a_class_name_may_take_are_checked_where_written() {
        let scene = |class: &str| format!("const a = 1;\nconst v = <View className={class} />;\n");
        assert!(rewrite("s.tsx", &scene("{a > 1 ? `p-4 ${a}` : \"m-2\"}"), 0).is_ok());
        // Words running into a value are left to the frame.
        assert!(rewrite("s.tsx", &scene("{`text-${a} ${a}-x`}"), 0).is_ok());
        for (class, says) in [
            (
                "\"block hover:p-4\"",
                "2:34: className hover:p-4: variants are not supported",
            ),
            (
                "{a > 1 ? \"p-4\" : \"p-foo\"}",
                "2:45: className p-foo is not a Tailwind class",
            ),
            (
                "{`p-4 ${a && \"flex-grow\"} m-1`}",
                "2:41: className flex-grow is a Tailwind v3 name: write grow",
            ),
            ("{`p-4 ${a} m-1 w-`}", "2:42: className w- is not"),
        ] {
            let e = err(&scene(class));
            assert!(e.starts_with(says), "{class}: {e}");
        }
    }

    #[test]
    fn imports_and_exports() {
        let r = rewrite(
            "dir/s.motion.tsx",
            "import Title, { A, B as C } from \"../lib/title\";\nimport { D } from './d';\n\
             export const x = A;\nexport default function Scene() { return null; }\n",
            0,
        )
        .unwrap();
        assert_eq!(r.imports, ["lib/title", "dir/d"]);
        assert!(
            r.code
                .contains("const { default: Title, A, B: C } = __import(\"lib/title\");")
        );
        assert!(
            r.code.contains("return { x, default: Scene };"),
            "{}",
            r.code
        );
        let r = rewrite(
            "s.tsx",
            "function Scene() { return null; }\nexport default Scene;\n",
            0,
        )
        .unwrap();
        assert!(r.code.contains("return { default: Scene };"), "{}", r.code);
        assert!(err("import { scaleLinear } from \"d3\";\n").contains("built in"));
        assert!(err("import { lodash } from \"lodash\";\n").contains("not packages"));
        assert!(err("import * as M from \"./m\";\n").contains("unsupported import"));
        assert!(err("const a = 1;\nexport { a };\n").contains("unsupported export"));
    }

    #[test]
    fn operations_become_runtime_calls() {
        let r = rewrite(
            "s.js",
            "const f = (t) => t > 1 ? `x${t * 2}` : o.p[k];\nif (t) { m.n(1); }\n",
            0,
        )
        .unwrap();
        for part in [
            "__sel(__b(\">\", t, 1)",
            "__tpl([\"x\",\"\"], __b(\"*\", t, 2))",
            "__idx(__get(o, \"p\"), k, ",
            "__call(m, \"n\", ",
            "if (__if(t, ",
        ] {
            assert!(r.code.contains(part), "{part} in\n{}", r.code);
        }
        assert_eq!(r.lines[1], 1);
    }
}
