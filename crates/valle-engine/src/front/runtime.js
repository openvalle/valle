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
  const parse = JSON.parse;
  const NativeError = Error;
  const NativeNumber = Number;

  // What one variant may hold; past these a trace fails. An op takes about 300 bytes of script
  // memory while tracing, so the op limit stays well inside the memory limit.
  const MAX_OPS = 250000;
  const MAX_NODES = 100000;
  const MAX_GUARDS = 10000;

  let G = null;

  // ------------------------------------------------------------------------------ traced values

  // `late`: known only after layout, as it reads a box (`bounds()`).
  function T(op, args, v, late) {
    this.op = op;
    this.args = freeze(args);
    this.v = v;
    this.late = late;
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
    const late = op.startsWith("bounds|") || args.some((a) => a instanceof T && a.late);
    if (!late) return new T(op, args, host.op(op, args.map(val)), false);
    // While tracing a box measures 0; what fails on such boxes waits for layout as NaN.
    let v;
    try {
      v = host.op(op, args.map(val));
    } catch {
      v = NaN;
    }
    return new T(op, args, v, true);
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
    if (t.late || G.when.some(([c]) => c.late)) {
      throw new TypeError(
        "a value from bounds() is known only after layout, so it cannot decide what the frame " +
          "holds (" +
          kind +
          ") or reach code that does not trace it",
      );
    }
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
  const EQUALITY = new Set(["==", "===", "!=", "!=="]);
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
    // A traced value is never null or undefined, so it is not equal to them.
    if (EQUALITY.has(op) && (a == null || b == null)) return op[0] === "!";
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
      clipSeconds: G.clip[0],
      clipProgress: G.clip[1],
    };
  }

  function useComposition() {
    const c = G.composition;
    return {
      width: c.width,
      height: c.height,
      fps: c.fps,
      duration: c.duration,
      clipDuration: c.clipDuration ?? c.duration,
    };
  }

  // A resource slot of `controls.resources`, which tools read without running the scene.
  function resource(slot) {
    return slot;
  }

  // A prop of `controls.props` of a kind, which tools read without running the scene.
  const propKind = (kind) => (options) => ({ kind, ...options });

  // The scene's role in a Timeline, which tools read without running the scene.
  const roleOf = (role) => (options) => ({ role, ...options });

  const EXTRAPOLATE = ["clamp", "extend"];
  const INTERPOLATE = ["easing", "colorSpace", "hue", "extrapolateLeft", "extrapolateRight"];

  // `interpolate(input, inputs, outputs, { easing, colorSpace, hue, extrapolateLeft,
  // extrapolateRight })` over numbers, colors, or lengths and angles.
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
      if (!INTERPOLATE.includes(k)) {
        throw new TypeError(
          "interpolate has no option " + k + "; it takes " + INTERPOLATE.join(", "),
        );
      }
    }
    const fixed = (k, otherwise) =>
      o[k] instanceof T ? exact(o[k], "interpolate", site()) : (o[k] ?? otherwise);
    const text = (k, otherwise) => {
      const v = fixed(k, otherwise);
      if (typeof v !== "string") throw new TypeError("interpolate " + k + " must be a string");
      return v;
    };
    const side = (k) => {
      const e = fixed(k, "clamp");
      if (!EXTRAPOLATE.includes(e)) {
        throw new TypeError("interpolate " + k + ' must be "clamp" or "extend"');
      }
      return e;
    };
    const args = [x, ...inputs, ...outputs];
    const strings = outputs.every((o) => typeof val(o) === "string");
    if (![x, ...inputs].every(numeric) || !(strings || outputs.every(numeric))) {
      throw new TypeError(
        "interpolate takes numbers in, and numbers, colors, or lengths and angles out",
      );
    }
    const spec = [
      side("extrapolateLeft"),
      side("extrapolateRight"),
      text("colorSpace", ""),
      text("hue", ""),
      text("easing", "linear"),
    ];
    const name = "interpolate|" + spec.join("|");
    return args.some(isT) ? mk(name, args) : host.op(name, args);
  }

  // A sequence of stages: each starts `at` a time or `after` an earlier one, with a `delay` or
  // an `overlap`; the first starts at 0 if it says neither. Fixed values, so plain numbers out.
  function defineSequence(stages) {
    if (!stages || typeof stages !== "object" || isArray(stages)) {
      throw new TypeError("defineSequence takes an object of stages by name");
    }
    const out = {};
    for (const name of keys(stages)) {
      const s = stages[name] ?? {};
      const at = (k) => {
        if (!STAGE.includes(k)) {
          throw new TypeError("stage " + name + " has no " + k + "; it takes " + STAGE.join(", "));
        }
        const v = s[k] instanceof T ? exact(s[k], "defineSequence", site()) : s[k];
        if (v !== undefined && k !== "after" && !(typeof v === "number" && Number.isFinite(v))) {
          throw new TypeError("stage " + name + " " + k + " must be a number");
        }
        return v;
      };
      keys(s).forEach(at);
      const duration = at("duration");
      if (!(duration > 0)) throw new TypeError("stage " + name + " needs a duration over 0");
      let start;
      if (at("at") !== undefined) {
        if (at("after") !== undefined || at("delay") !== undefined || at("overlap") !== undefined) {
          throw new TypeError(
            "stage " + name + " starts at a time; after, delay and overlap go without at",
          );
        }
        start = at("at");
      } else if (at("after") !== undefined) {
        const before = out[at("after")];
        if (!before) {
          throw new TypeError(
            "stage " + name + " is after " + at("after") + ", which is no stage before it",
          );
        }
        start = before.start + before.duration + (at("delay") ?? 0) - (at("overlap") ?? 0);
      } else if (
        keys(out).length === 0 &&
        at("delay") === undefined &&
        at("overlap") === undefined
      ) {
        start = 0;
      } else {
        throw new TypeError("stage " + name + " needs at or after");
      }
      out[name] = { start, duration };
    }
    return out;
  }
  const STAGE = ["duration", "at", "after", "delay", "overlap"];

  // A stage's progress at `t`: once, `count` times, or there and back `count` times; the stage, the
  // count, the index and the gap are fixed.
  function progress(name, t, stage, how, count, shift) {
    const fix = (v) => (v instanceof T ? exact(v, name, site()) : v);
    const finite = (v) => typeof v === "number" && Number.isFinite(v);
    const [start, duration] = [fix(stage?.start), fix(stage?.duration)];
    if (!finite(start) || !finite(duration)) {
      throw new TypeError(name + " takes a stage from defineSequence, or { start, duration }");
    }
    if (!numeric(t)) throw new TypeError(name + " takes a number of seconds");
    if (!finite(fix(count)) || !finite(shift)) throw new TypeError(name + " takes numbers");
    if (!(duration > 0) || !(fix(count) > 0)) {
      throw new TypeError(name + ": the duration and the count must be over 0");
    }
    const op = "progress|" + [start + shift, duration, fix(count), how].join("|");
    return isT(t) ? mk(op, [t]) : host.op(op, [t]);
  }
  const stageProgress = (t, stage) => progress("stageProgress", t, stage, "once", 1, 0);
  const staggerProgress = (t, stage, index, gap) => {
    const fix = (v) => (v instanceof T ? exact(v, "staggerProgress", site()) : v);
    return progress("staggerProgress", t, stage, "once", 1, fix(index) * fix(gap));
  };
  const repeatProgress = (t, stage, count) =>
    progress("repeatProgress", t, stage, "repeat", count, 0);
  const yoyoProgress = (t, stage, count) => progress("yoyoProgress", t, stage, "yoyo", count, 0);

  // `simulate({ dt, duration, init, step })`: a state of numbers stepped here every `dt` seconds
  // up to `duration`, once; `at(t)` reads each field linearly between steps.
  function simulate(options) {
    const o = options ?? {};
    if (keys(o).some((k) => !SIMULATE.includes(k))) {
      throw new TypeError("simulate takes " + SIMULATE.join(", "));
    }
    const fix = (v) => (v instanceof T ? exact(v, "simulate", site()) : v);
    const [dt, duration] = [fix(o.dt), fix(o.duration)];
    if (!(Number.isFinite(dt) && dt > 0 && Number.isFinite(duration) && duration > 0)) {
      throw new TypeError("simulate dt and duration must be numbers over 0");
    }
    if (typeof o.init !== "function" || typeof o.step !== "function") {
      throw new TypeError("simulate init and step must be functions");
    }
    const steps = Math.ceil(duration / dt);
    if (steps > MAX_STEPS) {
      throw new RangeError("simulate takes at most " + MAX_STEPS + " steps; make dt longer");
    }
    let names;
    const read = (state) => {
      if (!plain(state) || hasT(state)) {
        throw new TypeError(
          "a simulation's state is an object of numbers that do not depend on the time",
        );
      }
      const k = keys(state).sort();
      names ??= k;
      if (k.join() !== names.join() || k.some((n) => !Number.isFinite(state[n]))) {
        throw new TypeError("a simulation's state keeps the same fields, each a finite number");
      }
      return state;
    };
    let state = read(o.init());
    const fields = names.map((n) => [state[n]]);
    for (let i = 0; i < steps; i++) {
      state = read(o.step(state, i * dt, dt));
      names.forEach((n, j) => fields[j].push(state[n]));
    }
    const ops = fields.map((v) => "table|" + dt + "|" + duration + "|" + v.join(" "));
    return {
      at(t) {
        if (!numeric(t)) throw new TypeError("a simulation's at(seconds) takes a number");
        const out = {};
        names.forEach((n, j) => (out[n] = isT(t) ? mk(ops[j], [t]) : host.op(ops[j], [t])));
        return out;
      },
    };
  }
  const SIMULATE = ["dt", "duration", "init", "step"];
  const MAX_STEPS = 16384;

  // `seededRandom(seed, count, [low, high])`: `count` numbers in [low, high), [0, 1) if left out,
  // the same on every host. Fixed values in, so plain numbers out.
  function seededRandom(seed, count, range) {
    const fix = (v) => (v instanceof T ? exact(v, "seededRandom", site()) : v);
    [seed, count, range] = [fix(seed), fix(count), fix(range ?? [0, 1])];
    const [low, high] = isArray(range) ? range.map(fix) : [];
    if (!Number.isSafeInteger(seed) || seed < 0) {
      throw new TypeError("seededRandom seed must be a whole number, at least 0");
    }
    if (!Number.isSafeInteger(count) || count < 0 || count > MAX_RANDOM) {
      throw new TypeError("seededRandom count must be a whole number from 0 to " + MAX_RANDOM);
    }
    if (range.length !== 2 || !Number.isFinite(low) || !Number.isFinite(high)) {
      throw new TypeError("seededRandom range must be [low, high], two numbers");
    }
    return host.random(seed, count, low, high);
  }
  const MAX_RANDOM = 1 << 20;

  // `noise1d(seed, x)` and `noise2d(seed, x, y)`: smooth noise in [0, 1]; the seed is fixed.
  const noise = (name, seed, at) => {
    seed = seed instanceof T ? exact(seed, name, site()) : seed;
    if (!Number.isSafeInteger(seed) || seed < 0) {
      throw new TypeError(name + " seed must be a whole number, at least 0");
    }
    if (!at.every(numeric)) throw new TypeError(name + " takes numbers");
    return at.some(isT) ? mk("noise|" + seed, at) : host.op("noise|" + seed, at);
  };
  const noise1d = (seed, x) => noise("noise1d", seed, [x]);
  const noise2d = (seed, x, y) => noise("noise2d", seed, [x, y]);

  // The text and the style `measureText` or `textOutline` (`name`) lays out, fixed, the style as
  // [property, value] pairs.
  function typeset(name, text, style, options) {
    const fix = (v) => (v instanceof T ? exact(v, name, site()) : v);
    text = fix(text);
    if (typeof text !== "string" && typeof text !== "number") {
      throw new TypeError(name + "(text, " + options + ") takes a string");
    }
    const s = style ?? {};
    if (!plain(s))
      throw new TypeError(name + "'s " + options + " is an object, as a <Text>'s style is");
    const declared = [];
    for (const [k, v] of entries(s)) {
      const value = fix(v);
      if (value === undefined || value === null) continue;
      if (typeof value !== "number" && typeof value !== "string") {
        throw new TypeError(name + " style " + k + " must be a number or a string");
      }
      declared.push([k, value]);
    }
    return [String(text), declared];
  }

  // `measureText(text, style)`: the box a <Text> of `text` in `style` lays out in, alone and
  // absolute in the canvas: max-content wide, or wrapped at the style's maxWidth; `width` and
  // `height` in px and its count of `lines`. What it measures cannot change with time.
  function measureText(text, style) {
    const [content, declared] = typeset("measureText", text, style, "style");
    const [width, height, lines] = host.measure(content, stringify(declared));
    return freeze({ width, height, lines });
  }

  // `textOutline(text, { ...style, origin, align })`: the outlines of the glyphs a <Text> of
  // `text` in `style` lays out, as SVG path data in px: the first line's baseline at `origin`
  // ({ x, y }, 0 0 by default), the lines aligned "left" (the default), "center" or "right" of
  // its x. It gives `path`, all of them, their `bounds` ({ x, y, width, height }), and `glyphs`,
  // each glyph with ink: its `path`, its baseline origin `x` and `y`, and its `line` and `word`
  // (white space parts words) from 0. What it outlines cannot change with time.
  function textOutline(text, options) {
    const { origin, align, ...style } = options ?? {};
    const fix = (v) => (v instanceof T ? exact(v, "textOutline", site()) : v);
    const o = fix(origin) ?? {};
    const x = fix(o.x) ?? 0;
    const y = fix(o.y) ?? 0;
    if (typeof x !== "number" || typeof y !== "number") {
      throw new TypeError("textOutline origin is { x, y } in px");
    }
    const [content, declared] = typeset("textOutline", text, style, "options");
    const out = JSON.parse(host.outline(content, stringify(declared), x, y, fix(align) ?? "left"));
    freeze(out.bounds);
    out.glyphs.forEach(freeze);
    freeze(out.glyphs);
    return freeze(out);
  }

  // `rangeSelector(unit, { start, end, offset, softness, shape })`: the weight in [0, 1] of the
  // unit `perUnit` gets, at its index over the count, as After Effects' range selector gives it
  // (`eval.rs`). The shape is fixed: "square" (the default), "ramp", "smooth" or "triangle".
  const RANGE = ["start", "end", "offset", "softness", "shape"];
  const SHAPES = ["square", "ramp", "smooth", "triangle"];
  function rangeSelector(unit, options) {
    if (!unit || !Number.isInteger(unit.index) || !Number.isInteger(unit.count)) {
      throw new TypeError("rangeSelector takes the unit perUnit gets, then its options");
    }
    const o = options ?? {};
    for (const k of keys(o)) {
      if (!RANGE.includes(k)) {
        throw new TypeError("rangeSelector has no option " + k + "; it takes " + RANGE.join(", "));
      }
    }
    const shape = o.shape instanceof T ? exact(o.shape, "shape", site()) : (o.shape ?? "square");
    if (!SHAPES.includes(shape)) {
      throw new TypeError("rangeSelector shape is " + SHAPES.map((x) => '"' + x + '"').join(", "));
    }
    const args = [
      unit.index / unit.count,
      o.start ?? 0,
      o.end ?? 1,
      o.offset ?? 0,
      o.softness ?? 0,
    ];
    if (!args.every(numeric))
      throw new TypeError("rangeSelector start, end, offset and softness are numbers");
    const op = "range|" + shape;
    return args.some(isT) ? mk(op, args) : host.op(op, args);
  }

  // `formatNumber(value, { decimals, grouping })`, `formatPercent(value, { decimals, grouping })`
  // and `padNumber(value, { width })`: text as `Intl.NumberFormat("en-US")` writes it, remade
  // every frame for a value that changes with time. The options are fixed.
  function formatted(name, value, options, kind, allowed) {
    const o = options ?? {};
    if (keys(o).some((k) => !allowed.includes(k))) {
      throw new TypeError(name + " takes " + allowed.join(" and "));
    }
    const fix = (v) => (v instanceof T ? exact(v, name, site()) : v);
    const whole = (v, least, most) => Number.isInteger(v) && v >= least && v <= most;
    let spec;
    if (kind === "pad") {
      const width = fix(o.width);
      if (!whole(width, 1, 21))
        throw new TypeError(name + " width must be a whole number, 1 to 21");
      spec = ["pad", width, false];
    } else {
      const [decimals, grouping] = [fix(o.decimals), fix(o.grouping ?? true)];
      if (decimals !== undefined && !whole(decimals, 0, 100)) {
        throw new TypeError(name + " decimals must be a whole number, 0 to 100");
      }
      if (typeof grouping !== "boolean")
        throw new TypeError(name + " grouping must be true or false");
      spec = [kind, decimals ?? "", grouping];
    }
    if (!numeric(value)) throw new TypeError(name + " takes a number");
    const op = "format|" + spec.join("|");
    return isT(value) ? mk(op, [value]) : host.op(op, [value]);
  }
  const NUMBER = ["decimals", "grouping"];
  const formatNumber = (v, o) => formatted("formatNumber", v, o, "number", NUMBER);
  const formatPercent = (v, o) => formatted("formatPercent", v, o, "percent", NUMBER);
  const padNumber = (v, o) => formatted("padNumber", v, o, "pad", ["width"]);

  // Path data in: `pathLength(path)`, `pointAt(path, progress)` as `{ x, y }`, and
  // `tangentAt(path, progress)` as the unit direction `{ x, y }` and its `angle` in degrees.
  const pathOp = (name, f, args) => {
    const operand = (a) => numeric(a) || typeof val(a) === "string";
    if (typeof val(args[0]) !== "string" || !args.slice(1).every(operand)) {
      throw new TypeError(name + " takes path data" + (args.length > 1 ? " and a number" : ""));
    }
    return args.some(isT) ? mk("path|" + f, args) : host.op("path|" + f, args);
  };
  const pathLength = (path) => pathOp("pathLength", "length", [path]);
  const pointAt = (path, progress) => ({
    x: pathOp("pointAt", "x", [path, progress]),
    y: pathOp("pointAt", "y", [path, progress]),
  });
  const tangentAt = (path, progress) => ({
    x: pathOp("tangentAt", "dx", [path, progress]),
    y: pathOp("tangentAt", "dy", [path, progress]),
    angle: pathOp("tangentAt", "angle", [path, progress]),
  });

  // `roundCorners(path, radius)`: path data with its corners rounded.
  const roundCorners = (path, radius) => pathOp("roundCorners", "roundCorners", [path, radius]);
  // AE's path modifiers: `puckerBloat(path, amount)` with 1 for 100%, `twist(path, angle)` in
  // degrees, and `zigzag(path, { size, ridges })`.
  const puckerBloat = (path, amount) => pathOp("puckerBloat", "puckerBloat", [path, amount]);
  const twist = (path, angle) => pathOp("twist", "twist", [path, angle]);
  // `inflatePath(path, distance, { join, miterLimit })`: AE's Offset Paths, mitered at 4 unless
  // told otherwise.
  const inflatePath = (path, distance, options) => {
    const o = options ?? {};
    if (keys(o).some((k) => k !== "join" && k !== "miterLimit")) {
      throw new TypeError(
        "inflatePath(path, distance, { join, miterLimit }) takes join and miterLimit",
      );
    }
    const fix = (v) => (v instanceof T ? exact(v, "inflatePath", site()) : v);
    const [join, limit] = [fix(o.join ?? "miter"), fix(o.miterLimit ?? 4)];
    if (!["miter", "round", "bevel"].includes(join)) {
      throw new TypeError('inflatePath join must be "miter", "round" or "bevel"');
    }
    if (!(Number.isFinite(limit) && limit >= 1)) {
      throw new TypeError("inflatePath miterLimit must be a number, at least 1");
    }
    const f = "inflate|" + join + "|" + (join === "miter" ? limit : "");
    return pathOp("inflatePath", f, [path, distance]);
  };
  // `morph(from, to, progress, { samples })`: any two paths, see `path/edit.rs`.
  const morph = (from, to, progress, options) => {
    const o = options ?? {};
    if (keys(o).some((k) => k !== "samples")) {
      throw new TypeError("morph(from, to, progress, { samples }) takes samples");
    }
    const samples = o.samples instanceof T ? exact(o.samples, "morph", site()) : o.samples;
    if (samples !== undefined && !(Number.isInteger(samples) && samples >= 2 && samples <= 4096)) {
      throw new TypeError("morph samples must be a whole number, 2 to 4096");
    }
    if (typeof val(to) !== "string") throw new TypeError("morph takes two paths and a progress");
    return pathOp("morph", "morph|" + (samples ?? ""), [from, to, progress]);
  };
  // `pathBoolean(a, b, op)` of two filled shapes.
  const pathBoolean = (a, b, op) => {
    op = op instanceof T ? exact(op, "pathBoolean", site()) : op;
    if (!["union", "intersect", "subtract", "xor"].includes(op)) {
      throw new TypeError('pathBoolean op must be "union", "intersect", "subtract" or "xor"');
    }
    if (typeof val(b) !== "string") throw new TypeError("pathBoolean takes two paths");
    return pathOp("pathBoolean", op, [a, b]);
  };
  const zigzag = (path, options) => {
    const o = options ?? {};
    if (keys(o).some((k) => k !== "size" && k !== "ridges")) {
      throw new TypeError("zigzag(path, { size, ridges }) takes size and ridges");
    }
    return pathOp("zigzag", "zigzag", [path, o.size ?? 0, o.ridges ?? 0]);
  };

  // `bounds("key/path")`: the border box of the element with this key path as laid out, before
  // transforms, in canvas px, as `{ x, y, width, height }`. Its values are late: known after
  // layout, so they set only what paints (`style/mod.rs`) and decide nothing.
  function bounds(path) {
    path = path instanceof T ? exact(path, "bounds", site()) : path;
    if (typeof path !== "string" || path === "") {
      throw new TypeError('bounds takes a key path, as "card/title"');
    }
    const m = (k) => mk("bounds|" + k + "|" + path, []);
    return { x: m("x"), y: m("y"), width: m("width"), height: m("height") };
  }
  // `anchor("key/path", side)`: a point on that border box, `{ x, y }`.
  const SIDES = {
    center: [0.5, 0.5],
    left: [0, 0.5],
    right: [1, 0.5],
    top: [0.5, 0],
    bottom: [0.5, 1],
    "top-left": [0, 0],
    "top-right": [1, 0],
    "bottom-left": [0, 1],
    "bottom-right": [1, 1],
  };
  function anchor(path, side) {
    side = side instanceof T ? exact(side, "anchor", site()) : (side ?? "center");
    if (!Object.hasOwn(SIDES, side)) {
      throw new TypeError("anchor side must be one of " + keys(SIDES).join(", "));
    }
    const [u, v] = SIDES[side];
    const b = bounds(path);
    const along = (start, size, k) => (k === 0 ? start : __b("+", start, __b("*", size, k)));
    return { x: along(b.x, b.width, u), y: along(b.y, b.height, v) };
  }
  // `connect(a, b, { route })`: path data from point `a` to point `b`, a line, or with route
  // "cubic" a curve that leaves and reaches them level, both controls at the middle x.
  function connect(a, b, options) {
    const o = options ?? {};
    if (keys(o).some((k) => k !== "route")) {
      throw new TypeError("connect(a, b, { route }) takes route");
    }
    const route = o.route instanceof T ? exact(o.route, "connect", site()) : (o.route ?? "line");
    if (![a?.x, a?.y, b?.x, b?.y].every(numeric)) {
      throw new TypeError("connect takes two points, { x, y }");
    }
    if (route === "line") return __tpl(["M", " ", " L", " ", ""], a.x, a.y, b.x, b.y);
    if (route !== "cubic") throw new TypeError('connect route must be "line" or "cubic"');
    const mid = __b("/", __b("+", a.x, b.x), 2);
    const parts = ["M", " ", " C", " ", " ", " ", " ", " ", ""];
    return __tpl(parts, a.x, a.y, mid, a.y, mid, b.y, b.x, b.y);
  }

  // `defineLayoutStates({ state: { layoutId: { x, y, width, height } } })`: named layouts of the
  // same elements, each a box in px of its containing block. `flip(states, from, to, progress)`
  // moves each element between two of them, as its `style.layoutTransition`.
  const LAYOUTS = Symbol("layout states");
  const FLIP = Symbol("flip");
  function defineLayoutStates(states) {
    if (!plain(states) || hasT(states)) {
      throw new TypeError(
        "defineLayoutStates takes states of boxes that do not depend on the time",
      );
    }
    const names = keys(states);
    if (names.length < 2) throw new TypeError("defineLayoutStates takes two states or more");
    const ids = (name) => {
      if (!plain(states[name])) throw new TypeError("layout state " + name + " must be an object");
      return keys(states[name]).sort().join();
    };
    for (const name of names) {
      if (ids(name) !== ids(names[0])) {
        throw new TypeError("layout state " + name + " lists other layoutIds than " + names[0]);
      }
      for (const [id, r] of entries(states[name])) {
        const finite = (k) => Number.isFinite(r?.[k]);
        if (!["x", "y", "width", "height"].every(finite) || !(r.width > 0 && r.height > 0)) {
          throw new TypeError(
            "layout state " + name + " " + id + " must be { x, y, width, height }, sized over 0",
          );
        }
      }
    }
    return freeze({ [LAYOUTS]: states });
  }
  function flip(layouts, from, to, progress) {
    if (!layouts?.[LAYOUTS]) throw new TypeError("flip takes states from defineLayoutStates");
    const states = layouts[LAYOUTS];
    [from, to] = [fixed(from, "flip", site()), fixed(to, "flip", site())];
    for (const s of [from, to]) {
      if (!Object.hasOwn(states, s)) {
        throw new TypeError(
          "flip: no layout state " + s + "; there are " + keys(states).join(", "),
        );
      }
    }
    if (!numeric(progress)) throw new TypeError("flip progress must be a number");
    return freeze({ [FLIP]: { states, from, to, progress } });
  }

  // `clamp(x, min, max)` as CSS clamps: `min` wins over `max`.
  const clamp = (x, min, max) => {
    if (![x, min, max].every(numeric)) throw new TypeError("clamp(x, min, max) takes numbers");
    return math("max", [min, math("min", [x, max])]);
  };
  // `mod(x, m)`: the remainder in [0, m), where JavaScript's `%` takes the sign of `x`.
  const mod = (x, m) => {
    if (!numeric(x) || !numeric(m)) throw new TypeError("mod(x, m) takes numbers");
    return __b("%", __b("+", __b("%", x, m), m), m);
  };
  // `pingPong(x, length)`: from 0 up to `length` and back, every `2 length`.
  const pingPong = (x, length) => {
    if (!numeric(x) || !numeric(length)) {
      throw new TypeError("pingPong(x, length) takes numbers");
    }
    return __b("-", length, math("abs", [__b("-", mod(x, __b("*", length, 2)), length)]));
  };

  // `wiggle(t, { seed, frequency, amplitude, phase })`: a value drifting within ±amplitude,
  // `frequency` times a second; `wiggle2D` takes an amplitude for each axis and gives `{ x, y }`.
  function wiggleOf(name, t, options, axes) {
    const o = options ?? {};
    if (keys(o).some((k) => !WIGGLE.includes(k))) {
      throw new TypeError(name + " takes " + WIGGLE.join(", "));
    }
    const fix = (v) => (v instanceof T ? exact(v, name, site()) : v);
    const [seed, frequency, phase] = [fix(o.seed ?? 0), fix(o.frequency), fix(o.phase ?? 0)];
    let amplitude = fix(o.amplitude);
    if (axes === 2 && !isArray(amplitude)) amplitude = [amplitude, amplitude];
    const amplitudes = axes === 2 ? amplitude.map(fix) : [amplitude];
    if (!Number.isSafeInteger(seed) || seed < 0) {
      throw new TypeError(name + " seed must be a whole number, at least 0");
    }
    if (!(Number.isFinite(frequency) && frequency > 0) || !Number.isFinite(phase)) {
      throw new TypeError(name + " frequency must be a number over 0, and phase a number");
    }
    if (amplitudes.length !== axes || !amplitudes.every(Number.isFinite)) {
      throw new TypeError(
        name + " amplitude must be a number" + (axes === 2 ? ", or [x, y] numbers" : ""),
      );
    }
    if (!numeric(t)) throw new TypeError(name + " takes a number of seconds");
    return amplitudes.map((a, axis) => {
      const op = "wiggle|" + [seed, frequency, a, phase, axis].join("|");
      return isT(t) ? mk(op, [t]) : host.op(op, [t]);
    });
  }
  const WIGGLE = ["seed", "frequency", "amplitude", "phase"];
  const wiggle = (t, options) => wiggleOf("wiggle", t, options, 1)[0];
  const wiggle2D = (t, options) => {
    const [x, y] = wiggleOf("wiggle2D", t, options, 2);
    return { x, y };
  };

  // `particles(t, { seed, count, emitter, birth, lifetime, loop, velocity, gravity, forces })`:
  // `count` particles, one born every `birth.interval` seconds from 0 (around again every
  // count × interval with `loop`) at a seeded point of `emitter` ({ x, y, width, height }) with a
  // seeded velocity ({ x: [low, high], y: [low, high] } pixels a second), living `lifetime`
  // seconds. Each is `{ index, key, x, y, age, progress, alive }` at `t`, to render with `.map`.
  function particles(t, options) {
    const o = options ?? {};
    if (!plain(o) || keys(o).some((k) => !PARTICLES.includes(k))) {
      throw new TypeError("particles(t, options) takes " + PARTICLES.join(", "));
    }
    const fix = (v, what) => {
      v = v instanceof T ? exact(v, "particles", site()) : v;
      if (typeof v !== "number" || !Number.isFinite(v)) {
        throw new TypeError(
          "particles " + what + " must be a number that does not depend on the time",
        );
      }
      return v;
    };
    const box = (v, names, what) => {
      if (!plain(v) || keys(v).some((k) => !names.includes(k))) {
        throw new TypeError("particles " + what + " is { " + names.join(", ") + " }");
      }
      return names.map((n) => fix(v[n] ?? 0, what + "." + n));
    };
    const range = (r, what) => {
      if (!isArray(r) || r.length !== 2) {
        throw new TypeError("particles velocity." + what + " is [low, high]");
      }
      return r.map((x) => fix(x, "velocity." + what));
    };
    const [seed, count, lifetime] = [
      fix(o.seed ?? 0, "seed"),
      fix(o.count, "count"),
      fix(o.lifetime, "lifetime"),
    ];
    if (!Number.isSafeInteger(seed) || seed < 0) {
      throw new TypeError("particles seed must be a whole number, at least 0");
    }
    if (!Number.isInteger(count) || count < 1 || count > MAX_PARTICLES) {
      throw new TypeError("particles count must be a whole number from 1 to " + MAX_PARTICLES);
    }
    const [interval] = box(o.birth ?? {}, ["interval"], "birth");
    if (!(interval > 0 && lifetime > 0)) {
      throw new TypeError("particles birth.interval and lifetime must be over 0 seconds");
    }
    const looping = o.loop ?? false;
    if (typeof looping !== "boolean") throw new TypeError("particles loop is true or false");
    const v = o.velocity ?? {};
    if (!plain(v) || keys(v).some((k) => k !== "x" && k !== "y")) {
      throw new TypeError("particles velocity is { x: [low, high], y: [low, high] }");
    }
    const forces = o.forces ?? [];
    if (!isArray(forces) || !forces.every((f) => plain(f) && f[FORCE])) {
      throw new TypeError("particles forces is a list of curlNoise() and drag()");
    }
    const spec = JSON.stringify({
      seed,
      count,
      interval,
      loop: looping,
      lifetime,
      emitter: box(o.emitter ?? {}, ["x", "y", "width", "height"], "emitter"),
      velocity: [...range(v.x ?? [0, 0], "x"), ...range(v.y ?? [0, 0], "y")],
      gravity: box(o.gravity ?? {}, ["x", "y"], "gravity"),
      forces: forces.map((f) => f[FORCE]),
    });
    if (!numeric(t)) throw new TypeError("particles takes a number of seconds");
    const at = (index, field) => {
      const op = "particle|" + field + "|" + index + "|" + spec;
      return isT(t) ? mk(op, [t]) : host.op(op, [t]);
    };
    return Array.from({ length: count }, (_, index) =>
      freeze({
        index,
        key: "particle-" + index,
        x: at(index, "x"),
        y: at(index, "y"),
        age: at(index, "age"),
        progress: at(index, "progress"),
        alive: at(index, "alive"),
      }),
    );
  }
  const PARTICLES = [
    "seed",
    "count",
    "emitter",
    "birth",
    "lifetime",
    "loop",
    "velocity",
    "gravity",
    "forces",
  ];
  const MAX_PARTICLES = 10000;
  const FORCE = Symbol("force");

  // `curlNoise({ seed, scale, strength })`: a force swirling along a noise field's contours,
  // `scale` its features per pixel, `strength` pixels a second squared. `drag(coefficient)`:
  // a force against the velocity, `coefficient` a second.
  function curlNoise(options) {
    const o = options ?? {};
    if (!plain(o) || keys(o).some((k) => !["seed", "scale", "strength"].includes(k))) {
      throw new TypeError("curlNoise takes seed, scale, strength");
    }
    const fix = (v) => (v instanceof T ? exact(v, "curlNoise", site()) : v);
    const [seed, scale, strength] = [fix(o.seed ?? 0), fix(o.scale), fix(o.strength)];
    if (!Number.isSafeInteger(seed) || seed < 0 || !(scale > 0) || !Number.isFinite(strength)) {
      throw new TypeError("curlNoise takes a whole seed from 0, a scale over 0 and a strength");
    }
    return freeze({ [FORCE]: { curl: [seed, scale, strength] } });
  }
  function drag(coefficient) {
    const k = coefficient instanceof T ? exact(coefficient, "drag", site()) : coefficient;
    if (!(Number.isFinite(k) && k >= 0))
      throw new TypeError("drag(coefficient) takes a number from 0");
    return freeze({ [FORCE]: { drag: k } });
  }

  // `field({ from, to, progress, stagger })`: each value of `from` going to the one of `to` in
  // the same place, starting late by its delay (`stagger` × index, or `stagger[index]` in
  // [0, 1)) and arriving at progress 1. Values are numbers, or arrays or objects of numbers.
  function field(options) {
    const o = options ?? {};
    if (!plain(o) || keys(o).some((k) => !["from", "to", "progress", "stagger"].includes(k))) {
      throw new TypeError("field takes from, to, progress, stagger");
    }
    const { from, to, progress } = o;
    if (!isArray(from) || !isArray(to) || from.length !== to.length) {
      throw new TypeError("field from and to are lists of the same length");
    }
    if (!numeric(progress)) throw new TypeError("field progress must be a number");
    const n = from.length;
    let stagger = o.stagger ?? 0;
    stagger = stagger instanceof T ? exact(stagger, "field", site()) : stagger;
    const delays = isArray(stagger)
      ? stagger.map((d) => (d instanceof T ? exact(d, "field", site()) : d))
      : Array.from({ length: n }, (_, i) => i * stagger);
    if (delays.length !== n || !delays.every((d) => typeof d === "number" && d >= 0 && d < 1)) {
      throw new TypeError(
        "field stagger is a step from 0 that keeps the last delay below 1, or one delay in [0, 1) per value",
      );
    }
    const blend = (a, b, u, where) => {
      if (typeof a === "number" && typeof b === "number") return __b("+", a, __b("*", b - a, u));
      if (isArray(a) && isArray(b) && a.length === b.length) {
        return a.map((x, i) => blend(x, b[i], u, where));
      }
      if (plain(a) && plain(b) && keys(a).join() === keys(b).join()) {
        const out = {};
        for (const k of keys(a)) out[k] = blend(a[k], b[k], u, where);
        return out;
      }
      throw new TypeError(
        "field " +
          where +
          ": from and to are numbers, or arrays or objects of numbers, of one shape",
      );
    };
    return from.map((a, i) => {
      const u = clamp(__b("/", __b("-", progress, delays[i]), 1 - delays[i]), 0, 1);
      return blend(a, to[i], u, "value " + i);
    });
  }

  // `freezeTime(t, { from, to })`: the time held at `from` until `to`, later less the time held.
  function freezeTime(t, options) {
    const o = options ?? {};
    if (keys(o).some((k) => k !== "from" && k !== "to")) {
      throw new TypeError("freezeTime(seconds, { from, to }) takes only from and to");
    }
    const fix = (v) => (v instanceof T ? exact(v, "freezeTime", site()) : v);
    const [from, to] = [fix(o.from), fix(o.to)];
    if (!numeric(t) || !Number.isFinite(from) || !Number.isFinite(to) || !(from < to)) {
      throw new TypeError("freezeTime takes a number of seconds and from before to, both numbers");
    }
    const op = "freeze|" + from + "|" + to;
    return isT(t) ? mk(op, [t]) : host.op(op, [t]);
  }

  // `spring(t, options)` and `springVelocity(t, options)`: options as `spring.rs` takes them,
  // fixed values.
  const springOf = (name) => (t, options) => {
    const o = options ?? {};
    for (const k of keys(o)) {
      if (!SPRING.includes(k)) {
        throw new TypeError(name + " has no option " + k + "; it takes " + SPRING.join(", "));
      }
    }
    if (!numeric(t)) throw new TypeError(name + "(seconds, options?) takes a number of seconds");
    const spec = SPRING.map((k) => {
      const v = o[k] instanceof T ? exact(o[k], name, site()) : o[k];
      if (v === undefined) return "";
      if (typeof v !== (k === "preset" ? "string" : "number") || String(v).includes("|")) {
        throw new TypeError(
          name + " " + k + (k === "preset" ? " must be a name" : " must be a number"),
        );
      }
      return String(v);
    });
    const op = name + "|" + spec.join("|");
    return isT(t) ? mk(op, [t]) : host.op(op, [t]);
  };
  const SPRING = [
    "preset",
    "duration",
    "bounce",
    "mass",
    "stiffness",
    "damping",
    "initialVelocity",
  ];
  const spring = springOf("spring");
  const springVelocity = springOf("springVelocity");

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

  const INTRINSIC = {
    View: "view",
    Text: "text",
    Span: "span",
    Mask: "mask",
    Image: "image",
    Video: "video",
    ShaderLayer: "shader",
  };

  // Tags that are gone, with what replaces them.
  const REMOVED = {
    Scene: "return the elements directly",
    Screen: "content outside <World> is in screen space",
    Group: "use <View>",
    Flex: 'use <View className="flex">',
    Absolute: 'use <View className="absolute">',
    GeometryBatch: "render an array of elements with .map",
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
  // Valle elements that come in a later version.
  const LATER_TAGS = [
    "Scene3D",
    "Mesh",
    "Texture",
    "Anchor",
    "AmbientLight",
    "DirectionalLight",
    "HemisphereLight",
    "PointLight",
    "SpotLight",
    "RectAreaLight",
    "Particles3D",
    "Glass",
    "GlassField",
    "MathFormula",
  ];
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
    view: ["layoutId"],
    text: ["layoutId", "split", "perUnit"],
    span: [],
    mask: ["source", "mode", "invert", "layoutId"],
    image: ["src", "layoutId"],
    video: ["src", "trimStart", "rate", "time", "layoutId"],
    shader: ["code", "source", "uniforms", "inputs", "outset", "layoutId", "__transition"],
  };

  const fixed = (x, kind, at) => (x instanceof T ? exact(x, kind, at) : x);

  function styleOf(style, tag, at, layoutId) {
    if (style === undefined || style === null) return [];
    if (!plain(style)) fail(at, "<" + tag + "> style must be an object");
    const out = [];
    for (const [k, v] of entries(style)) {
      if (k === "layoutTransition") {
        out.push(...flipped(v, tag, at, layoutId));
        continue;
      }
      if (k === "motionBlur") {
        if (v !== undefined && v !== null) out.push(["motionBlur", trail(v, tag, at)]);
        continue;
      }
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
      out.push([k, renamed(k, v)]);
    }
    return out;
  }

  // `motionBlur: { velocity: { x, y }, shutterAngle }` as the trail the element makes while the
  // shutter is open, `x y` in its own px: the velocity in px a second of the scene's clock, times
  // the rates of the TimeScopes around, times the shutter's part of a frame (180° by default).
  // `"auto"` stays, for the scene to measure.
  function trail(v, tag, at) {
    if (fixed(v, "motionBlur", at) === "auto") return "auto";
    const form = "<" + tag + '> motionBlur is "auto" or { velocity: { x, y }, shutterAngle }';
    if (!plain(v)) fail(at, form);
    for (const k of keys(v)) if (k !== "velocity" && k !== "shutterAngle") fail(at, form);
    const { velocity } = v;
    const angle = fixed(v.shutterAngle ?? 180, "shutterAngle", at);
    if (!plain(velocity) || !numeric(velocity.x) || !numeric(velocity.y)) fail(at, form);
    if (!(Number.isFinite(angle) && angle >= 0 && angle <= 360)) {
      fail(at, "<" + tag + "> motionBlur shutterAngle is degrees from 0 to 360");
    }
    const exposure = (G.rate * angle) / 360 / G.root.fps;
    return __tpl(["", " ", ""], __b("*", velocity.x, exposure), __b("*", velocity.y, exposure));
  }

  // An element's `layoutId`, a name written as a literal.
  function layoutIdOf(props, tag, at) {
    const id = fixed(props.layoutId, "layoutId", at);
    if (id !== undefined && (typeof id !== "string" || id === "")) {
      fail(at, "<" + tag + "> layoutId must be a name");
    }
    return id;
  }

  // `layoutTransition: flip(…)` as the styles that move the element from its box in one state
  // to its box in the other: laid out absolute at `to`, and translated and scaled back from
  // `from` about its top left as the progress goes from 0 to 1. A size never crosses 0.
  function flipped(f, tag, at, id) {
    if (!f || !f[FLIP]) fail(at, "<" + tag + "> style layoutTransition takes flip(…)");
    if (id === undefined) fail(at, "<" + tag + "> style layoutTransition needs a layoutId");
    const { states, from, to, progress } = f[FLIP];
    if (!Object.hasOwn(states[from], id)) {
      fail(at, "<" + tag + "> layoutId " + id + " is in no layout state");
    }
    const [a, b] = [states[from][id], states[to][id]];
    const back = (d) => __b("*", d, __b("-", 1, progress));
    const ratio = (s, t) => math("max", [0, __b("+", 1, back(s / t - 1))]);
    return [
      ["position", "absolute"],
      ["left", b.x],
      ["top", b.y],
      ["width", b.width],
      ["height", b.height],
      ["transformOrigin", "0 0"],
      ["translate", __tpl(["", "px ", "px"], back(a.x - b.x), back(a.y - b.y))],
      ["scale", __tpl(["", " ", ""], ratio(a.width, b.width), ratio(a.height, b.height))],
    ];
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
        if (c.type === "TimeScope") {
          return scoped(c.props, here, () => c.children.forEach((x) => add(x, here)));
        }
        if (["Echo", "Shutter", "World", "Transition"].includes(c.type)) {
          fail(here, "<" + c.type + "> cannot be inside <" + tag + ">: put the <Text> in it");
        }
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

  // A <Text split perUnit>'s content: its text split into units by `split` (`host.units`), each
  // a unit node in the style `perUnit(unit)` gives it, and the white space between them. A unit
  // is { index, count, start, end, text }, start and end UTF-16 indexes into the text. Which
  // units there are cannot depend on time.
  function splitOf(node, props, children, tag, at) {
    const split = fixed(props.split, "split", at);
    if (split !== "char" && split !== "word" && split !== "line") {
      fail(at, "<" + tag + '> split is "char", "word" or "line"');
    }
    if (typeof props.perUnit !== "function") {
      fail(at, "<" + tag + "> split needs perUnit, a function of a unit that returns its style");
    }
    let text = "";
    const add = (c) => {
      if (c === null || c === undefined || typeof c === "boolean") return;
      if (isArray(c)) return c.forEach(add);
      if (typeof c === "object" && !isT(c)) {
        fail(at, "<" + tag + " split> holds text only, not <Span>s or other elements yet");
      }
      text += String(fixed(c, "split", at));
    };
    children.forEach(add);
    const ends = host.units(text, split);
    const count = ends.length / 2;
    const content = [];
    let last = 0;
    for (let index = 0; index < count; index++) {
      const [start, end] = [ends[2 * index], ends[2 * index + 1]];
      if (start > last) content.push({ text: text.slice(last, start) });
      const unit = freeze({ index, count, start, end, text: text.slice(start, end) });
      if (++G.nodes > MAX_NODES) fail(at, "a frame has more than " + MAX_NODES + " elements");
      const u = {
        type: "unit",
        path: null,
        classes: [],
        style: styleOf(props.perUnit(unit), tag, at),
        children: [],
        site: at,
        content: [{ text: unit.text }],
      };
      node.children.push(u);
      content.push({ span: u });
      last = end;
    }
    if (last < text.length) content.push({ text: text.slice(last) });
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

  // The image `src` names: a resource by its name, or a picture the Timeline gives as
  // `input://name`; null for anything else.
  function pictureOf(src) {
    if (typeof src !== "string") return null;
    if (src.startsWith("resource://")) return named(src.slice("resource://".length));
    if (src.startsWith("input://")) return inputNamed(src.slice("input://".length));
    return null;
  }

  // An <Image>'s resource, or the picture it shows, by the name its src gives as a literal:
  // which files a scene reads cannot depend on time.
  function imageOf(node, props, children, at) {
    const src = props.src;
    if (src instanceof T) {
      fail(at, '<Image> src is written as a literal, such as "resource://logo"; it cannot change');
    }
    const name = pictureOf(src);
    if (name === null) {
      fail(
        at,
        '<Image> src is a resource declared in controls.resources, such as "resource://logo", ' +
          'or a picture the Timeline gives, such as "input://source", not ' +
          stringify(src),
      );
    }
    if (children.length > 0) fail(at, "<Image> has no children");
    node.src = name;
  }

  // A <Video>'s resource, by the name its src gives as a literal, and the time it shows, in
  // seconds into the video: `time`, or `trimStart` plus the clock times `rate`.
  function videoOf(node, props, children, at) {
    const src = props.src;
    if (src instanceof T || typeof src !== "string" || !src.startsWith("resource://")) {
      fail(
        at,
        "<Video> src is a video resource declared in controls.resources, written as a literal, " +
          'such as "resource://clip"',
      );
    }
    if (children.length > 0) fail(at, "<Video> has no children");
    node.src = named(src.slice("resource://".length));
    const number = (x, what) => {
      if (!(x instanceof T) && !Number.isFinite(x)) fail(at, "<Video> " + what + " is a number");
      return x;
    };
    if (props.time !== undefined) {
      if (props.trimStart !== undefined || props.rate !== undefined) {
        fail(at, "<Video> takes time, or trimStart and rate, not both");
      }
      node.time = number(props.time, "time");
      return;
    }
    const trimStart = number(props.trimStart ?? 0, "trimStart");
    const rate = number(props.rate ?? 1, "rate");
    node.time = __b("+", trimStart, __b("*", G.seconds, rate));
  }

  // A uniform's value as parts, numbers, booleans, colors or traced values: checked.
  function uniformOf(parts, what, at) {
    for (const p of parts) {
      if (!(p instanceof T) && !["number", "boolean", "string"].includes(typeof p)) {
        fail(at, what + " is a number, a boolean, a color or an array of numbers");
      }
    }
    return parts;
  }

  // <Transition kind progress params>, or <Transition code uniforms progress>: a box of its two
  // children, one over the other, isolated, which a shader goes from the first to the second
  // by `progress`, 0 to 1. A built-in `kind` (fade by default) takes `params`; `code` is a
  // transition in gl-transitions' form, which takes `uniforms`. Kind and code cannot change.
  const TRANSITION = ["kind", "progress", "params", "code", "uniforms", "style", "className"];
  function transitionOf(props, children, out, at) {
    if (G.svg > 0) fail(at, "<Transition> cannot be inside an <svg>");
    for (const k of keys(props)) {
      if (k !== "key" && !TRANSITION.includes(k)) {
        fail(at, "<Transition> has no prop " + k + "; it takes " + TRANSITION.join(", "));
      }
    }
    const shown = [];
    const gather = (c) => {
      if (isArray(c)) return c.forEach(gather);
      if (c === null || c === undefined || typeof c === "boolean") return;
      if (typeof c === "string" && c.trim() === "") return;
      shown.push(c);
    };
    gather(children);
    if (shown.length !== 2) {
      fail(at, "<Transition> takes two children: what it goes from, then what it goes to");
    }
    const custom = props.code !== undefined;
    if (custom && (props.kind !== undefined || props.params !== undefined)) {
      fail(at, "<Transition> takes a kind with params, or code with uniforms, not both");
    }
    if (!custom && props.uniforms !== undefined) {
      fail(at, "<Transition> uniforms go with code; a kind takes params");
    }
    const kind = custom ? null : fixed(props.kind ?? "fade", "kind", at);
    const code = custom ? fixed(props.code, "code", at) : null;
    if (custom ? typeof code !== "string" : typeof kind !== "string") {
      fail(at, custom ? "<Transition> code is a string" : "<Transition> kind is a string");
    }
    const given = (custom ? props.uniforms : props.params) ?? {};
    const what = custom ? "uniforms" : "params";
    if (!plain(given)) fail(at, "<Transition> " + what + " is an object of values by name");
    const uniforms = [["progress", [props.progress ?? 0]]];
    for (const [k, v] of entries(given)) {
      if (k === "progress") fail(at, "<Transition> progress is a prop of its own, not in " + what);
      const parts = Array.isArray(v) ? v : [v];
      uniforms.push([k, uniformOf(parts, "<Transition> " + what + " " + k, at)]);
    }
    // Each child in a cell of its own over the whole box, which it fills as a grid item.
    const fill = {
      display: "grid",
      gridTemplateColumns: "minmax(0, 1fr)",
      gridTemplateRows: "minmax(0, 1fr)",
    };
    const cell = (c) => ({
      __el: true,
      type: "View",
      props: { style: { ...fill, gridArea: "1 / 1", isolation: "isolate" } },
      children: [c],
      site: at,
    });
    const box = {
      style: { ...props.style, ...fill },
      __transition: { kind, code, uniforms },
    };
    if (props.key !== undefined) box.key = props.key;
    if (props.className !== undefined) box.className = props.className;
    element("Transition", box, shown.map(cell), out, at);
  }

  // A <ShaderLayer>'s GLSL code, which cannot change, or the shader resource it is in, written as
  // a literal; the values its uniforms take by name, each
  // a number, a boolean, a color or an array of numbers for a vector, any of them traced, the
  // image resources it reads by name, written as literals, and the px it paints out past its box.
  function shaderOf(node, props, at) {
    if (props.__transition) {
      const { kind, code, uniforms } = props.__transition;
      node.shader = { kind, code, source: null, uniforms, inputs: [], outset: 0, transition: true };
      return;
    }
    const code = props.code === undefined ? null : fixed(props.code, "code", at);
    let source = null;
    if (props.source !== undefined) {
      const src = props.source;
      if (code !== null) fail(at, "<ShaderLayer> takes code or source, not both");
      if (typeof src !== "string" || !src.startsWith("resource://")) {
        fail(
          at,
          '<ShaderLayer> source is a shader resource written as a literal, such as "resource://wave"',
        );
      }
      source = named(src.slice("resource://".length));
    } else if (typeof code !== "string") {
      fail(
        at,
        "<ShaderLayer> code is its shader's GLSL, a string, or source names a shader resource",
      );
    }
    const given = props.uniforms ?? {};
    if (!plain(given)) {
      fail(at, "<ShaderLayer> uniforms is an object of the values its uniforms take, by name");
    }
    const uniforms = [];
    for (const [k, v] of entries(given)) {
      const parts = Array.isArray(v) ? v : [v];
      uniforms.push([k, uniformOf(parts, "<ShaderLayer> uniforms " + k, at)]);
    }
    const read = props.inputs ?? {};
    if (!plain(read)) fail(at, "<ShaderLayer> inputs is an object of image resources, by name");
    const inputs = [];
    for (const [k, src] of entries(read)) {
      const name = pictureOf(src);
      if (name === null) {
        fail(
          at,
          "<ShaderLayer> inputs " +
            k +
            " is an image resource or a picture the Timeline gives, written as a literal, such " +
            'as "resource://noise" or "input://source"',
        );
      }
      inputs.push([k, name]);
    }
    const outset = fixed(props.outset ?? 0, "outset", at);
    if (typeof outset !== "number" || !(outset >= 0) || outset === Infinity) {
      fail(at, "<ShaderLayer> outset is px it paints past its box, 0 or more");
    }
    node.shader = { code, source, uniforms, inputs, outset };
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

  function element(type, props, children, out, at, inText, inside) {
    if (typeof type !== "string") fail(at, "an element type must be a tag or a component function");
    if (type in SVG) return svgElement(type, props, children, out, at);
    if (G.svg > 0) {
      if (SVG_SILENT.includes(type)) return;
      if (type in SVG_NEVER) fail(at, "<" + type + "> is not supported: " + SVG_NEVER[type]);
      if (INTRINSIC[type]) fail(at, "<" + type + "> cannot be inside an <svg>");
      fail(at, "<" + type + "> in an <svg> is not supported yet");
    }
    const kind = type === "Transition" ? "shader" : INTRINSIC[type];
    if (!kind) {
      if (type in REMOVED) fail(at, "<" + type + "> is removed: " + REMOVED[type]);
      if (type in HTML) {
        fail(at, "HTML tags are not supported: use <" + HTML[type] + "> instead of <" + type + ">");
      }
      if (LATER_TAGS.includes(type)) fail(at, "<" + type + "> is not supported yet");
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
      style: styleOf(props.style, type, at, layoutIdOf(props, type, at)),
      children: [],
      site: at,
    };
    if (kind === "mask") maskOf(node, props, at);
    if (kind === "image") imageOf(node, props, children, at);
    if (kind === "video") videoOf(node, props, children, at);
    if (kind === "shader") shaderOf(node, props, at);
    if (key !== null) G.keys.push(key);
    try {
      if (kind === "text" && (props.split !== undefined || props.perUnit !== undefined)) {
        splitOf(node, props, children, type, at);
      } else if (kind === "text" || kind === "span") {
        contentOf(node, children, type, at);
      } else if (inside !== undefined) {
        inside(node.children);
      } else {
        if (kind === "mask") render(props.source, node.source.children, at);
        for (const c of children) render(c, node.children, at);
      }
    } finally {
      if (key !== null) G.keys.pop();
    }
    out.push(node);
  }

  const progressOf = (seconds) => clamp(__b("/", seconds, G.composition.duration), 0, 1);

  // Runs `f` under a <TimeScope>'s clock: `(seconds − offset) × rate` in ticks, rounded down, and
  // `progress` from it. Like a component, it adds no element and a key is a segment of key paths.
  // In the scene a Timeline lowers to, a clip's TimeScope names it in the clip table instead.
  function scoped(props, at, f, out) {
    if (props.clip !== undefined && G.composition.timeline) return clipped(props, at, f, out);
    for (const k of keys(props)) {
      if (k !== "key" && k !== "offset" && k !== "rate") {
        fail(at, "<TimeScope> has no prop " + k + "; it takes offset and rate");
      }
    }
    const offset = fixed(props.offset ?? 0, "offset", at);
    const rate = fixed(props.rate ?? 1, "rate", at);
    if (!Number.isFinite(offset) || !Number.isFinite(rate)) {
      fail(at, "<TimeScope> offset (seconds) and rate are numbers");
    }
    const key = props.key === undefined ? null : String(fixed(props.key, "key", at));
    const op = "scope|" + offset + "|" + rate;
    const outer = [G.seconds, G.progress, G.rate];
    G.seconds = mk(op, [G.seconds]);
    G.progress = progressOf(G.seconds);
    G.scopes.push(op);
    G.rate *= rate;
    if (key !== null) G.keys.push(key);
    try {
      f();
    } finally {
      if (key !== null) G.keys.pop();
      G.scopes.pop();
      [G.seconds, G.progress, G.rate] = outer;
    }
  }

  // A Timeline clip, `<TimeScope clip={id} composition style resources inputs>`, which only the
  // lowering writes: a box styled `style` that shows while the clip is on, its children on the
  // clip's time from the clip table; `composition` is what useComposition() gives inside
  // (`{ width, height, duration, clipDuration }`, seconds), and `resources` and `inputs` name the
  // scene's resources and pictures that the names of the component inside stand for; with
  // `picture`, the box draws that picture of the scene instead of showing where it is.
  function clipped(props, at, f, out) {
    const id = props.clip;
    const c = props.composition;
    const op = (part) => "clip|" + part + "|" + id;
    const on = mk(op("on"), [G.seconds]);
    const local = mk(op("local"), [G.seconds]);
    const outer = [G.seconds, G.progress, G.clip, G.composition, G.names, G.inputs];
    if (c !== undefined) {
      G.composition = { ...G.composition, ...c };
    }
    G.seconds = mk(op("time"), [G.seconds]);
    G.progress = progressOf(G.seconds);
    G.clip = [local, mk(op("progress"), [outer[0]])];
    G.scopes.push(op("time"));
    if (props.resources !== undefined) G.names = props.resources;
    if (props.inputs !== undefined) G.inputs = props.inputs;
    const style = { ...props.style, display: mk("select", [on, "flex", "none"]) };
    try {
      element("View", { key: props.key, style }, [], out, at, false, f);
      if (props.picture !== undefined) out[out.length - 1].picture = props.picture;
    } finally {
      G.scopes.pop();
      [G.seconds, G.progress, G.clip, G.composition, G.names, G.inputs] = outer;
    }
  }

  // A resource's name in the scene: in a Timeline clip, the scene's resource that the
  // component's own name stands for.
  const named = (name) => (G.names !== null && Object.hasOwn(G.names, name) ? G.names[name] : name);

  // A picture's name, `input://name`: in a Timeline clip, the picture of the scene it stands
  // for.
  const inputNamed = (name) =>
    G.inputs !== null && Object.hasOwn(G.inputs, name)
      ? "input://" + G.inputs[name]
      : "input://" + name;

  // Resource names in a style's text, `url(resource://name)` and a font family's, as the scene
  // names them.
  function renamed(k, v) {
    if (G.names === null || typeof v !== "string") return v;
    v = v.replace(/resource:\/\/([A-Za-z0-9._-]+)/g, (_, n) => "resource://" + named(n));
    if (k === "fontFamily") {
      v = v
        .split(",")
        .map((f) => {
          const name = f.trim().replace(/^["']|["']$/g, "");
          return Object.hasOwn(G.names, name) ? G.names[name] : f.trim();
        })
        .join(", ");
    }
    return v;
  }

  // Runs `f` at another output time, the root clock through the op `sample`: the clip clock is
  // there, and the scene clock is it through the TimeScopes around.
  function sampled(sample, f) {
    const outer = [G.seconds, G.progress, G.clip];
    const root = mk(sample, [G.clip[0]]);
    G.clip = [root, progressOf(root)];
    G.seconds = G.scopes.reduce((t, op) => mk(op, [t]), root);
    G.progress = progressOf(G.seconds);
    G.sampling = true;
    try {
      f();
    } finally {
      G.sampling = false;
      [G.seconds, G.progress, G.clip] = outer;
    }
  }

  const SAMPLE = Symbol("sample");
  // Output times a frame may sample across its <Echo>s and <Shutter>s.
  const MAX_SAMPLES = 128;

  // <Echo count delay decay> and <Shutter samples angle>: copies of the children at other output
  // times, stacked in one grid cell of an isolated box. Echo paints the copy `count × delay`
  // seconds back first at opacity `decay^count`, newer ones over it, and the children at the
  // frame's time last. Shutter adds its copies at `1 / samples` opacity (plus-lighter), which
  // averages them.
  function sampling(type, props, children, out, at) {
    if (G.svg > 0) fail(at, "<" + type + "> cannot be inside an <svg>");
    if (G.sampling) fail(at, "<" + type + "> cannot be inside an <Echo> or a <Shutter>");
    const echo = type === "Echo";
    const names = echo ? ["count", "delay", "decay"] : ["samples", "angle"];
    for (const k of keys(props)) {
      if (k !== "key" && !names.includes(k)) {
        fail(at, "<" + type + "> has no prop " + k + "; it takes " + names.join(", "));
      }
    }
    const get = (k, d) => fixed(props[k] ?? d, k, at);
    const whole = (v, max) => Number.isInteger(v) && v >= 1 && v <= max;
    const c = G.root;
    const tail = "|" + c.ticks + "|" + c.looped;
    const cells = [];
    const cell = (style, sample) => {
      const inner =
        sample === null
          ? children
          : [{ __el: true, type: SAMPLE, props: { sample }, children, site: at }];
      cells.push({
        __el: true,
        type: "View",
        props: { style: { gridArea: "1 / 1", ...style } },
        children: inner,
        site: at,
      });
    };
    if (echo) {
      const [count, delay, decay] = [get("count"), get("delay"), get("decay")];
      if (!whole(count, 32)) fail(at, "<Echo> count is a whole number from 1 to 32");
      if (!(Number.isFinite(delay) && delay > 0))
        fail(at, "<Echo> delay is a number of seconds above 0");
      if (!(Number.isFinite(decay) && decay >= 0 && decay <= 1)) {
        fail(at, "<Echo> decay is a number from 0 to 1");
      }
      for (let i = count; i >= 1; i--)
        cell({ opacity: decay ** i }, "echo|" + i + "|" + delay + tail);
      cell({}, null);
    } else {
      const [n, angle] = [get("samples"), get("angle", 180)];
      if (!whole(n, 32)) fail(at, "<Shutter> samples is a whole number from 1 to 32");
      if (!(Number.isInteger(angle) && angle >= 0 && angle <= 360)) {
        fail(at, "<Shutter> angle is a whole number of degrees from 0 to 360");
      }
      const style = n === 1 ? {} : { opacity: 1 / n, mixBlendMode: "plus-lighter" };
      for (let k = 0; k < n; k++) {
        cell(style, "shutter|" + k + "|" + n + "|" + angle + "|" + c.rate + tail);
      }
    }
    G.samples += cells.length;
    if (G.samples > MAX_SAMPLES) {
      fail(at, "a frame samples more than " + MAX_SAMPLES + " times across <Echo>s and <Shutter>s");
    }
    const style = { display: "grid", isolation: "isolate" };
    const box = { key: props.key, style };
    if (props.key === undefined) delete box.key;
    render({ __el: true, type: "View", props: box, children: cells, site: at }, out, at);
  }

  // <World camera={{ center, zoom, rotation }}>: a box over the canvas, laid out as the canvas is,
  // whose painting the camera moves: `center` (canvas px, the canvas's center by default) comes to
  // the canvas's center, scaled by `zoom` and turned by `rotation` degrees, so the world turns the
  // other way. Layout stays where it is.
  function world(props, children, out, at) {
    if (G.svg > 0) fail(at, "<World> cannot be inside an <svg>");
    for (const k of keys(props)) {
      if (k !== "key" && k !== "camera") fail(at, "<World> has no prop " + k + "; it takes camera");
    }
    const camera = props.camera ?? {};
    if (!plain(camera)) fail(at, "<World> camera is { center, zoom, rotation }");
    for (const k of keys(camera)) {
      if (k !== "center" && k !== "zoom" && k !== "rotation") {
        fail(at, "<World> camera has no field " + k + "; it takes center, zoom and rotation");
      }
    }
    const { width, height } = G.composition;
    const center = camera.center ?? { x: width / 2, y: height / 2 };
    const [zoom, rotation] = [camera.zoom ?? 1, camera.rotation ?? 0];
    if (!plain(center) || !numeric(center.x) || !numeric(center.y)) {
      fail(at, "<World> camera center is a point, { x, y }");
    }
    if (!numeric(zoom) || !numeric(rotation)) {
      fail(at, "<World> camera zoom and rotation (degrees) are numbers");
    }
    const minus = (x) => __b("-", 0, x);
    const transform = __tpl(
      ["translate(", "px, ", "px) rotate(", "deg) scale(", ") translate(", "px, ", "px)"],
      width / 2,
      height / 2,
      minus(rotation),
      zoom,
      minus(center.x),
      minus(center.y),
    );
    const style = { position: "fixed", inset: 0, transformOrigin: "0 0", transform };
    const box = props.key === undefined ? { style } : { key: props.key, style };
    render({ __el: true, type: "View", props: box, children, site: at }, out, at);
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
    if (type === "TimeScope") {
      return scoped(
        props,
        here,
        (inner) => children.forEach((c) => render(c, inner ?? out, here)),
        out,
      );
    }
    if (type === SAMPLE) {
      return sampled(props.sample, () => children.forEach((c) => render(c, out, here)));
    }
    if (type === "Echo" || type === "Shutter") return sampling(type, props, children, out, here);
    if (type === "World") return world(props, children, out, here);
    if (type === "Transition" && !props.__transition) {
      return transitionOf(props, children, out, here);
    }
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
      if ("time" in n) out.time = ref(n.time);
      if ("picture" in n) out.picture = n.picture;
      if ("shader" in n) {
        const { code, source, uniforms, inputs, outset } = n.shader;
        out.shader = {
          code,
          source,
          kind: n.shader.kind ?? null,
          transition: n.shader.transition ?? false,
          uniforms: uniforms.map(([k, v]) => [k, v.map(ref)]),
          inputs,
          outset,
        };
      }
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

  // What the built-in charts module takes from the runtime: whether a value is traced, its
  // concrete value fixed by a guard, a color as d3 reads one, and text joined from parts as one
  // template.
  const __charts = freeze({
    traced: isT,
    fixed: escape,
    color: host.color,
    concat: (parts) => (parts.some(isT) ? template(parts) : parts.join("")),
  });

  // A built-in module's exports, as globals.
  function __builtin(exports) {
    Object.assign(globalThis, exports);
  }

  function __import(key) {
    const factory = factories.get(key);
    if (!instances.has(factory)) instances.set(factory, freeze(factory()));
    return instances.get(factory);
  }

  // Traces the scene `entry` at a time; returns the variant as JSON.
  function __trace(entry, seconds, progress, composition, props) {
    G = {
      guards: [],
      when: [],
      keys: [],
      ops: 0,
      nodes: 0,
      // How deep in <svg> elements tracing is.
      svg: 0,
      composition,
      // The scene's own: Echo and Shutter sample its clock, inside clips as well.
      root: composition,
      // In a Timeline clip, the scene's resources and pictures that a component's names stand for.
      names: null,
      inputs: null,
      seconds: new T("seconds", [], seconds, false),
      progress: new T("progress", [], progress, false),
    };
    // A TimeScope changes the clock under it, not the clip's.
    G.clip = [G.seconds, G.progress];
    // The ops of the TimeScopes tracing is in, outermost first, and their rates' product.
    G.scopes = [];
    G.rate = 1;
    // Whether tracing is in an <Echo> or a <Shutter>, and how many times it has sampled.
    G.sampling = false;
    G.samples = 0;
    instances = new Map();
    try {
      const scene = __import(entry);
      if (typeof scene.default !== "function") {
        throw new TypeError("the scene's default export must be its root component, a function");
      }
      const roots = [];
      render(__h(scene.default, parse(props)), roots, -1);
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

  // Valle functions that come in a later version.
  const LATER = [
    "particles3d",
    "project3d",
    "extrude",
    "lathe",
    "tube",
    "morphSequence",
    "noiseDisplace",
    "simplify",
    "resamplePath",
    "reversePath",
    "strokeToPath",
    "pathTrajectory",
    "fitText",
  ];

  // Functions of Valle's earlier engine that are gone, with what replaces them.
  const GONE = {
    TAU: "write 2 * Math.PI",
    deg: "convert with Math.PI: degrees * Math.PI / 180 are radians",
    rad: "convert with Math.PI: degrees * Math.PI / 180 are radians",
    fract: "write x - Math.floor(x)",
    stage: "write the stage as an object in defineSequence()",
    trail: "use staggerProgress(), or clamp(p + i * gap, 0, 1)",
    defineRepeater: "use Array.from({ length: n }, (_, i) => …)",
    motionPath: "use the CSS offsetPath, offsetDistance and offsetRotate: auto",
    follow: "use the CSS offsetPath, offsetDistance and offsetRotate: auto",
    autoRotate: "use the CSS offsetRotate: auto",
    pathTemplate: "write the path data as a template string",
    morphPath: "use morph()",
    boolean: "use pathBoolean()",
    offsetPath: "use inflatePath(); offsetPath is the CSS property",
    linearGradient: "write a CSS gradient, or an SVG <linearGradient>",
    radialGradient: "write a CSS gradient, or an SVG <radialGradient>",
    gradientStop: "write a CSS gradient, or an SVG <stop>",
    point: "write { x, y }",
    rect: "write { x, y, width, height }",
    path: "path data is the string itself",
    areaBand: "use area() with y0 and y1",
    sector: "use arc()",
    curve: "use line() with a curve such as curveMonotoneX",
    cubic: "write the path data as a template string",
    niceDomain: "use a scale's nice()",
    extentOrDefault: "use extent(values) ?? fallback",
    geoProject: "use a projection, as proj([lon, lat])",
    displacement: "write displacement(…) in filter",
    motionBlur: "write motionBlur as an object { velocity, shutterAngle }",
  };
  for (const name of MATH) GONE[name] ??= "use Math." + name + "()";

  // Free names the scene reads: a capitalized one is a tag (an unknown one fails when rendered,
  // with its replacement where it has one); the gone hooks throw with theirs.
  function __declareFree(names) {
    for (const name of names) {
      if (name in globalThis) continue;
      if (name in GONE) {
        defineProperty(globalThis, name, {
          get() {
            throw new TypeError(name + " is removed: " + GONE[name]);
          },
        });
      } else if (LATER.includes(name)) {
        globalThis[name] = () => {
          throw new TypeError(name + "() is not supported yet");
        };
      } else if (name in UNAVAILABLE) {
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
    for (const name of [
      "__host",
      "__declareFree",
      "__define",
      "__builtin",
      "__charts",
      "__lockdown",
    ]) {
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
    __builtin,
    __charts,
    __import,
    __trace,
    __declareFree,
    __lockdown,
    useCurrentTime,
    useComposition,
    interpolate,
    spring,
    springVelocity,
    defineSequence,
    freezeTime,
    simulate,
    wiggle,
    wiggle2D,
    particles,
    curlNoise,
    drag,
    field,
    seededRandom,
    pathLength,
    pointAt,
    tangentAt,
    roundCorners,
    puckerBloat,
    twist,
    inflatePath,
    pathBoolean,
    morph,
    zigzag,
    bounds,
    anchor,
    connect,
    defineLayoutStates,
    flip,
    formatNumber,
    formatPercent,
    padNumber,
    noise1d,
    rangeSelector,
    measureText,
    textOutline,
    noise2d,
    clamp,
    mod,
    pingPong,
    stageProgress,
    staggerProgress,
    repeatProgress,
    yoyoProgress,
    View: "View",
    Text: "Text",
    Span: "Span",
    Mask: "Mask",
    Image: "Image",
    ShaderLayer: "ShaderLayer",
    Transition: "Transition",
    TimeScope: "TimeScope",
    Echo: "Echo",
    Shutter: "Shutter",
    World: "World",
    resource,
    number: propKind("number"),
    string: propKind("string"),
    boolean: propKind("boolean"),
    color: propKind("color"),
    select: propKind("select"),
    point: propKind("point"),
    length: propKind("length"),
    angle: propKind("angle"),
    rect: propKind("rect"),
    overlay: roleOf("overlay"),
    transition: roleOf("transition"),
    effect: roleOf("effect"),
    captionPresenter: roleOf("captionPresenter"),
  });
})();
