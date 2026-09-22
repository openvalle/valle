//! Deterministic compile-time JavaScript sandbox. Reject time, unseeded randomness, I/O, and other
//! nondeterministic globals. Lexical scanning provides source-specific errors; runtime throwing
//! getters also catch indirect access that bypasses scanning.

use rquickjs::{Context as JsContext, Runtime as JsRuntime};
use valle_motion::diag::{DiagCode, MotionDiagnostic};

/// Marker distinguishing forbidden access from ordinary evaluation failures.
const FORBIDDEN_MARK: &str = "valle:forbidden:";

/// Private value binding inserted by the Motion compiler while lowering a ThemeProvider subtree.
/// `eval_json` activates it only around that expression, even though prepare evaluations reuse one
/// hardened QuickJS context.
pub(crate) const THEME_SCOPE_BINDING: &str = "__valleMotionThemeScope";

/// One lexical slot lets module-level pure helpers call `useTheme()` while an expression is being
/// prepared. `__valleWithTheme` restores the prior value in `finally`, so a provider can never
/// leak into a sibling evaluation even after an exception.
const THEME_SCOPE_JS: &str = r#"
const __valleNoTheme = Symbol("valle:no-theme");
let __valleActiveTheme = __valleNoTheme;
const __valleFreezeTheme = (value) => {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) __valleFreezeTheme(child);
    Object.freeze(value);
  }
  return value;
};
const __valleWithTheme = (value, evaluate) => {
  const previous = __valleActiveTheme;
  __valleActiveTheme = __valleFreezeTheme(value);
  try { return evaluate(); }
  finally { __valleActiveTheme = previous; }
};
const useTheme = (...args) => {
  if (args.length !== 0) throw new Error("valle:compute:useTheme() takes no arguments");
  if (__valleActiveTheme === __valleNoTheme) {
    throw new Error("valle:compute:useTheme() requires an enclosing ThemeProvider");
  }
  return __valleActiveTheme;
};
"#;

/// Replace nondeterministic globals with throwing getters. Disable locale-sensitive operations,
/// GC-timing APIs, and dynamic code construction independently of QuickJS build features.
const HARDEN_JS: &str = r#"
(() => {
  const boom = (name) => { throw new Error("valle:forbidden:" + name); };
  const fnProto = Object.getPrototypeOf(function () {});
  const names = [
    "Date", "performance", "fetch", "XMLHttpRequest", "WebSocket",
    "setTimeout", "setInterval", "requestAnimationFrame", "queueMicrotask",
    "process", "require", "import", "Worker", "crypto", "navigator", "document", "window",
    "Intl", "WeakRef", "FinalizationRegistry", "eval", "Function",
  ];
  for (const k of names) {
    try {
      Object.defineProperty(globalThis, k, { get: () => boom(k), configurable: false });
    } catch (_) { /* The property is already absent. */ }
  }
  Math.random = () => boom("Math.random");
  for (const k of [
    "asin", "acos", "atan", "sinh", "cosh", "tanh",
    "asinh", "acosh", "atanh", "exp", "expm1", "log", "log1p", "log2", "log10",
    "cbrt", "hypot",
  ]) {
    Math[k] = () => boom("Math." + k);
  }
  try {
    Object.defineProperty(fnProto, "constructor", { get: () => boom("Function"), configurable: false });
  } catch (_) {}
  const die = (name) => function () { return boom(name); };
  String.prototype.localeCompare = die("localeCompare");
  for (const proto of [Object.prototype, String.prototype, Number.prototype, Array.prototype]) {
    try { proto.toLocaleString = die("toLocaleString"); } catch (_) {}
  }
})();
"#;

/// Names rejected before sandbox evaluation.
const FORBIDDEN_NAMES: &[&str] = &[
    "Date",
    "Math.random",
    "performance",
    "fetch",
    "XMLHttpRequest",
    "WebSocket",
    "setTimeout",
    "setInterval",
    "requestAnimationFrame",
    "queueMicrotask",
    "process",
    "require",
    "globalThis",
    "Worker",
    "crypto",
    "navigator",
    "document",
    "window",
    "eval",
    "Function",
    // Locale-sensitive operations and GC-timing APIs.
    "Intl",
    "toLocaleString",
    "localeCompare",
    "WeakRef",
    "FinalizationRegistry",
];

/// Reject host-dependent Math implementations without a deterministic Rust bridge. Bridged
/// operations share the static and runtime kernel; exponentiation remains an AST-level rejection.
const APPROXIMATED_MATH: &[&str] = &[
    "Math.asin",
    "Math.acos",
    "Math.atan",
    "Math.sinh",
    "Math.cosh",
    "Math.tanh",
    "Math.asinh",
    "Math.acosh",
    "Math.atanh",
    "Math.expm1",
    "Math.log",
    "Math.log1p",
    "Math.log2",
    "Math.log10",
    "Math.cbrt",
    "Math.hypot",
];

