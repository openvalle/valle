// Tracing runtime of the Motion frontend.
//
// A scene runs once per variant with traced inputs: the clock (`seconds`, `progress`). Every
// operation a traced value reaches records an op of the frame program, under the name the engine
// evaluates it by (`front/lower.rs`), and the host computes the op's value with the engine's own
// evaluator, so a value seen while tracing is bit for bit the value a frame computes. `Math`
// functions also go through the host (libm), so a result is the same on every platform.
//
// Where a traced value decides structure (an `if`, a loop test, a `switch`, an index, the arms of
// a conditional that are elements), a guard records the value it had and only the branch taken
// exists in the variant, as in React. A frame whose guards fail is traced again. A traced value
// that reaches code that cannot trace it (a coercion, a native method) gets an exact-value guard
// and is reported as an escape.
//
// Modules are factories, instantiated afresh for every trace, so module-level state cannot carry
// from one variant into another.
//
// After this file the host declares the scene's free names (`__declareFree`), defines its modules
// (`__define`) and calls `__lockdown()`: the host object and the setup entries go, `eval` and the
// function constructors throw, the absent globals (`Date`, `Promise`, …) throw with their
// replacement when read, and every built-in object is frozen, so nothing outlives a trace.

"use strict";

(function () {
  const host = globalThis.__host;
  const { defineProperty, entries, freeze, getOwnPropertyDescriptors, getPrototypeOf, keys } =
    Object;
  const isArray = Array.isArray;
  const nativeFrom = Array.from;
  const stringify = JSON.stringify;
  const NativeError = Error;
  const NativeNumber = Number;

  // What one variant may hold; past these a trace fails. An op takes about 300 bytes of script
  // memory while tracing, so the op limit stays well inside the memory limit.
  const MAX_OPS = 250000;
  const MAX_NODES = 100000;
  const MAX_GUARDS = 10000;

  let G = null;

  // ------------------------------------------------------------------------------ traced values

  function T(op, args, v) {
    this.op = op;
    this.args = freeze(args);
    this.v = v;
    freeze(this);
  }
  const isT = (x) => x instanceof T;
  const val = (x) => (x instanceof T ? x.v : x);

  function mk(op, args) {
    if (++G.ops > MAX_OPS) {
      throw new RangeError(
        "a frame has more than " +
          MAX_OPS +
          " values that depend on the time, such as from a loop that adds to one every turn",
      );
    }
    return new T(op, args, host.op(op, args.map(val)));
  }

  const plain = (x) => {
    if (x === null || typeof x !== "object" || isArray(x)) return false;
    const p = getPrototypeOf(x);
    return p === Object.prototype || p === null;
  };

  function hasT(x, d = 0) {
    if (x instanceof T) return true;
    if (d > 6 || x === null || typeof x !== "object" || x.__el) return false;
    if (isArray(x)) return x.some((y) => hasT(y, d + 1));
    if (!plain(x)) return false;
    for (const k in x) if (hasT(x[k], d + 1)) return true;
    return false;
  }

  // ------------------------------------------------------------------------------ guards

  // The innermost line of a scene module on the stack, as `m3.js:12`.
  function site() {
    const m = /\b(m\d+\.js):(\d+)/.exec(new NativeError().stack || "");
    return m ? m[1] + ":" + m[2] : "?";
  }

  function guard(t, want, kind, at) {
    if (G.guards.length >= MAX_GUARDS) {
      throw new RangeError(
        "a frame decides on the time more than " +
          MAX_GUARDS +
          " times, such as in a loop that runs seconds × 1000 turns",
      );
    }
    G.guards.push({ t, want, when: G.when.slice(), kind, at });
  }

  const truthy = (v) => !!v;

  // The concrete value of `t`, valid while `t` keeps it.
  function exact(t, kind, at) {
    guard(t, { eq: t.v }, kind, at);
    return t.v;
  }
  const escape = (x) => (x instanceof T ? exact(x, "escape", site()) : x);

  T.prototype = {
    [Symbol.toPrimitive]() {
      return exact(this, "escape", site());
    },
    toJSON() {
      return exact(this, "escape", site());
    },
  };

  // ------------------------------------------------------------------------------ operators

  const BIN = {
    "+": (a, b) => a + b,
    "-": (a, b) => a - b,
    "*": (a, b) => a * b,
    "/": (a, b) => a / b,
    "%": (a, b) => a % b,
    "**": (a, b) => host.op("pow", [NativeNumber(a), NativeNumber(b)]),
    "<": (a, b) => a < b,
    ">": (a, b) => a > b,
    "<=": (a, b) => a <= b,
    ">=": (a, b) => a >= b,
    "==": (a, b) => a == b,
    "!=": (a, b) => a != b,
    "===": (a, b) => a === b,
    "!==": (a, b) => a !== b,
    "&": (a, b) => a & b,
    "|": (a, b) => a | b,
    "^": (a, b) => a ^ b,
    "<<": (a, b) => a << b,
    ">>": (a, b) => a >> b,
    ">>>": (a, b) => a >>> b,
  };
  const UN = {
    "-": (a) => -a,
    "+": (a) => +a,
    "!": (a) => !a,
    "~": (a) => ~a,
    typeof: (a) => typeof a,
  };

  const ARITH = { "+": "add", "-": "sub", "*": "mul", "/": "div", "%": "rem", "**": "pow" };
  const COMPARE = {
    "<": "lt",
    "<=": "le",
    ">": "gt",
    ">=": "ge",
    "==": "eq",
    "===": "eq",
    "!=": "ne",
    "!==": "ne",
  };
  const numeric = (x) => typeof val(x) === "number";

  function __b(op, a, b) {
    if (!(a instanceof T) && !(b instanceof T)) {
      // BigInt exponents stay with the engine's operator.
      if (op === "**" && (typeof a === "bigint" || typeof b === "bigint")) return a ** b;
      return BIN[op](a, b);
    }
    if (op === "+" && (typeof val(a) === "string" || typeof val(b) === "string")) {
      return template([a, b]);
    }
    if (numeric(a) && numeric(b)) {
      if (op in ARITH) return mk(ARITH[op], [a, b]);
      if (op in COMPARE) return mk(COMPARE[op], [a, b]);
    }
    return BIN[op](escape(a), escape(b));
  }

  function __u(op, a) {
    if (!(a instanceof T)) return UN[op](a);
    if (op === "typeof") return typeof a.v;
    if (op === "-" && numeric(a)) return mk("neg", [a]);
    if (op === "+" && numeric(a)) return a;
    if (op === "!") return mk("select", [a, false, true]);
    return UN[op](escape(a));
  }

  function __if(c, at) {
    if (!(c instanceof T)) return c;
    const want = truthy(c.v);
    guard(c, { truthy: want }, "if", at);
    return want;
  }

  function __sw(d, at) {
    return d instanceof T ? exact(d, "switch", at) : d;
  }

  // ------------------------------------------------------------------------------ conditionals

  const FAIL = { fail: true };
  const scalar = (x) =>
    x instanceof T || typeof x === "number" || typeof x === "string" || typeof x === "boolean";

  // A value that picks `a` or `b` by `c`; objects and arrays of one shape pick field by field.
  function selectValue(c, a, b, d = 0) {
    if (a === b) return a;
    if (scalar(a) && scalar(b) && typeof val(a) === typeof val(b)) return mk("select", [c, a, b]);
    if (d < 4 && isArray(a) && isArray(b) && a.length === b.length) {
      const out = [];
      for (let i = 0; i < a.length; i++) {
        const r = selectValue(c, a[i], b[i], d + 1);
        if (r === FAIL) return FAIL;
        out.push(r);
      }
      return out;
    }
    if (d < 4 && plain(a) && plain(b) && !a.__el && !b.__el) {
      const ka = keys(a);
      if (ka.length !== keys(b).length || ka.some((k) => !(k in b))) return FAIL;
      const out = {};
      for (const k of ka) {
        const r = selectValue(c, a[k], b[k], d + 1);
        if (r === FAIL) return FAIL;
        out[k] = r;
      }
      return out;
    }
    return FAIL;
  }

  // Evaluates both arms, each under the condition that selects it.
  function arms(c, ta, tb) {
    const m = { g: G.guards.length, a: undefined, b: undefined, err: false };
    for (const [side, thunk] of [
      [true, ta],
      [false, tb],
    ]) {
      G.when.push([c, side]);
      try {
        const r = thunk();
        if (side) m.a = r;
        else m.b = r;
      } catch {
        m.err = true;
      } finally {
        G.when.pop();
      }
    }
    return m;
  }

  // The value `pick` makes of both arms; otherwise the arms' guards are dropped, `c` is guarded
  // and null tells the caller to take the branch `c` selects.
  function decide(c, at, m, pick) {
    if (!m.err) {
      const r = pick(m);
      if (r !== FAIL) return { r };
    }
    G.guards.length = m.g;
    guard(c, { truthy: truthy(c.v) }, "select", at);
    return null;
  }

  function __sel(c, ta, tb, at) {
    if (!(c instanceof T)) return c ? ta() : tb();
    const d = decide(c, at, arms(c, ta, tb), (m) => selectValue(c, m.a, m.b));
    if (d) return d.r;
    return truthy(c.v) ? ta() : tb();
  }

  function __and(a, tb, at) {
    if (!(a instanceof T)) return a ? tb() : a;
    const d = decide(
      a,
      at,
      arms(a, tb, () => a),
      (m) => (scalar(m.a) ? selectValue(a, m.a, a) : FAIL),
    );
    if (d) return d.r;
    return truthy(a.v) ? tb() : a.v;
  }

  function __or(a, tb, at) {
    if (!(a instanceof T)) return a ? a : tb();
    const d = decide(
      a,
      at,
      arms(a, () => a, tb),
      (m) => (scalar(m.b) ? selectValue(a, a, m.b) : FAIL),
    );
    if (d) return d.r;
    return truthy(a.v) ? a.v : tb();
  }

  // A traced value is a number, string or boolean, never nullish.
  function __nc(a, tb) {
    return a instanceof T ? a : (a ?? tb());
  }

  // ------------------------------------------------------------------------------ templates

  function template(parts) {
    const out = [];
    let text = "";
    for (const p of parts) {
      if (p instanceof T) {
        if (text) out.push({ text });
        text = "";
        out.push(p);
      } else {
        text += String(p);
      }
    }
    if (text) out.push({ text });
    return mk("template", out);
  }

  function __tpl(strings, ...vals) {
    if (!vals.some(hasT)) {
      let s = strings[0];
      for (let i = 0; i < vals.length; i++) s += String(vals[i]) + strings[i + 1];
      return s;
    }
    const parts = [strings[0]];
    for (let i = 0; i < vals.length; i++) parts.push(escapeDeep(vals[i]), strings[i + 1]);
    return template(parts);
  }

  // A traced value inside an object or array cannot be printed by a template.
  const escapeDeep = (x) => (x instanceof T || !hasT(x) ? x : String(escapeAll(x)));
  function escapeAll(x, d = 0) {
    if (x instanceof T) return escape(x);
    if (d > 6 || x === null || typeof x !== "object") return x;
    if (isArray(x)) return x.map((y) => escapeAll(y, d + 1));
    if (!plain(x)) return x;
    const out = {};
    for (const k in x) out[k] = escapeAll(x[k], d + 1);
    return out;
  }

  // ------------------------------------------------------------------------------ members, calls

  function __get(o, k) {
    return o instanceof T ? escape(o)[k] : o[k];
  }

  function __idx(o, k, at) {
    if (o instanceof T) return escape(o)[escape(k)];
    if (k instanceof T) return o[exact(k, "index", at)];
    return o[k];
  }

  // `Math`'s functions, through the host; traced when an argument is.
  const MATH = [
    "abs",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "cbrt",
    "ceil",
    "clz32",
    "cos",
    "cosh",
    "exp",
    "expm1",
    "floor",
    "fround",
    "hypot",
    "imul",
    "log",
    "log10",
    "log1p",
    "log2",
    "max",
    "min",
    "pow",
    "round",
    "sign",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
    "trunc",
  ];
  function math(name, args) {
    if (args.some(isT) && args.every(numeric)) return mk("math|" + name, args);
    return host.op(
      "math|" + name,
      args.map((a) => NativeNumber(a)),
    );
  }

  // Array methods that test what their callback returns.
  const TESTS = new Set([
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "filter",
    "some",
    "every",
  ]);

  function __call(o, name, at, ...args) {
    if (o instanceof T) return escape(o)[name](...args.map(escape));
    // A traced result decides there, as an `if` does (JavaScript takes any object as true).
    if (isArray(o) && TESTS.has(name) && typeof args[0] === "function") {
      const f = args[0];
      args[0] = function (...a) {
        const r = f.apply(this, a);
        return r instanceof T ? __if(r, at) : r;
      };
    }
    return o[name](...args);
  }

  function __spread(x) {
    return escape(x);
  }

  // `Array.from({ length })` with a traced length: the length decides the structure.
  Array.from = function from(source, ...rest) {
    if (source && source.length instanceof T) {
      source = { ...source, length: exact(source.length, "length", site()) };
    }
    return nativeFrom.call(Array, source, ...rest);
  };

  // ------------------------------------------------------------------------------ Motion API

  function useCurrentTime() {
    return {
      seconds: G.seconds,
      progress: G.progress,
      clipSeconds: G.seconds,
      clipProgress: G.progress,
    };
  }

  function useComposition() {
    const c = G.composition;
    return {
      width: c.width,
      height: c.height,
      fps: c.fps,
      duration: c.duration,
      clipDuration: c.duration,
    };
  }

  // A resource slot of `controls.resources`, which tools read without running the scene.
  function resource(slot) {
    return slot;
  }

  const EXTRAPOLATE = ["clamp", "extend"];

  // `interpolate(input, inputs, outputs, { extrapolateLeft, extrapolateRight })` over numbers.
  // The options decide the op, so a traced option is fixed by a guard.
  function interpolate(x, inputs, outputs, options) {
    if (
      !isArray(inputs) ||
      !isArray(outputs) ||
      inputs.length < 2 ||
      inputs.length !== outputs.length
    ) {
      throw new TypeError(
        "interpolate(input, inputRange, outputRange, options?) takes two ranges of one length, at least 2",
      );
    }
    const o = options ?? {};
    for (const k of keys(o)) {
      if (k !== "extrapolateLeft" && k !== "extrapolateRight") {
        throw new TypeError(
          "interpolate has no option " + k + "; it takes extrapolateLeft and extrapolateRight",
        );
      }
    }
    const side = (k) => {
      const e = o[k] instanceof T ? exact(o[k], "interpolate", site()) : (o[k] ?? "clamp");
      if (!EXTRAPOLATE.includes(e)) {
        throw new TypeError("interpolate " + k + ' must be "clamp" or "extend"');
      }
      return e;
    };
    const args = [x, ...inputs, ...outputs];
    if (!args.every(numeric)) throw new TypeError("interpolate takes numbers");
    const name = "interpolate|" + side("extrapolateLeft") + "|" + side("extrapolateRight");
    return args.some(isT) ? mk(name, args) : host.op(name, args);
  }

  // ------------------------------------------------------------------------------ elements

  function __h(type, props, ...children) {
    return { __el: true, type, props: props || {}, children, site: -1 };
  }

  function __hs(site, type, props, ...children) {
    return { __el: true, type, props: props || {}, children, site };
  }

  const __F = "__F";

  // A TypeError at an element's place in the source.
  function fail(at, message) {
    const e = new TypeError(message);
    if (at >= 0) e.site = at;
    throw e;
  }

  const INTRINSIC = { View: "view", Text: "text", Span: "span", Mask: "mask", Image: "image" };

  // Tags that are gone, with what replaces them.
  const REMOVED = {
    Scene: "return the elements directly",
    Screen: "content outside <World> is in screen space",
    Group: "use <View>",
    Flex: 'use <View className="flex">',
    Absolute: 'use <View className="absolute">',
    GeometryBatch: "particles() and field() return arrays: render them with .map",
    Clip: "use the CSS clip-path property",
    MaskSource: "use <Mask source={…}>",
    ThemeProvider: "import theme constants from a module, or pass them as props",
    Path: "use <path> inside an inline <svg>",
    Circle: "use <circle> inside an inline <svg>",
    Rect: "use <rect> inside an inline <svg>",
    Ellipse: "use <ellipse> inside an inline <svg>",
    Line: "use <line> inside an inline <svg>",
    Polyline: "use <polyline> inside an inline <svg>",
    Polygon: "use <polygon> inside an inline <svg>",
  };
  const HTML = {
    div: "View",
    section: "View",
    main: "View",
    article: "View",
    span: "Span",
    p: "Text",
    strong: "Span",
    h1: "Text",
    h2: "Text",
    h3: "Text",
    h4: "Text",
    h5: "Text",
    h6: "Text",
    img: "Image",
    video: "Video",
  };

  // The SVG elements Valle draws and how it checks them, as the engine's tables have them: each
  // element's own attributes and the elements it holds (null any, absent none), the attributes
  // every element takes, those that change nothing, the elements that draw nothing, and those
  // never drawn with what to do instead.
  const SVG_TABLES = JSON.parse(host.svg);
  const SVG = SVG_TABLES.elements;
  const SVG_HOLDS = SVG_TABLES.holds;
  const SVG_ATTRIBUTES = SVG_TABLES.common;
  const SVG_INERT = SVG_TABLES.inert;
  const SVG_SILENT = SVG_TABLES.silent;
  const SVG_NEVER = SVG_TABLES.never;

  // Props each element takes besides key, className and style.
  const PROPS = {
    view: [],
    text: [],
    span: [],
    mask: ["source", "mode", "invert"],
    image: ["src"],
  };

  const fixed = (x, kind, at) => (x instanceof T ? exact(x, kind, at) : x);

  function styleOf(style, tag, at) {
    if (style === undefined || style === null) return [];
    if (!plain(style)) fail(at, "<" + tag + "> style must be an object");
    const out = [];
    for (const [k, v] of entries(style)) {
      if (k.startsWith("--")) {
        fail(
          at,
          "CSS variables (" + k + ") are not supported: share values as JavaScript constants",
        );
      }
      if (k.includes("-")) {
        const camel = k.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
        fail(at, "style keys are camelCase, as in React: write " + camel + ", not " + k);
      }
      if (v === undefined || v === null) continue;
      if (!scalar(v) || typeof v === "boolean") {
        fail(at, "<" + tag + "> style " + k + " must be a number or a string");
      }
      out.push([k, v]);
    }
    return out;
  }

  // What a <Text> or a <Span> holds, into `node`: runs of strings, numbers and traced values,
  // each joined, and <Span>s, which are its children too.
  function contentOf(node, children, tag, at) {
    const content = [];
    let parts = [];
    const flush = () => {
      if (parts.length === 0) return;
      content.push({ text: parts.some(isT) ? template(parts) : parts.map(String).join("") });
      parts = [];
    };
    const add = (c, at) => {
      if (c === null || c === undefined || typeof c === "boolean") return;
      if (isArray(c)) return c.forEach((x) => add(x, at));
      if (c.__el) {
        const here = c.site >= 0 ? c.site : at;
        if (c.type === __F) return c.children.forEach((x) => add(x, here));
        if (typeof c.type === "function") return expand(c, here, (x) => add(x, here));
        if ((INTRINSIC[c.type] && c.type !== "Span") || c.type in SVG) {
          fail(here, "<" + c.type + "> cannot be inside <" + tag + ">: it holds text and <Span>s");
        }
        flush();
        element(c.type, c.props, c.children, node.children, here, true);
        content.push({ span: node.children[node.children.length - 1] });
        return;
      }
      if (typeof c === "object" && !isT(c)) fail(at, "<" + tag + "> holds text, not an object");
      parts.push(c);
    };
    children.forEach((c) => add(c, at));
    flush();
    node.content = content;
  }

  // A <Mask>'s mode and inversion, and the root its source renders into, which is laid out in
  // its border box as the canvas is.
  function maskOf(node, props, at) {
    if (props.source === undefined) {
      fail(at, "<Mask> needs a source: the element whose painting masks its children");
    }
    const mode = fixed(props.mode ?? "alpha", "mode", at);
    if (mode !== "alpha" && mode !== "luminance") {
      fail(at, '<Mask> mode is "alpha" or "luminance", not ' + stringify(mode));
    }
    const invert = fixed(props.invert ?? false, "invert", at);
    if (typeof invert !== "boolean") fail(at, "<Mask> invert is true or false");
    node.luminance = mode === "luminance";
    node.invert = invert;
    node.source = { type: "view", path: null, classes: [], style: [], children: [], site: at };
  }

  // An <Image>'s resource, by the name its src gives as a literal: which files a scene reads
  // cannot depend on time.
  function imageOf(node, props, children, at) {
    const src = props.src;
    if (src instanceof T) {
      fail(at, '<Image> src is written as a literal, such as "resource://logo"; it cannot change');
    }
    if (typeof src !== "string" || !src.startsWith("resource://")) {
      fail(
        at,
        '<Image> src is a resource declared in controls.resources, such as "resource://logo", not ' +
          stringify(src),
      );
    }
    if (children.length > 0) fail(at, "<Image> has no children");
    node.src = src.slice("resource://".length);
  }

  // An inline SVG element, into `out`: its attributes, constants or traced values, and the SVG
  // elements in it.
  function svgElement(type, props, children, out, at) {
    if (type !== "svg" && G.svg === 0) fail(at, "<" + type + "> must be inside an <svg>");
    if (type === "svg" && G.svg > 0) fail(at, "an <svg> inside an <svg> is not supported yet");
    if (++G.nodes > MAX_NODES) fail(at, "a frame has more than " + MAX_NODES + " elements");
    const attributes = [];
    for (const [k, v] of entries(props)) {
      if (k === "key" || k === "className" || k === "style" || k === "children") continue;
      if (SVG_INERT.includes(k) || /^(aria|data)-/.test(k) || v === undefined || v === null) {
        continue;
      }
      if (k.includes("-") || k.includes(":")) {
        const camel = k.replace(/[-:]([a-z])/g, (_, c) => c.toUpperCase());
        fail(at, "attributes are camelCase, as in React: write " + camel + ", not " + k);
      }
      if (/^on[A-Z]/.test(k)) {
        fail(
          at,
          "<" + type + "> " + k + ": event handlers are not supported, a video has no interaction",
        );
      }
      if (type === "svg" && k === "transform") {
        fail(at, "<svg> transform is not supported yet: transform it with its style");
      }
      if (!SVG[type].includes(k) && !SVG_ATTRIBUTES.includes(k)) {
        fail(at, "<" + type + "> " + k + " is not supported yet");
      }
      if (!scalar(v) || typeof v === "boolean") {
        fail(at, "<" + type + "> " + k + " must be a number or a string");
      }
      attributes.push([k, v]);
    }
    const key = props.key === undefined ? null : String(fixed(props.key, "key", at));
    const className = fixed(props.className ?? "", "className", at);
    if (typeof className !== "string") fail(at, "<" + type + "> className must be a string");
    const node = {
      type: "svg",
      tag: type,
      attributes,
      path: key === null ? null : [...G.keys, key].join("/"),
      classes: className.split(/\s+/).filter(Boolean),
      style: styleOf(props.style, type, at),
      children: [],
      site: at,
    };
    if (key !== null) G.keys.push(key);
    G.svg++;
    try {
      for (const c of children) render(c, node.children, at);
    } finally {
      G.svg--;
      if (key !== null) G.keys.pop();
    }
    const holds = SVG_HOLDS[type];
    if (holds === undefined && node.children.length > 0) {
      fail(at, "<" + type + "> holds no elements");
    }
    for (const c of node.children) {
      if (holds && !holds.tags.includes(c.tag)) {
        fail(c.site, "<" + c.tag + "> cannot be inside <" + type + ">: it holds " + holds.what);
      }
      if (c.tag === "stop" && !holds) {
        fail(c.site, "<stop> must be inside a <linearGradient> or <radialGradient>");
      }
    }
    out.push(node);
  }

  function element(type, props, children, out, at, inText) {
    if (typeof type !== "string") fail(at, "an element type must be a tag or a component function");
    if (type in SVG) return svgElement(type, props, children, out, at);
    if (G.svg > 0) {
      if (SVG_SILENT.includes(type)) return;
      if (type in SVG_NEVER) fail(at, "<" + type + "> is not supported: " + SVG_NEVER[type]);
      if (INTRINSIC[type]) fail(at, "<" + type + "> cannot be inside an <svg>");
      fail(at, "<" + type + "> in an <svg> is not supported yet");
    }
    const kind = INTRINSIC[type];
    if (!kind) {
      if (type in REMOVED) fail(at, "<" + type + "> is removed: " + REMOVED[type]);
      if (type in HTML) {
        fail(at, "HTML tags are not supported: use <" + HTML[type] + "> instead of <" + type + ">");
      }
      fail(at, "<" + type + "> is not a Valle element");
    }
    if (kind === "span" && !inText) fail(at, "<Span> must be inside <Text>");
    if (++G.nodes > MAX_NODES) fail(at, "a frame has more than " + MAX_NODES + " elements");
    for (const k of keys(props)) {
      if (k === "key" || k === "className" || k === "style" || PROPS[kind].includes(k)) continue;
      if (/^on[A-Z]/.test(k)) {
        fail(
          at,
          "<" + type + "> " + k + ": event handlers are not supported, a video has no interaction",
        );
      }
      fail(at, "<" + type + "> has no prop " + k);
    }
    const key = props.key === undefined ? null : String(fixed(props.key, "key", at));
    const className = fixed(props.className ?? "", "className", at);
    if (typeof className !== "string") fail(at, "<" + type + "> className must be a string");
    const node = {
      type: kind,
      path: key === null ? null : [...G.keys, key].join("/"),
      classes: className.split(/\s+/).filter(Boolean),
      style: styleOf(props.style, type, at),
      children: [],
      site: at,
    };
    if (kind === "mask") maskOf(node, props, at);
    if (kind === "image") imageOf(node, props, children, at);
    if (key !== null) G.keys.push(key);
    try {
      if (kind === "text" || kind === "span") {
        contentOf(node, children, type, at);
      } else {
        if (kind === "mask") render(props.source, node.source.children, at);
        for (const c of children) render(c, node.children, at);
      }
    } finally {
      if (key !== null) G.keys.pop();
    }
    out.push(node);
  }

  // Appends the nodes `x` renders to `out`; `at` is the enclosing element's site.
  function render(x, out, at) {
    if (x === null || x === undefined || typeof x === "boolean") return;
    if (isArray(x)) return x.forEach((y) => render(y, out, at));
    if (!x.__el) {
      if (typeof x === "string" && x.trim() === "") return;
      if (G.svg > 0) fail(at, "text in an <svg> is not supported yet: " + stringify(String(x)));
      fail(at, "text must be inside <Text>: " + stringify(String(x)));
    }
    const { type, props, children } = x;
    const here = x.site >= 0 ? x.site : at;
    if (type === __F) return children.forEach((c) => render(c, out, here));
    if (typeof type === "function") return expand(x, here, (y) => render(y, out, here));
    element(type, props, children, out, here);
  }

  // What component element `x` renders, given to `f`. A keyed component is a segment of its
  // nodes' key paths.
  function expand(x, here, f) {
    const { type, props, children } = x;
    const kids = children.length === 0 ? undefined : children.length === 1 ? children[0] : children;
    const key = props.key === undefined ? null : String(fixed(props.key, "key", here));
    const rest = { ...props };
    delete rest.key;
    if (key !== null) G.keys.push(key);
    try {
      return f(type(kids === undefined ? rest : { ...rest, children: kids }));
    } finally {
      if (key !== null) G.keys.pop();
    }
  }

  // ------------------------------------------------------------------------------ output

  function constant(x) {
    if (typeof x === "number") {
      if (Number.isFinite(x)) return Object.is(x, -0) ? { nf: "-0" } : x;
      return { nf: String(x) };
    }
    return x;
  }

  // The variant as `front/lower.rs` reads it: ops in dependency order, nodes flat.
  function serialize(roots, guards) {
    const ids = new Map();
    const ops = [];
    const arg = (t, a) =>
      a instanceof T
        ? { e: ids.get(a) }
        : t.op === "template"
          ? { text: a.text }
          : { c: constant(a) };
    function id(root) {
      const stack = [root];
      while (stack.length > 0) {
        const t = stack[stack.length - 1];
        if (ids.has(t)) {
          stack.pop();
          continue;
        }
        const before = stack.length;
        for (const a of t.args) if (a instanceof T && !ids.has(a)) stack.push(a);
        if (stack.length > before) continue;
        stack.pop();
        ops.push([t.op, t.args.map((a) => arg(t, a))]);
        ids.set(t, ops.length - 1);
      }
      return ids.get(root);
    }
    const ref = (x) => (x instanceof T ? { e: id(x) } : { c: constant(x) });
    const nodes = [];
    function node(n) {
      const out = {
        type: n.type,
        path: n.path,
        classes: n.classes,
        style: n.style.map(([k, v]) => [k, ref(v)]),
        children: [],
        site: n.site >= 0 ? n.site : null,
      };
      nodes.push(out);
      const index = nodes.length - 1;
      if ("source" in n) {
        out.mask = { source: node(n.source), luminance: n.luminance, invert: n.invert };
      }
      if ("src" in n) out.src = n.src;
      if ("tag" in n) {
        out.tag = n.tag;
        out.attributes = n.attributes.map(([k, v]) => [k, ref(v)]);
      }
      out.children = n.children.map(node);
      if ("content" in n) {
        // Its children are its Spans, in the order its content has them.
        let k = 0;
        out.content = n.content.map((p) =>
          "span" in p ? { span: out.children[k++] } : { text: ref(p.text) },
        );
      }
      return index;
    }
    const top = roots.map(node);
    const checks = guards.map((g) => ({
      e: id(g.t),
      truthy: g.want.truthy,
      eq: "eq" in g.want ? constant(g.want.eq) : undefined,
      when: g.when.map(([c, side]) => [id(c), side]),
      kind: g.kind,
      at: g.at,
    }));
    return { ops, nodes, roots: top, guards: checks };
  }

  // ------------------------------------------------------------------------------ modules

  const factories = new Map();
  let instances = null;

  // `factory` makes the exports of the module the import keys `names` resolve to.
  function __define(names, factory) {
    for (const k of names) factories.set(k, factory);
  }

  function __import(key) {
    const factory = factories.get(key);
    if (!instances.has(factory)) instances.set(factory, freeze(factory()));
    return instances.get(factory);
  }

  // Traces the scene `entry` at a time; returns the variant as JSON.
  function __trace(entry, seconds, progress, composition) {
    G = {
      guards: [],
      when: [],
      keys: [],
      ops: 0,
      nodes: 0,
      // How deep in <svg> elements tracing is.
      svg: 0,
      composition,
      seconds: new T("seconds", [], seconds),
      progress: new T("progress", [], progress),
    };
    instances = new Map();
    try {
      const scene = __import(entry);
      if (typeof scene.default !== "function") {
        throw new TypeError("the scene's default export must be its root component, a function");
      }
      const roots = [];
      render(__h(scene.default, {}), roots, -1);
      return stringify(serialize(roots, G.guards));
    } finally {
      G = null;
      instances = null;
    }
  }

  // ------------------------------------------------------------------------------ environment

  // React hooks and Remotion names that have no place here, with what replaces them.
  const UNAVAILABLE = {
    useState:
      "components have no state: a frame is a function of time; keep cached values as module constants",
    useReducer: "components have no state: a frame is a function of time",
    useRef: "components have no state: keep cached values as module constants",
    useEffect: "there are no effects: a frame is a function of time",
    useLayoutEffect: "there are no effects: a frame is a function of time",
    useMemo: "compute the value as a module constant",
    useCallback: "define the function at module level",
    useContext: "import module constants, or pass values as props",
    createContext: "import module constants, or pass values as props",
    useTheme: "import theme constants from a module, or pass them as props",
    useCurrentFrame: "use useCurrentTime(), which gives seconds",
    useVideoConfig: "use useComposition()",
  };

  // Free names the scene reads: a capitalized one is a tag (an unknown one fails when rendered,
  // with its replacement where it has one); the gone hooks throw with theirs.
  function __declareFree(names) {
    for (const name of names) {
      if (name in globalThis) continue;
      if (name in UNAVAILABLE) {
        globalThis[name] = () => {
          throw new TypeError(name + "() is not supported: " + UNAVAILABLE[name]);
        };
      } else if (/^[A-Z]/.test(name)) {
        globalThis[name] = name;
      }
    }
  }

  // Globals the environment does not have, with what replaces them.
  const ABSENT = {
    Date: "animation time comes from useCurrentTime()",
    performance: "animation time comes from useCurrentTime()",
    Promise: "a frame is computed synchronously, so there is nothing to wait for",
    queueMicrotask: "a frame is computed synchronously, so there is nothing to wait for",
    setTimeout: "animate with interpolate() over useCurrentTime()",
    setInterval: "animate with interpolate() over useCurrentTime()",
    requestAnimationFrame: "animate with interpolate() over useCurrentTime()",
    fetch: "pass outside data in through controls.data",
    XMLHttpRequest: "pass outside data in through controls.data",
    Proxy: "animation does not need it",
    WeakRef: "its result depends on when garbage is collected",
    FinalizationRegistry: "its result depends on when garbage is collected",
    Intl: "its output differs between platforms; use formatNumber() and the other format functions",
  };

  function refuse(what, instead) {
    return function () {
      throw new TypeError(what + " is not supported: " + instead);
    };
  }

  // Freezes `x`, its prototypes and everything reachable from its properties.
  function harden(x, seen = new Set()) {
    if ((typeof x !== "object" && typeof x !== "function") || x === null || seen.has(x)) return;
    seen.add(x);
    harden(getPrototypeOf(x), seen);
    for (const d of Object.values(getOwnPropertyDescriptors(x))) {
      harden(d.value, seen);
      harden(d.get, seen);
      harden(d.set, seen);
    }
    freeze(x);
  }

  // Built-in objects no global property leads to, reached through instances.
  const HIDDEN = [
    () => T.prototype,
    () => getPrototypeOf(function* () {}),
    () => getPrototypeOf([][Symbol.iterator]()),
    () => getPrototypeOf(new Map()[Symbol.iterator]()),
    () => getPrototypeOf(new Set()[Symbol.iterator]()),
    () => getPrototypeOf(""[Symbol.iterator]()),
    () => getPrototypeOf(/(?:)/g[Symbol.matchAll]("")),
    () => getPrototypeOf([].values().map((x) => x)),
    () => getPrototypeOf(Iterator.from({ next: () => ({ done: true }) })),
    () => getPrototypeOf(Iterator.concat()),
  ];

  function __lockdown() {
    for (const name of ["__host", "__declareFree", "__define", "__lockdown"]) {
      delete globalThis[name];
    }

    const noFunction = refuse(
      "new Function",
      "code made at run time bypasses tracing; write it in the scene",
    );
    noFunction.prototype = Function.prototype;
    // oxlint-disable-next-line no-eval -- this replaces eval with one that throws
    globalThis.eval = refuse(
      "eval",
      "code made at run time bypasses tracing; write it in the scene",
    );
    globalThis.Function = noFunction;
    for (const f of [function () {}, function* () {}]) {
      defineProperty(getPrototypeOf(f), "constructor", { value: noFunction });
    }

    for (const [name, instead] of entries(ABSENT)) {
      delete globalThis[name];
      defineProperty(globalThis, name, {
        get() {
          throw new ReferenceError(name + " is not available in scenes: " + instead);
        },
      });
    }

    // The stack-trace settings are context state that would outlive a trace: fix them.
    Error.stackTraceLimit = 64;
    defineProperty(Error, "stackTraceLimit", { value: 64 });
    defineProperty(Error, "prepareStackTrace", { value: undefined });

    Math.random = refuse(
      "Math.random()",
      "it differs on every run; derive values from an index or a seed",
    );
    for (const name of MATH) Math[name] = (...args) => math(name, args);

    // An async function, with no promises here; QuickJS-ng also leaks a reference when it
    // instantiates the function, which the freezing walk below would do.
    delete Array.fromAsync;

    harden(globalThis);
    for (const reach of HIDDEN) {
      let x;
      try {
        x = reach();
      } catch {
        continue;
      }
      harden(x);
    }
  }

  Object.assign(globalThis, {
    __b,
    __u,
    __if,
    __sw,
    __sel,
    __and,
    __or,
    __nc,
    __tpl,
    __get,
    __idx,
    __call,
    __spread,
    __h,
    __hs,
    __F,
    __define,
    __import,
    __trace,
    __declareFree,
    __lockdown,
    useCurrentTime,
    useComposition,
    interpolate,
    View: "View",
    Text: "Text",
    Span: "Span",
    Mask: "Mask",
    Image: "Image",
    resource,
  });
})();