/// Return a diagnostic for a forbidden name matched at identifier boundaries.
pub fn scan_forbidden(src: &str, path: &str) -> Option<MotionDiagnostic> {
    for name in FORBIDDEN_NAMES {
        if contains_identifier(src, name) {
            return Some(MotionDiagnostic::new(
                DiagCode::SandboxForbidden,
                path,
                format!(
                    "`{name}` is forbidden in the deterministic compile-time sandbox \
                     (artifacts must be reproducible; use an explicit `seededRandom(seed)` \
                     primitive if you need randomness)"
                ),
            ));
        }
    }
    for name in APPROXIMATED_MATH {
        if contains_identifier(src, name) {
            return Some(MotionDiagnostic::new(
                DiagCode::SandboxForbidden,
                path,
                format!(
                    "`{name}` is implementation-approximated in ECMAScript and evaluates \
                     through the host libm — the same source would compile to different \
                     artifact bytes on different build hosts. Precompute the value outside \
                     the source, or express the curve with `interpolate` easings"
                ),
            ));
        }
    }
    None
}

fn contains_identifier(hay: &str, needle: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    // Ignore literal string contents while retaining expressions inside template interpolation.
    let masked = mask_string_literals(hay);
    let hay = masked.as_str();
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let start = from + i;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_word(hay[..start].chars().next_back().unwrap());
        let after_ok = end == hay.len() || !is_word(hay[end..].chars().next().unwrap());
        // Plain property names are not global references; dotted forbidden names still match as a
        // whole.
        let is_property =
            !needle.contains('.') && hay[..start].chars().next_back().is_some_and(|c| c == '.');
        if before_ok && after_ok && !is_property {
            return true;
        }
        from = end;
    }
    false
}

/// Mask literal text and comments while preserving offsets and template expressions. Track nested
/// strings, templates, and code braces separately. Leave uncertain syntax visible as possible
/// references; regular-expression literals are outside the supported grammar.
fn mask_string_literals(src: &str) -> String {
    /// A nested template frame: literal text or an interpolation expression.
    enum Frame {
        /// Template text up to the next backtick or interpolation opener.
        Text,
        /// Interpolation with code-brace depth, excluding braces inside nested strings.
        Interp(usize),
    }
    let mut out = String::with_capacity(src.len());
    let mut stack: Vec<Frame> = Vec::new();
    let mut chars = src.chars().peekable();

    while let Some(c) = chars.next() {
        // Mask template text until a closing backtick or interpolation opener.
        if matches!(stack.last(), Some(Frame::Text)) {
            match c {
                '`' => {
                    stack.pop();
                    out.push(c);
                }
                '$' if chars.peek() == Some(&'{') => {
                    out.push('$');
                    out.push(chars.next().unwrap_or('{'));
                    stack.push(Frame::Interp(1));
                }
                '\\' => {
                    // Mask both characters of an escape pair to preserve length.
                    out.push(' ');
                    if let Some(n) = chars.next() {
                        out.push(if n.is_whitespace() { n } else { ' ' });
                    }
                }
                _ => out.push(if c.is_whitespace() { c } else { ' ' }),
            }
            continue;
        }
        // Recognize strings, templates, and comments in code segments.
        match c {
            '\'' | '"' => {
                out.push(c);
                let quote = c;
                let mut escaped = false;
                for c in chars.by_ref() {
                    let is_end = !escaped && c == quote;
                    out.push(if is_end || c.is_whitespace() { c } else { ' ' });
                    escaped = !escaped && c == '\\';
                    if is_end {
                        break;
                    }
                }
            }
            '`' => {
                out.push(c);
                stack.push(Frame::Text);
            }
            '/' if chars.peek() == Some(&'/') => {
                // Mask line comments through the end of the line.
                out.push(' ');
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push(c);
                        break;
                    }
                    out.push(if c.is_whitespace() { c } else { ' ' });
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                // Mask block comments through the closing delimiter.
                out.push(' ');
                out.push(' ');
                chars.next();
                let mut star = false;
                for c in chars.by_ref() {
                    if star && c == '/' {
                        out.push(' ');
                        break;
                    }
                    star = c == '*';
                    out.push(if c.is_whitespace() { c } else { ' ' });
                }
            }
            '{' => {
                if let Some(Frame::Interp(d)) = stack.last_mut() {
                    *d += 1;
                }
                out.push(c);
            }
            '}' => {
                if let Some(Frame::Interp(d)) = stack.last_mut() {
                    *d -= 1;
                    if *d == 0 {
                        stack.pop();
                    }
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Marker distinguishing measurement errors from ordinary evaluation failures.
const MEASURE_MARK: &str = "valle:measure:";

/// Marker for explicit computation-builtin rejections that must reach the author unchanged.
const COMPUTE_MARK: &str = "valle:compute:";

/// Marker for invalid typed-value coercions, which must not be treated as failed constant folding.
const TYPED_MARK: &str = "valle:typed:";

/// Prepare-time intrinsic measurement environment. Reject empty font sets so successful
/// measurements always have meaningful font inputs.
pub struct MeasureEnv {
    fonts: std::rc::Rc<valle_motion::Fonts>,
    /// `None` until the entry file's delivery contract supplies the canvas. A host that compiles
    /// a source string with an explicit viewport binds it here; an entry compile takes it from
    /// `composition` so the canvas cannot drift from what the scene renders at.
    viewport: Option<valle_motion::Viewport>,
}

impl MeasureEnv {
    /// Require nonempty fonts and the same viewport used for rendering relative units.
    pub fn new(font_blobs: &[Vec<u8>], viewport: (u32, u32)) -> Result<Self, MotionDiagnostic> {
        Self::new_with_aliases(font_blobs, &[], viewport)
    }

    /// Project variant constructor. Aliases are the control URIs authors may name as a
    /// `measureText` font family, mapped to the exact bytes bound by that clip.
    pub fn new_with_aliases(
        font_blobs: &[Vec<u8>],
        aliases: &[(String, Vec<u8>)],
        viewport: (u32, u32),
    ) -> Result<Self, MotionDiagnostic> {
        let mut fonts = valle_motion::Fonts::default();
        if font_blobs.is_empty() {
            valle_motion::register_default_motion_fonts(&mut fonts).map_err(|error| {
                MotionDiagnostic::new(
                    DiagCode::StaticEvalFailed,
                    "module",
                    format!("cannot register measure font: {error}"),
                )
            })?;
        } else {
            for blob in font_blobs {
                fonts
                    .register(valle_motion::motion_font_resource(blob.clone()))
                    .map_err(|error| {
                        MotionDiagnostic::new(
                            DiagCode::StaticEvalFailed,
                            "module",
                            format!("cannot register measure font: {error}"),
                        )
                    })?;
            }
        }
        for (alias, blob) in aliases {
            fonts
                .register(valle_motion::FontResource::new(blob.clone()).override_info(
                    valle_motion::FontOverride {
                        family_name: Some(std::sync::Arc::<str>::from(alias.as_str())),
                        ..Default::default()
                    },
                ))
                .map_err(|error| {
                    MotionDiagnostic::new(
                        DiagCode::StaticEvalFailed,
                        "module",
                        format!("cannot register measure font `{alias}`: {error}"),
                    )
                })?;
        }
        Ok(MeasureEnv {
            fonts: std::rc::Rc::new(fonts),
            viewport: Some(valle_motion::Viewport::new(viewport)),
        })
    }

    /// Fonts without a canvas, for entry compiles that take the canvas from `composition`.
    ///
    /// Relative units inside `measureText` must use the canvas the scene renders at. The compiler
    /// binds it from the parsed contract before evaluating module-level constants and rejects an
    /// entry that measures text without a literal contract, so an unbound environment can never
    /// reach a measurement.
    pub fn new_unbound_with_aliases(
        font_blobs: &[Vec<u8>],
        aliases: &[(String, Vec<u8>)],
    ) -> Result<Self, MotionDiagnostic> {
        // Reuse the bound constructor for font registration, then drop the placeholder canvas.
        let mut env = Self::new_with_aliases(font_blobs, aliases, (1, 1))?;
        env.viewport = None;
        Ok(env)
    }

    /// Same fonts bound to the logical canvas of the entry file's delivery contract.
    pub fn bind_viewport(&self, viewport: (u32, u32)) -> Self {
        Self {
            fonts: self.fonts.clone(),
            viewport: Some(valle_motion::Viewport::new(viewport)),
        }
    }

    /// Whether a host or a previous bind already supplied the logical canvas.
    pub fn has_viewport(&self) -> bool {
        self.viewport.is_some()
    }
}

/// Deterministic sandbox that evaluates module-level static declarations once.
pub struct Sandbox {
    // Keep Runtime alive longer than Context by preserving field drop order.
    ctx: JsContext,
    _rt: JsRuntime,
}

impl Sandbox {
    /// Without a measurement environment, measureText is unavailable; the caller reports missing
    /// fonts.
    pub fn new(prelude: &str, measure: Option<&MeasureEnv>) -> Result<Sandbox, MotionDiagnostic> {
        let rt = JsRuntime::new().map_err(|e| eval_error("", format!("quickjs runtime: {e}")))?;
        let ctx =
            JsContext::full(&rt).map_err(|e| eval_error("", format!("quickjs context: {e}")))?;
        // An unbound canvas is not a failure here: an entry that never measures text must still be
        // able to report its real problem (a missing delivery contract) instead of a measurement
        // error. The call itself reports the missing canvas.
        let measure = measure.map(|env| (env.fonts.clone(), env.viewport));
        ctx.with(|ctx| -> Result<(), String> {
            run(&ctx, HARDEN_JS.as_bytes()).map(|_: Option<String>| ())?;
            // Install host functions before evaluating module-level constants that may call them.
            if let Some((fonts, viewport)) = measure {
                let host = rquickjs::Function::new(ctx.clone(), move |request: String| -> String {
                    match viewport {
                        Some(viewport) => measure_text_json(&request, &fonts, viewport),
                        None => serde_json::json!({
                            "error": "measureText needs the entry file's `composition` to bind the \
                                      logical canvas before module constants run"
                        })
                        .to_string(),
                    }
                })
                .map_err(|e| format!("cannot install measureText: {e}"))?;
                ctx.globals()
                    .set("__valle_measure_text", host)
                    .map_err(|e| format!("cannot bind measureText: {e}"))?;
                run(&ctx, MEASURE_JS.as_bytes()).map(|_: Option<String>| ())?;
            }
            // Pure computation builtins require no external inputs and are always available.
            let compute = rquickjs::Function::new(ctx.clone(), |request: String| -> String {
                valle_motion::compute::bridge::dispatch(&request)
            })
            .map_err(|e| format!("cannot install compute bridge: {e}"))?;
            ctx.globals()
                .set("__valle_compute", compute)
                .map_err(|e| format!("cannot bind compute bridge: {e}"))?;
            run(&ctx, COMPUTE_JS.as_bytes()).map(|_: Option<String>| ())?;
            let motion_builtin =
                rquickjs::Function::new(ctx.clone(), |request: String| -> String {
                    valle_motion::builtin::dispatch(&request)
                })
                .map_err(|e| format!("cannot install Motion builtin bridge: {e}"))?;
            ctx.globals()
                .set("__valle_motion_builtin", motion_builtin)
                .map_err(|e| format!("cannot bind Motion builtin bridge: {e}"))?;
            run(&ctx, MOTION_BUILTIN_JS.as_bytes()).map(|_: Option<String>| ())?;
            run(&ctx, THEME_SCOPE_JS.as_bytes()).map(|_: Option<String>| ())?;
            if !prelude.trim().is_empty() {
                run(&ctx, prelude.as_bytes()).map(|_: Option<String>| ())?;
            }
            Ok(())
        })
        .map_err(|m| classify(&m, "prelude"))?;
        Ok(Sandbox { ctx, _rt: rt })
    }

    /// Evaluate a static expression with temporary constant bindings. Reject functions, symbols,
    /// cycles, and other non-JSON results.
    pub fn eval_json(
        &self,
        expr_src: &str,
        bindings: &[(String, serde_json::Value)],
        path: &str,
    ) -> Result<serde_json::Value, MotionDiagnostic> {
        if let Some(d) = scan_forbidden(expr_src, path) {
            return Err(d);
        }
        let mut program = String::from("(() => {\n");
        let has_theme_scope = bindings.iter().any(|(name, _)| name == THEME_SCOPE_BINDING);
        for (name, value) in bindings {
            program.push_str(&rebind(name, value));
        }
        if has_theme_scope {
            program.push_str("const __valleStaticValue = __valleWithTheme(");
            program.push_str(THEME_SCOPE_BINDING);
            program.push_str(", () => (");
        } else {
            program.push_str("const __valleStaticValue = (");
        }
        program.push_str(expr_src);
        if has_theme_scope {
            program.push_str("));\n");
        } else {
            program.push_str(");\n");
        }
        program.push_str("return JSON.stringify({ value: __valleStaticValue, negativeZero: typeof __valleStaticValue === \"number\" && Object.is(__valleStaticValue, -0) });\n})()");

        let json: String = self
            .ctx
            .with(|ctx| run::<Option<String>>(&ctx, program.as_bytes()))
            .map_err(|m| classify(&m, path))?
            .ok_or_else(|| {
                MotionDiagnostic::new(
                    DiagCode::StaticEvalFailed,
                    path,
                    "static expression is not JSON-representable (function / undefined / symbol?)",
                )
            })?;
        let mut envelope: serde_json::Value =
            serde_json::from_str(&json).map_err(|e| eval_error(path, e.to_string()))?;
        let object = envelope.as_object_mut().ok_or_else(|| {
            eval_error(path, "static evaluator returned a malformed value envelope")
        })?;
        let negative_zero = object
            .get("negativeZero")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let value = object.remove("value").ok_or_else(|| {
            MotionDiagnostic::new(
                DiagCode::StaticEvalFailed,
                path,
                "static expression is not JSON-representable (function / undefined / symbol?)",
            )
        })?;
        if negative_zero {
            Ok(serde_json::json!(-0.0_f64))
        } else {
            Ok(value)
        }
    }
}

/// Allowlist constructors when rebuilding scale objects from serialized data.
const SCALE_CONSTRUCTORS: &[&str] = &[
    "scaleLinear",
    "scaleBand",
    "scalePoint",
    "scaleSequential",
    "scaleQuantize",
];

/// Rebind a value as a JS const. Reconstruct scale methods through allowlisted constructors. Inject
/// values through JSON.parse so __proto__ stays an own property rather than changing the object
/// prototype.
fn rebind(name: &str, value: &serde_json::Value) -> String {
    let constructor = value
        .get("__valleScale")
        .and_then(serde_json::Value::as_str)
        .filter(|kind| SCALE_CONSTRUCTORS.contains(kind));
    // Encode JSON text as a JSON string literal before passing it to JSON.parse.
    let payload = serde_json::Value::String(value.to_string());
    match constructor {
        Some(constructor) => format!("const {name} = {constructor}(JSON.parse({payload}));\n"),
        None => format!("const {name} = JSON.parse({payload});\n"),
    }
}

/// Extract QuickJS's pending exception message, including diagnostic markers.
fn run<'js, T: rquickjs::FromJs<'js>>(ctx: &rquickjs::Ctx<'js>, src: &[u8]) -> Result<T, String> {
    match ctx.eval::<T, _>(src) {
        Ok(v) => Ok(v),
        Err(rquickjs::Error::Exception) => {
            let v = ctx.catch();
            let msg = v
                .clone()
                .into_exception()
                .and_then(|e| e.message())
                .unwrap_or_else(|| format!("{v:?}"));
            Err(msg)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Expose measureText through a JSON string-to-string bridge.
///
/// The wrapper validates the option object before the host sees it: a removed or misspelled key must
/// name the replacement the author should write, and the options are limited to the typography an
/// authored `style` object accepts.
const MEASURE_JS: &str = r#"
(() => {
const OPTIONS = ["fontSize", "fontFamily", "fontWeight", "letterSpacing", "lineHeight", "maxWidth"];
const REMOVED = ["className", "style"];
const fail = (message) => { throw new Error("valle:measure:" + message); };
const show = (value) => (typeof value === "number" && !Number.isFinite(value) ? String(value) : JSON.stringify(value));
const finite = (name, value, positive) => {
  if (typeof value !== "number" || !Number.isFinite(value) || (positive && value <= 0)) {
    fail(name + " must be a finite" + (positive ? " positive" : "") + " number, got " + show(value));
  }
};
globalThis.measureText = (text, options = {}) => {
  if (typeof text !== "string") {
    fail("measureText(text, options) needs a string");
  }
  if (options === null || typeof options !== "object" || Array.isArray(options)) {
    fail("measureText options must be an object");
  }
  for (const key of Object.keys(options)) {
    if (REMOVED.includes(key)) {
      fail("measureText does not accept `" + key + "`; pass explicit typography instead: " +
        "{ fontSize, fontFamily, fontWeight, letterSpacing, lineHeight }");
    }
    if (!OPTIONS.includes(key)) {
      fail("unknown option `" + key + "`; measureText accepts " + OPTIONS.join(", "));
    }
  }
  finite("fontSize", options.fontSize, true);
  if (options.fontFamily !== undefined && options.fontFamily !== null
      && typeof options.fontFamily !== "string") {
    fail("fontFamily must be a string");
  }
  if (options.fontWeight !== undefined && options.fontWeight !== null) {
    finite("fontWeight", options.fontWeight, true);
  }
  if (options.letterSpacing !== undefined && options.letterSpacing !== null) {
    finite("letterSpacing", options.letterSpacing, false);
  }
  if (options.lineHeight !== undefined && options.lineHeight !== null) {
    finite("lineHeight", options.lineHeight, false);
  }
  const maxWidth = options.maxWidth ?? null;
  if (maxWidth !== null) {
    finite("maxWidth", maxWidth, true);
  }
  const raw = __valle_measure_text(JSON.stringify({
    text,
    fontSize: options.fontSize,
    fontFamily: options.fontFamily ?? null,
    fontWeight: options.fontWeight ?? null,
    letterSpacing: options.letterSpacing ?? null,
    lineHeight: options.lineHeight ?? null,
    maxWidth,
  }));
  const result = JSON.parse(raw);
  if (result.error) { throw new Error("valle:measure:" + result.error); }
  return result;
};
})();
"#;

/// JavaScript wrappers marshal data to Rust rather than duplicating computation algorithms. Scale
/// objects retain their parameters and delegate method calls to the same Rust implementation.
const COMPUTE_JS: &str = r#"
(() => {
const call = (op, args) => {
  const result = JSON.parse(__valle_compute(JSON.stringify({ op, args })));
  if (result.error) { throw new Error("valle:compute:" + result.error); }
  return result.ok;
};

globalThis.ticks = (start, stop, count) => call("ticks", { start, stop, count });
globalThis.niceDomain = (start, stop, count) => call("niceDomain", { start, stop, count });
globalThis.extent = (values) => call("extent", { values });
globalThis.extentOrDefault = (min, max) => call("extentOrDefault", { start: min, stop: max });
globalThis.stack = (values, options = {}) => call("stack", {
  values,
  offset: options.offset ?? "zero",
});

globalThis.scaleLinear = (options = {}) => {
  const domain = options.domain;
  const range = options.range;
  return {
    __valleScale: "scaleLinear",
    domain, range,
    map: (value) => call("linear.map", { domain, range, value }),
    invert: (value) => call("linear.invert", { domain, range, value }),
    ticks: (count) => call("ticks", { domain, count }),
  };
};

globalThis.scaleBand = (options = {}) => {
  const base = {
    range: options.range,
    count: options.count,
    paddingInner: options.paddingInner ?? 0,
    paddingOuter: options.paddingOuter ?? 0,
    align: options.align ?? 0.5,
  };
  return {
    __valleScale: "scaleBand",
    ...base,
    band: (index) => call("band.band", { ...base, index }),
    center: (index) => {
      const [start, end] = call("band.band", { ...base, index });
      return (start + end) / 2;
    },
    step: () => call("band.step", base),
    bandwidth: () => call("band.bandwidth", base),
  };
};

globalThis.scalePoint = (options = {}) => {
  const base = { range: options.range, count: options.count };
  return {
    __valleScale: "scalePoint",
    ...base,
    at: (index) => call("point.at", { ...base, index }),
    step: () => call("point.step", base),
  };
};

globalThis.scaleSequential = (options = {}) => {
  const domain = options.domain;
  const colors = options.colors;
  return {
    __valleScale: "scaleSequential",
    domain, colors,
    map: (value) => call("sequential.map", { domain, colors, value }),
  };
};
globalThis.scaleQuantize = (options = {}) => {
  const domain = options.domain;
  const colors = options.colors;
  return {
    __valleScale: "scaleQuantize",
    domain, colors,
    map: (value) => call("quantize.map", { domain, colors, value }),
  };
};

globalThis.pie = (values, options = {}) => call("pie", {
  values,
  startAngle: options.startAngle ?? 0,
  endAngle: options.endAngle ?? globalThis.TAU,
  padAngle: options.padAngle ?? 0,
});
globalThis.curve = (points, options = {}) => {
  const payload = (points ?? []).map((point) => {
    if (point && typeof point === "object" && "x" in point && "y" in point) {
      return [point.x, point.y];
    }
    if (Array.isArray(point) && point.length === 2) {
      return point;
    }
    throw new Error("valle:compute:curve points must be Point values");
  });
  return __typed({
    __valleType: "pathData",
    d: call("curve", { points: payload, type: options.type ?? "monotoneX" }),
  });
};

// Project the entire coordinate batch together so all points share projection parameters and
// canvas fitting.
globalThis.geoProject = (options = {}) => call("geo.project", {
  points: options.points,
  projection: options.projection ?? "albers",
  width: options.width,
  height: options.height,
  padding: options.padding ?? 0.04,
});
globalThis.geoPath = (options = {}) => call("geo.path", {
  polygons: options.polygons,
  projection: options.projection ?? "albers",
  width: options.width,
  height: options.height,
  padding: options.padding ?? 0.04,
  clip: options.clip ?? null,
});
globalThis.placeLabels = (candidates, options = {}) => call("label.place", {
  candidates,
  bounds: options.bounds,
  padding: options.padding ?? 4,
});

// Lay out a directed graph, returning ranks, rows, backEdges, and node centers.
globalThis.graphLayout = (options = {}) => call("graph.layout", {
  sizes: options.sizes,
  edges: options.edges,
  nodeGap: options.nodeGap ?? 40,
  rankGap: options.rankGap ?? 64,
});

// Explicitly seeded random values and noise; unseeded Math.random is unavailable in the sandbox.
globalThis.seededRandom = (seed, count, range) => call("random", { seed, count, range });
globalThis.noise1d = (seed, x) => call("noise1d", { seed, x });
globalThis.noise2d = (seed, x, y) => call("noise2d", { seed, x, y });
})();
"#;

/// Deterministic math and formatting: JavaScript marshals arguments; Rust owns every operation.
const MOTION_BUILTIN_JS: &str = r#"
(() => {
const call = (op, args) => {
  const result = JSON.parse(__valle_motion_builtin(JSON.stringify({ op, args })));
  if (result.error) { throw new Error("valle:compute:" + result.error); }
  return result.ok;
};
for (const op of ["sin", "cos", "floor", "ceil", "round", "fract"]) {
  globalThis[op] = (value) => call(op, { value });
}
for (const op of ["atan2", "pow", "mod", "pingPong"]) {
  globalThis[op] = (lhs, rhs) => call(op, { lhs, rhs });
}
Math.sqrt = (value) => call("sqrt", { value });
Math.sin = globalThis.sin;
Math.cos = globalThis.cos;
Math.tan = (value) => call("tan", { value });
Math.atan2 = globalThis.atan2;
Math.pow = globalThis.pow;
Math.floor = globalThis.floor;
Math.ceil = globalThis.ceil;
Math.round = globalThis.round;
Math.trunc = (value) => call("trunc", { value });
// Deterministic authoring aliases. These constants use only ECMAScript-specified IEEE-754 arithmetic;
// frame-time lowering uses the same literal factors and typed arithmetic nodes.
globalThis.TAU = 6.283185307179586;
globalThis.deg = (value) => value * 0.017453292519943295;
globalThis.rad = (value) => value;
globalThis.noise1d = (seed, x) => call("noise1d", { seed, x });
globalThis.noise2d = (seed, x, y) => call("noise2d", { seed, x, y });
globalThis.formatNumber = (value, options = {}) => call("format", {
  value,
  format: { kind: "number", decimals: options.decimals ?? 0, grouping: options.grouping ?? false },
});
globalThis.formatPercent = (value, options = {}) => call("format", {
  value,
  format: { kind: "percent", decimals: options.decimals ?? 0, grouping: options.grouping ?? false },
});
globalThis.padNumber = (value, options = {}) => call("format", {
  value,
  format: { kind: "pad", width: options.width },
});
})();
"#;

/// Detect measureText before creating the sandbox to report missing fonts directly.
pub fn mentions_measure(src: &str) -> bool {
    contains_identifier(src, "measureText")
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MeasureRequestJson {
    text: String,
    font_size: f64,
    font_family: Option<String>,
    font_weight: Option<f64>,
    letter_spacing: Option<f64>,
    line_height: Option<f64>,
    max_width: Option<f64>,
}

/// Return measurement failures as JSON errors for marked JS exceptions; never panic across the
/// QuickJS boundary.
fn measure_text_json(
    request: &str,
    fonts: &valle_motion::Fonts,
    viewport: valle_motion::Viewport,
) -> String {
    let fail = |message: String| {
        serde_json::json!({ "error": message })
            .to_string()
            .replace('\n', " ")
    };
    let request: MeasureRequestJson = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => return fail(format!("bad measureText options: {error}")),
    };
    match valle_motion::measure_text(
        &valle_motion::TextMeasure {
            text: &request.text,
            font_size: request.font_size,
            font_family: request.font_family.as_deref(),
            font_weight: request.font_weight,
            letter_spacing: request.letter_spacing,
            line_height: request.line_height,
            max_width: request.max_width,
        },
        fonts,
        viewport,
    ) {
        Ok(measured) => serde_json::json!({
            "width": measured.width,
            "height": measured.height,
        })
        .to_string(),
        Err(error) => fail(error.to_string()),
    }
}

fn eval_error(path: &str, message: impl Into<String>) -> MotionDiagnostic {
    MotionDiagnostic::new(DiagCode::StaticEvalFailed, path, message)
}

/// Classify sandbox exceptions into actionable diagnostic categories.
fn classify(message: &str, path: &str) -> MotionDiagnostic {
    if let Some(i) = message.find(COMPUTE_MARK) {
        return MotionDiagnostic::new(
            DiagCode::BuiltinRejected,
            path,
            message[i + COMPUTE_MARK.len()..].replace('\n', " "),
        );
    }
    if let Some(i) = message.find(TYPED_MARK) {
        return MotionDiagnostic::new(
            DiagCode::BuiltinRejected,
            path,
            message[i + TYPED_MARK.len()..].replace('\n', " "),
        );
    }
    if let Some(i) = message.find(MEASURE_MARK) {
        return MotionDiagnostic::new(
            DiagCode::BuiltinRejected,
            path,
            message[i + MEASURE_MARK.len()..].replace('\n', " "),
        );
    }
    match message.find(FORBIDDEN_MARK) {
        Some(i) => {
            let name: String = message[i + FORBIDDEN_MARK.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
                .collect();
            MotionDiagnostic::new(
                DiagCode::SandboxForbidden,
                path,
                format!("`{name}` is forbidden in the deterministic compile-time sandbox"),
            )
        }
        None => eval_error(path, message.replace('\n', " ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_matching_does_not_fire_on_substrings() {
        assert!(contains_identifier("const x = Date.now()", "Date"));
        assert!(!contains_identifier("const myDate = 1", "Date"));
        assert!(!contains_identifier("const updated = 1", "Date"));
        assert!(contains_identifier("Math.random()", "Math.random"));
        assert!(!contains_identifier("MathRandomish()", "Math.random"));
    }

    /// Verify masking preserves real references around nested template strings and comments.
    #[test]
    fn masking_survives_interpolation_comments_and_braces() {
        // Strings inside interpolation are data, not identifier references.
        assert!(!contains_identifier("`${x === \"Date\" ? 1 : 2}`", "Date"));
        // String braces must not change interpolation nesting or hide later references.
        assert!(contains_identifier("`${a(\"{\") }` + Date.now()", "Date"));
        // Apostrophes in comments must not begin strings.
        assert!(contains_identifier("/* don't */ ctx.frame", "ctx"));
        assert!(contains_identifier("// isn't\nctx.frame", "ctx"));
        // Identifiers inside comments are inert.
        assert!(!contains_identifier("/* Date */ 1 + 1", "Date"));
        // Template text is data; interpolation expressions remain visible.
        assert!(!contains_identifier("`Date`", "Date"));
        assert!(contains_identifier("`${Date.now()}`", "Date"));
    }

    #[test]
    fn plain_static_expressions_evaluate() {
        let s = Sandbox::new("const BASE = 10; function twice(x) { return x * 2; }", None).unwrap();
        assert_eq!(
            s.eval_json("twice(BASE) + 1", &[], "t").unwrap(),
            serde_json::json!(21)
        );
        assert_eq!(
            s.eval_json("[1, 2, 3].map(x => x * BASE)", &[], "t")
                .unwrap(),
            serde_json::json!([10, 20, 30])
        );
    }

    #[test]
    fn bindings_are_scoped_to_one_call() {
        let s = Sandbox::new("", None).unwrap();
        let b = vec![("index".into(), serde_json::json!(2))];
        assert_eq!(
            s.eval_json("index * 3", &b, "t").unwrap(),
            serde_json::json!(6)
        );
        // Temporary bindings must not leak into later evaluations.
        assert!(s.eval_json("index", &[], "t").is_err());
    }

    #[test]
    fn theme_scope_reaches_module_helpers_and_is_restored_after_errors() {
        let s = Sandbox::new("const themedAccent = () => useTheme().colors.accent;", None).unwrap();
        let theme = vec![(
            THEME_SCOPE_BINDING.into(),
            serde_json::json!({ "colors": { "accent": "#38bdf8" } }),
        )];
        assert_eq!(
            s.eval_json("themedAccent()", &theme, "theme").unwrap(),
            serde_json::json!("#38bdf8")
        );
        assert!(
            s.eval_json("(() => { throw new Error('boom'); })()", &theme, "theme")
                .is_err()
        );
        let error = s.eval_json("useTheme()", &[], "theme").unwrap_err();
        assert_eq!(error.code, DiagCode::BuiltinRejected);
        assert!(error.message.contains("ThemeProvider"), "{error}");
    }

    /// Reject forbidden references during lexical scanning.
    #[test]
    fn forbidden_names_are_rejected_lexically() {
        let s = Sandbox::new("", None).unwrap();
        for src in [
            "Date.now()",
            "new Date().getTime()",
            "Math.random()",
            "performance.now()",
            "fetch('x')",
            "process.env.HOME",
        ] {
            let e = s.eval_json(src, &[], "t").unwrap_err();
            assert_eq!(e.code, DiagCode::SandboxForbidden, "{src}: {e}");
        }
    }

    #[test]
    fn forbidden_global_names_are_allowed_as_member_properties() {
        assert!(scan_forbidden("props.process", "t").is_none());
        assert!(scan_forbidden("props.window", "t").is_none());
        assert!(scan_forbidden("process.env", "t").is_some());
        assert!(scan_forbidden("Math.random()", "t").is_some());
    }

    /// Reject unbridged approximate Math operations while allowing deterministic bridged and
    /// precisely specified members.
    #[test]
    fn approximated_math_is_rejected_but_exactly_specified_math_folds() {
        for src in ["Math.log2(8)", "Math.asin(0.5)"] {
            let d = scan_forbidden(src, "t").expect(src);
            assert!(d.message.contains("implementation-approximated"), "{src}");
        }
        for src in [
            "Math.sin(1)",
            "Math.cos(1)",
            "Math.tan(1)",
            "Math.atan2(1, 2)",
            "Math.pow(2, 0.5)",
            "Math.sqrt(2)",
            "Math.floor(1.5)",
            "Math.PI",
            "Math.max(1, 2)",
        ] {
            assert!(scan_forbidden(src, "t").is_none(), "{src} must stay legal");
        }
        // Runtime guards catch indirect access that bypasses lexical scanning.
        let s = Sandbox::new("", None).unwrap();
        let e = s.eval_json("Math['l' + 'og'](1)", &[], "t").unwrap_err();
        assert_eq!(e.code, DiagCode::SandboxForbidden, "{e}");
    }

    /// Test runtime hardening through a prelude that bypasses lexical scanning.
    #[test]
    fn runtime_hardening_catches_what_lexing_never_saw() {
        let Err(e) = Sandbox::new("const STAMP = Date.now();", None) else {
            panic!("Date must be forbidden")
        };
        assert_eq!(e.code, DiagCode::SandboxForbidden, "{e}");
        assert!(e.message.contains("Date"), "{e}");

        let Err(e) = Sandbox::new("const R = Math.random();", None) else {
            panic!("Math.random must be forbidden")
        };
        assert_eq!(e.code, DiagCode::SandboxForbidden, "{e}");
        assert!(e.message.contains("Math.random"), "{e}");
    }

    /// Locale-sensitive methods remain blocked regardless of QuickJS's ICU configuration.
    #[test]
    fn locale_sensitive_apis_are_hardened_at_runtime() {
        for src in [
            "const S = ['b','a'].sort((x,y) => x.localeCompare(y)).join('');",
            "const N = (3.14159).toLocaleString();",
            "const A = [1,2].toLocaleString();",
        ] {
            let Err(e) = Sandbox::new(src, None) else {
                panic!("locale API must be forbidden: {src}")
            };
            assert_eq!(e.code, DiagCode::SandboxForbidden, "{src}: {e}");
        }
    }

    /// Dynamic Function construction is blocked even through a function's constructor property.
    #[test]
    fn function_constructor_is_hardened_at_runtime() {
        let Err(e) = Sandbox::new(
            "const X = (function(){}).constructor('return 6*7')();",
            None,
        ) else {
            panic!("Function constructor must be forbidden")
        };
        assert_eq!(e.code, DiagCode::SandboxForbidden, "{e}");
        assert!(e.message.contains("Function"), "{e}");
    }

    #[test]
    fn non_json_results_are_a_failure_not_a_null() {
        let s = Sandbox::new("", None).unwrap();
        assert_eq!(
            s.eval_json("(x) => x", &[], "t").unwrap_err().code,
            DiagCode::StaticEvalFailed
        );
        assert_eq!(
            s.eval_json("undefined", &[], "t").unwrap_err().code,
            DiagCode::StaticEvalFailed
        );
    }

    #[test]
    fn thrown_errors_surface_as_static_eval_failures() {
        let s = Sandbox::new("", None).unwrap();
        let e = s.eval_json("nope.nope", &[], "t").unwrap_err();
        assert_eq!(e.code, DiagCode::StaticEvalFailed);
    }
}
