// Charts: the subset of d3 v7 that scenes call as built-ins, by d3's names and with d3's results
// (design §2.8.8), and Valle's own label placing and graph layout. It is scene code to the
// frontend: rewritten for tracing as a scene's modules are, so a value that depends on the time
// goes through it as ops. It keeps no state.
//
// Ported from d3 7.9.0 (https://d3js.org): Copyright 2010-2023 Mike Bostock, under the ISC
// License: Permission to use, copy, modify, and/or distribute this software for any purpose with
// or without fee is hereby granted, provided that the above copyright notice and this permission
// notice appear in all copies. THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL
// WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
// FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR
// CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
// WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

// What the runtime gives: whether a value is traced, its concrete value fixed by a guard, and a
// color as d3 reads one, `[r, g, b, opacity]`. A traced value is never null, so where d3 tests
// for null or NaN, a traced one goes on as a number.
const { traced, fixed, color: parseColor, concat } = __charts;
const missing = (x) => !traced(x) && x == null;

// ------------------------------------------------------------------------------ d3-array

const e10 = Math.sqrt(50);
const e5 = Math.sqrt(10);
const e2 = Math.sqrt(2);

function tickSpec(start, stop, count) {
  const step = (stop - start) / Math.max(0, count);
  const power = Math.floor(Math.log10(step));
  const error = step / Math.pow(10, power);
  const factor = error >= e10 ? 10 : error >= e5 ? 5 : error >= e2 ? 2 : 1;
  let i1, i2, inc;
  if (power < 0) {
    inc = Math.pow(10, -power) / factor;
    i1 = Math.round(start * inc);
    i2 = Math.round(stop * inc);
    if (i1 / inc < start) ++i1;
    if (i2 / inc > stop) --i2;
    inc = -inc;
  } else {
    inc = Math.pow(10, power) * factor;
    i1 = Math.round(start / inc);
    i2 = Math.round(stop / inc);
    if (i1 * inc < start) ++i1;
    if (i2 * inc > stop) --i2;
  }
  if (i2 < i1 && 0.5 <= count && count < 2) return tickSpec(start, stop, count * 2);
  return [i1, i2, inc];
}

export function ticks(start, stop, count) {
  [start, stop, count] = [+start, +stop, +count];
  if (!(count > 0)) return [];
  if (start === stop) return [start];
  const reverse = stop < start;
  const [i1, i2, inc] = reverse ? tickSpec(stop, start, count) : tickSpec(start, stop, count);
  if (!(i2 >= i1)) return [];
  const n = i2 - i1 + 1;
  const out = [];
  if (reverse) {
    if (inc < 0) for (let i = 0; i < n; ++i) out.push((i2 - i) / -inc);
    else for (let i = 0; i < n; ++i) out.push((i2 - i) * inc);
  } else {
    if (inc < 0) for (let i = 0; i < n; ++i) out.push((i1 + i) / -inc);
    else for (let i = 0; i < n; ++i) out.push((i1 + i) * inc);
  }
  return out;
}

export function tickIncrement(start, stop, count) {
  return tickSpec(+start, +stop, +count)[2];
}

export function tickStep(start, stop, count) {
  [start, stop, count] = [+start, +stop, +count];
  const reverse = stop < start;
  const inc = reverse ? tickIncrement(stop, start, count) : tickIncrement(start, stop, count);
  return (reverse ? -1 : 1) * (inc < 0 ? 1 / -inc : inc);
}

export function nice(start, stop, count) {
  let prestep;
  while (true) {
    const step = tickIncrement(start, stop, count);
    if (step === prestep || step === 0 || !isFinite(step)) {
      return [start, stop];
    } else if (step > 0) {
      start = Math.floor(start / step) * step;
      stop = Math.ceil(stop / step) * step;
    } else if (step < 0) {
      start = Math.ceil(start * step) / step;
      stop = Math.floor(stop * step) / step;
    }
    prestep = step;
  }
}

export function extent(values, valueof) {
  let min;
  let max;
  let index = -1;
  for (let value of values) {
    if (valueof !== undefined) value = valueof(value, ++index, values);
    if (value != null) {
      if (min === undefined) {
        // oxlint-disable-next-line const-comparisons -- false for NaN
        if (value >= value) min = max = value;
      } else {
        if (min > value) min = value;
        if (max < value) max = value;
      }
    }
  }
  return [min, max];
}

// Where `x` goes in sorted `a`: past every value not above it.
function bisectRight(a, x, lo = 0, hi = a.length) {
  if (lo < hi) {
    // oxlint-disable-next-line const-comparisons -- false for NaN
    if (!traced(x) && !(x >= x)) return hi;
    do {
      const mid = (lo + hi) >>> 1;
      if (a[mid] <= x) lo = mid + 1;
      else hi = mid;
    } while (lo < hi);
  }
  return lo;
}

function sequence(n) {
  const out = [];
  for (let i = 0; i < n; i++) out.push(i);
  return out;
}

// ------------------------------------------------------------------------------ d3-interpolate

const constant = (x) => () => x;

function interpolateNumber(a, b) {
  [a, b] = [+a, +b];
  return (t) => a * (1 - t) + b * t;
}

function interpolateRound(a, b) {
  [a, b] = [+a, +b];
  return (t) => Math.round(a * (1 - t) + b * t);
}

function rgbOf(c) {
  const parsed = parseColor(String(c));
  return parsed && { r: parsed[0], g: parsed[1], b: parsed[2], opacity: parsed[3] };
}

function channel(a, b) {
  const d = b - a;
  return d ? (t) => a + t * d : constant(isNaN(a) ? b : a);
}

// As d3-color writes an rgb color.
function formatRgb(c) {
  const whole = (v) => Math.max(0, Math.min(255, Math.round(v) || 0));
  const a = traced(c.opacity) || !isNaN(c.opacity) ? Math.max(0, Math.min(1, c.opacity)) : 1;
  const r = `${whole(c.r)}, ${whole(c.g)}, ${whole(c.b)}`;
  return a === 1 ? `rgb(${r})` : `rgba(${r}, ${a})`;
}

function interpolateRgb(a, b) {
  [a, b] = [rgbOf(a) ?? {}, rgbOf(b) ?? {}];
  const [r, g, bl, opacity] = ["r", "g", "b", "opacity"].map((k) => channel(a[k], b[k]));
  return (t) => formatRgb({ r: r(t), g: g(t), b: bl(t), opacity: opacity(t) });
}

const NUMBER = /[-+]?(?:\d+\.?\d*|\.?\d+)(?:[eE][-+]?\d+)?/g;

// The numbers in `b` between those in `a`, the rest of `b` as is.
function interpolateString(a, b) {
  [a, b] = [String(a), String(b)];
  const as = [...a.matchAll(NUMBER)];
  const bs = [...b.matchAll(NUMBER)];
  const parts = [];
  let at = 0;
  for (let i = 0; i < Math.min(as.length, bs.length); i++) {
    const [am, bm] = [as[i][0], bs[i][0]];
    parts.push(b.slice(at, bs[i].index));
    parts.push(am === bm ? bm : interpolateNumber(am, bm));
    at = bs[i].index + bm.length;
  }
  parts.push(b.slice(at));
  const fixedParts = parts.every((p) => typeof p === "string");
  if (fixedParts) return constant(b);
  return (t) => {
    let out = "";
    for (const p of parts) out = out + (typeof p === "string" ? p : p(t));
    return out;
  };
}

function interpolateArray(a, b) {
  const nb = b ? b.length : 0;
  const na = a ? Math.min(nb, a.length) : 0;
  const x = [];
  for (let i = 0; i < na; i++) x.push(interpolate(a[i], b[i]));
  return (t) => b.map((v, i) => (i < na ? x[i](t) : v));
}

function interpolateObject(a, b) {
  if (a === null || typeof a !== "object") a = {};
  if (b === null || typeof b !== "object") b = {};
  const i = {};
  const c = {};
  for (const k in b) {
    if (k in a) i[k] = interpolate(a[k], b[k]);
    else c[k] = b[k];
  }
  return (t) => {
    for (const k in i) c[k] = i[k](t);
    return c;
  };
}

// d3's `interpolate(a, b)`, by what `b` is: numbers, colors, numbers in text, arrays, objects.
function interpolate(a, b) {
  const t = typeof b;
  if (missing(b) || t === "boolean") return constant(b);
  if (t === "number") return interpolateNumber(a, b);
  if (t === "string") return rgbOf(b) ? interpolateRgb(a, b) : interpolateString(a, b);
  if (Array.isArray(b)) return interpolateArray(a, b);
  return interpolateObject(a, b);
}

// ------------------------------------------------------------------------------ d3-scale

function identity(x) {
  return x;
}

function normalize(a, b) {
  a = +a;
  b = b - a;
  return b ? (x) => (x - a) / b : constant(isNaN(b) ? NaN : 0.5);
}

function clamper(a, b) {
  if (a > b) [a, b] = [b, a];
  return (x) => Math.max(a, Math.min(b, x));
}

function bimap(domain, range, interpolate) {
  let [d0, d1, r0, r1] = [domain[0], domain[1], range[0], range[1]];
  if (d1 < d0) [d0, r0] = [normalize(d1, d0), interpolate(r1, r0)];
  else [d0, r0] = [normalize(d0, d1), interpolate(r0, r1)];
  return (x) => r0(d0(x));
}

function polymap(domain, range, interpolate) {
  const j = Math.min(domain.length, range.length) - 1;
  if (domain[j] < domain[0]) {
    domain = domain.slice().reverse();
    range = range.slice().reverse();
  }
  const d = [];
  const r = [];
  for (let i = 0; i < j; i++) {
    d.push(normalize(domain[i], domain[i + 1]));
    r.push(interpolate(range[i], range[i + 1]));
  }
  return (x) => {
    const i = bisectRight(domain, x, 1, j) - 1;
    return r[i](d[i](x));
  };
}

// The scale methods of an initialized scale: `scaleX()`, `scaleX(range)` or
// `scaleX(domain, range)`.
function initRange(scale, args) {
  if (args.length === 1) scale.range(args[0]);
  else if (args.length > 1) scale.range(args[1]).domain(args[0]);
  return scale;
}

function noTickFormat() {
  throw new TypeError("tickFormat is not built in: write tick labels with formatNumber()");
}

// Ticks and nice of a scale with a numeric domain.
function linearish(scale) {
  const domain = scale.domain;
  scale.ticks = (count) => {
    const d = domain();
    return ticks(d[0], d[d.length - 1], count == null ? 10 : count);
  };
  scale.tickFormat = noTickFormat;
  scale.nice = (count) => {
    if (count == null) count = 10;
    const d = domain();
    let [i0, i1] = [0, d.length - 1];
    let [start, stop] = [d[i0], d[i1]];
    let prestep;
    let step;
    let maxIter = 10;
    if (stop < start) {
      [start, stop] = [stop, start];
      [i0, i1] = [i1, i0];
    }
    while (maxIter-- > 0) {
      step = tickIncrement(start, stop, count);
      if (step === prestep) {
        d[i0] = start;
        d[i1] = stop;
        return domain(d);
      } else if (step > 0) {
        start = Math.floor(start / step) * step;
        stop = Math.ceil(stop / step) * step;
      } else if (step < 0) {
        start = Math.ceil(start * step) / step;
        stop = Math.floor(stop * step) / step;
      } else {
        break;
      }
      prestep = step;
    }
    return scale;
  };
  return scale;
}

export function scaleLinear(...args) {
  let domain = [0, 1];
  let range = [0, 1];
  let interpolator = interpolate;
  let unknown;
  let clamp = identity;
  let piecewise;
  let output;
  let input;

  function rescale() {
    const n = Math.min(domain.length, range.length);
    if (clamp !== identity) clamp = clamper(domain[0], domain[n - 1]);
    piecewise = n > 2 ? polymap : bimap;
    output = input = null;
    return scale;
  }

  function scale(x) {
    if (!traced(x) && (x == null || isNaN((x = +x)))) return unknown;
    output ??= piecewise(domain, range, interpolator);
    return output(clamp(x));
  }

  scale.invert = (y) => {
    input ??= piecewise(range, domain, interpolateNumber);
    return clamp(input(y));
  };
  scale.domain = (...a) =>
    a.length ? ((domain = Array.from(a[0], (v) => +v)), rescale()) : domain.slice();
  scale.range = (...a) => (a.length ? ((range = Array.from(a[0])), rescale()) : range.slice());
  scale.rangeRound = (r) => {
    range = Array.from(r);
    interpolator = interpolateRound;
    return rescale();
  };
  scale.clamp = (...a) =>
    a.length ? ((clamp = a[0] ? true : identity), rescale()) : clamp !== identity;
  scale.interpolate = (...a) => (a.length ? ((interpolator = a[0]), rescale()) : interpolator);
  scale.unknown = (...a) => (a.length ? ((unknown = a[0]), scale) : unknown);
  scale.copy = () =>
    scaleLinear()
      .domain(domain)
      .range(range)
      .interpolate(interpolator)
      .clamp(clamp !== identity)
      .unknown(unknown);
  rescale();
  return initRange(linearish(scale), args);
}

export function scaleSequential(...args) {
  let [x0, x1, t0, t1, k10] = [0, 1, 0, 1, 1];
  let interpolator = identity;
  let clamp = false;
  let unknown;

  function scale(x) {
    if (!traced(x) && (x == null || isNaN((x = +x)))) return unknown;
    if (k10 === 0) return interpolator(0.5);
    x = (x - t0) * k10;
    return interpolator(clamp ? Math.max(0, Math.min(1, x)) : x);
  }

  scale.domain = (...a) => {
    if (!a.length) return [x0, x1];
    [x0, x1] = a[0];
    [t0, t1] = [(x0 = +x0), (x1 = +x1)];
    k10 = t0 === t1 ? 0 : 1 / (t1 - t0);
    return scale;
  };
  scale.clamp = (...a) => (a.length ? ((clamp = !!a[0]), scale) : clamp);
  scale.interpolator = (...a) => (a.length ? ((interpolator = a[0]), scale) : interpolator);
  const ranged =
    (make) =>
    (...a) => {
      if (!a.length) return [interpolator(0), interpolator(1)];
      interpolator = make(a[0][0], a[0][1]);
      return scale;
    };
  scale.range = ranged(interpolate);
  scale.rangeRound = ranged(interpolateRound);
  scale.unknown = (...a) => (a.length ? ((unknown = a[0]), scale) : unknown);
  scale.copy = () =>
    scaleSequential().domain([x0, x1]).interpolator(interpolator).clamp(clamp).unknown(unknown);
  linearish(scale);
  if (args.length === 1) {
    if (typeof args[0] === "function") scale.interpolator(args[0]);
    else scale.range(args[0]);
  } else if (args.length > 1) {
    scale.domain(args[0]);
    if (typeof args[1] === "function") scale.interpolator(args[1]);
    else scale.range(args[1]);
  }
  return scale;
}

export function scaleQuantize(...args) {
  let [x0, x1, n] = [0, 1, 1];
  let domain = [0.5];
  let range = [0, 1];
  let unknown;

  function scale(x) {
    // oxlint-disable-next-line const-comparisons -- false for NaN
    if (!traced(x) && !(x != null && x <= x)) return unknown;
    return range[bisectRight(domain, x, 0, n)];
  }

  function rescale() {
    domain = [];
    for (let i = 0; i < n; i++) domain.push(((i + 1) * x1 - (i - n) * x0) / (n + 1));
    return scale;
  }

  scale.domain = (...a) => {
    if (!a.length) return [x0, x1];
    [x0, x1] = a[0];
    [x0, x1] = [+x0, +x1];
    return rescale();
  };
  scale.range = (...a) => {
    if (!a.length) return range.slice();
    range = Array.from(a[0]);
    n = range.length - 1;
    return rescale();
  };
  scale.invertExtent = (y) => {
    const i = range.indexOf(y);
    if (i < 0) return [NaN, NaN];
    if (i < 1) return [x0, domain[0]];
    if (i >= n) return [domain[n - 1], x1];
    return [domain[i - 1], domain[i]];
  };
  scale.unknown = (...a) => (a.length ? ((unknown = a[0]), scale) : scale);
  scale.thresholds = () => domain.slice();
  scale.copy = () => scaleQuantize().domain([x0, x1]).range(range).unknown(unknown);
  return initRange(linearish(scale), args);
}

export function scaleBand(...args) {
  let keys = [];
  let index = new Map();
  let values = [];
  let [r0, r1] = [0, 1];
  let step;
  let bandwidth;
  let round = false;
  let paddingInner = 0;
  let paddingOuter = 0;
  let align = 0.5;

  // A domain value as a key, as d3's InternMap takes one.
  const key = (d) => (d !== null && typeof d === "object" ? d.valueOf() : d);

  function scale(d) {
    const i = index.get(key(fixed(d)));
    return i === undefined ? undefined : values[i % values.length];
  }

  function rescale() {
    const n = keys.length;
    const reverse = r1 < r0;
    let start = reverse ? r1 : r0;
    const stop = reverse ? r0 : r1;
    step = (stop - start) / Math.max(1, n - paddingInner + paddingOuter * 2);
    if (round) step = Math.floor(step);
    start += (stop - start - step * (n - paddingInner)) * align;
    bandwidth = step * (1 - paddingInner);
    if (round) [start, bandwidth] = [Math.round(start), Math.round(bandwidth)];
    values = sequence(n).map((i) => start + step * i);
    if (reverse) values.reverse();
    return scale;
  }

  scale.domain = (...a) => {
    if (!a.length) return keys.slice();
    [keys, index] = [[], new Map()];
    for (const value of a[0]) {
      if (index.has(key(value))) continue;
      index.set(key(value), keys.push(value) - 1);
    }
    return rescale();
  };
  scale.range = (...a) => {
    if (!a.length) return [r0, r1];
    [r0, r1] = a[0];
    [r0, r1] = [+r0, +r1];
    return rescale();
  };
  scale.rangeRound = (r) => {
    [r0, r1] = r;
    [r0, r1, round] = [+r0, +r1, true];
    return rescale();
  };
  scale.bandwidth = () => bandwidth;
  scale.step = () => step;
  scale.round = (...a) => (a.length ? ((round = !!a[0]), rescale()) : round);
  scale.padding = (...a) =>
    a.length ? ((paddingInner = Math.min(1, (paddingOuter = +a[0]))), rescale()) : paddingInner;
  scale.paddingInner = (...a) =>
    a.length ? ((paddingInner = Math.min(1, a[0])), rescale()) : paddingInner;
  scale.paddingOuter = (...a) => (a.length ? ((paddingOuter = +a[0]), rescale()) : paddingOuter);
  scale.align = (...a) =>
    a.length ? ((align = Math.max(0, Math.min(1, a[0]))), rescale()) : align;
  scale.copy = () =>
    scaleBand(keys, [r0, r1])
      .round(round)
      .paddingInner(paddingInner)
      .paddingOuter(paddingOuter)
      .align(align);
  rescale();
  if (args.length === 1) scale.range(args[0]);
  else if (args.length > 1) scale.range(args[1]).domain(args[0]);
  return scale;
}

// A band scale whose bands have no width; its padding is the outer padding.
function pointish(scale) {
  const copy = scale.copy;
  scale.padding = scale.paddingOuter;
  scale.paddingInner = scale.paddingOuter = undefined;
  scale.copy = () => pointish(copy());
  return scale;
}

export function scalePoint(...args) {
  return pointish(scaleBand(...args).paddingInner(1));
}

// ------------------------------------------------------------------------------ d3-path

const pi = Math.PI;
const tau = 2 * pi;
const pathEpsilon = 1e-6;
const tauEpsilon = tau - pathEpsilon;

// Path data as d3-path writes it, its numbers rounded to `digits` decimals unless null. Its
// parts are joined once, so a path of traced numbers is one template.
class Path {
  constructor(digits) {
    this._x0 = this._y0 = this._x1 = this._y1 = null;
    this._parts = [];
    let k = null;
    if (digits != null) {
      const d = Math.floor(digits);
      if (!(d >= 0)) throw new RangeError(`invalid digits: ${digits}`);
      if (d <= 15) k = 10 ** d;
    }
    this._k = k;
  }
  // Text, and numbers, rounded.
  _append(...items) {
    for (const [i, item] of items.entries()) {
      const number = i % 2 === 1;
      this._parts.push(number && this._k !== null ? Math.round(item * this._k) / this._k : item);
    }
  }
  _equal(x0, y0, x1, y1) {
    if (this._k === null) return x0 === x1 && y0 === y1;
    const r = (v) => Math.round(v * this._k);
    return r(x0) === r(x1) && r(y0) === r(y1);
  }
  moveTo(x, y) {
    this._append("M", (this._x0 = this._x1 = +x), ",", (this._y0 = this._y1 = +y));
  }
  closePath() {
    if (this._x1 !== null) {
      [this._x1, this._y1] = [this._x0, this._y0];
      this._append("Z");
    }
  }
  lineTo(x, y) {
    this._append("L", (this._x1 = +x), ",", (this._y1 = +y));
  }
  quadraticCurveTo(x1, y1, x, y) {
    this._append("Q", +x1, ",", +y1, ",", (this._x1 = +x), ",", (this._y1 = +y));
  }
  bezierCurveTo(x1, y1, x2, y2, x, y) {
    this._append("C", +x1, ",", +y1, ",", +x2, ",", +y2, ",", (this._x1 = +x), ",");
    this._append("", (this._y1 = +y));
  }
  arcTo(x1, y1, x2, y2, r) {
    [x1, y1, x2, y2, r] = [+x1, +y1, +x2, +y2, +r];
    if (r < 0) throw new Error(`negative radius: ${r}`);
    const [x0, y0] = [this._x1, this._y1];
    const [x21, y21, x01, y01] = [x2 - x1, y2 - y1, x0 - x1, y0 - y1];
    const l01_2 = x01 * x01 + y01 * y01;
    if (this._x1 === null) {
      this._append("M", (this._x1 = x1), ",", (this._y1 = y1));
    } else if (!(l01_2 > pathEpsilon)) {
      // Coincident with the last point: nothing to draw.
    } else if (!(Math.abs(y01 * x21 - y21 * x01) > pathEpsilon) || !r) {
      this._append("L", (this._x1 = x1), ",", (this._y1 = y1));
    } else {
      const [x20, y20] = [x2 - x0, y2 - y0];
      const l21_2 = x21 * x21 + y21 * y21;
      const l20_2 = x20 * x20 + y20 * y20;
      const [l21, l01] = [Math.sqrt(l21_2), Math.sqrt(l01_2)];
      const l = r * Math.tan((pi - Math.acos((l21_2 + l01_2 - l20_2) / (2 * l21 * l01))) / 2);
      const [t01, t21] = [l / l01, l / l21];
      if (Math.abs(t01 - 1) > pathEpsilon) {
        this._append("L", x1 + t01 * x01, ",", y1 + t01 * y01);
      }
      this._append("A", r, ",", r, ",0,0,", y01 * x20 > x01 * y20 ? 1 : 0, ",");
      this._append("", (this._x1 = x1 + t21 * x21), ",", (this._y1 = y1 + t21 * y21));
    }
  }
  // Its flags are selects, so traced angles stay ops; what decides the commands is guarded.
  arc(x, y, r, a0, a1, ccw) {
    [x, y, r] = [+x, +y, +r];
    if (r < 0) throw new Error(`negative radius: ${r}`);
    const [dx, dy] = [r * Math.cos(a0), r * Math.sin(a0)];
    const [x0, y0] = [x + dx, y + dy];
    const cw = ccw ? 0 : 1;
    let da = ccw ? a0 - a1 : a1 - a0;
    if (this._x1 === null) {
      this._append("M", x0, ",", y0);
    } else if (Math.abs(this._x1 - x0) > pathEpsilon || Math.abs(this._y1 - y0) > pathEpsilon) {
      this._append("L", x0, ",", y0);
    }
    if (!r) return;
    da = da < 0 ? (da % tau) + tau : da;
    if (da > tauEpsilon) {
      this._append("A", r, ",", r, ",0,1,", cw, ",", x - dx, ",", y - dy);
      this._append("A", r, ",", r, ",0,1,", cw, ",", (this._x1 = x0), ",");
      this._append("", (this._y1 = y0));
    } else if (da > pathEpsilon) {
      const [x1, y1] = [x + r * Math.cos(a1), y + r * Math.sin(a1)];
      if (da >= pi && this._equal(x0, y0, x1, y1)) {
        da /= 2;
        const a00 = a0 + da;
        this._append("A", r, ",", r, ",0,", da >= pi ? 1 : 0, ",", cw, ",");
        this._append("", x + r * Math.cos(a00), ",", y + r * Math.sin(a00));
      }
      this._append("A", r, ",", r, ",0,", da >= pi ? 1 : 0, ",", cw, ",", (this._x1 = x1), ",");
      this._append("", (this._y1 = y1));
    }
  }
  rect(x, y, w, h) {
    this._append("M", (this._x0 = this._x1 = +x), ",", (this._y0 = this._y1 = +y), "h");
    this._append("", (w = +w), "v", +h, "h", -w, "Z");
  }
  toString() {
    return concat(this._parts);
  }
}

// ------------------------------------------------------------------------------ d3-shape

const shapeConstant = (x) => () => x;

// A shape's path data, with its `digits()`, 3 unless set.
function withPath(shape) {
  let digits = 3;
  shape.digits = (...a) => {
    if (!a.length) return digits;
    if (a[0] == null) {
      digits = null;
    } else {
      const d = Math.floor(a[0]);
      if (!(d >= 0)) throw new RangeError(`invalid digits: ${a[0]}`);
      digits = d;
    }
    return shape;
  };
  return () => new Path(digits);
}

// A shape's path data, null when empty; traced data is never empty.
function text(buffer) {
  const t = buffer.toString();
  return traced(t) ? t : t || null;
}

const asArray = (x) => (typeof x === "object" && "length" in x ? x : Array.from(x));
const pointX = (p) => p[0];
const pointY = (p) => p[1];
const accessor = (f, otherwise) =>
  typeof f === "function" ? f : f === undefined ? otherwise : shapeConstant(f);

class Linear {
  constructor(context) {
    this._context = context;
  }
  areaStart() {
    this._line = 0;
  }
  areaEnd() {
    this._line = NaN;
  }
  lineStart() {
    this._point = 0;
  }
  lineEnd() {
    if (this._line || (this._line !== 0 && this._point === 1)) this._context.closePath();
    this._line = 1 - this._line;
  }
  point(x, y) {
    [x, y] = [+x, +y];
    if (this._point === 0) {
      this._point = 1;
      if (this._line) this._context.lineTo(x, y);
      else this._context.moveTo(x, y);
    } else {
      this._point = 2;
      this._context.lineTo(x, y);
    }
  }
}

export function curveLinear(context) {
  return new Linear(context);
}

const sign = (x) => (x < 0 ? -1 : 1);

// The slope of the tangent at the middle of three points (Steffen 1990).
function slope3(that, x2, y2) {
  const h0 = that._x1 - that._x0;
  const h1 = x2 - that._x1;
  const s0 = (that._y1 - that._y0) / (h0 || (h1 < 0 && -0));
  const s1 = (y2 - that._y1) / (h1 || (h0 < 0 && -0));
  const p = (s0 * h1 + s1 * h0) / (h0 + h1);
  return (sign(s0) + sign(s1)) * Math.min(Math.abs(s0), Math.abs(s1), 0.5 * Math.abs(p)) || 0;
}

// A one-sided slope.
function slope2(that, t) {
  const h = that._x1 - that._x0;
  return h ? ((3 * (that._y1 - that._y0)) / h - t) / 2 : t;
}

// The cubic Hermite segment to the last point, as a Bézier curve.
function hermite(that, t0, t1) {
  const [x0, y0, x1, y1] = [that._x0, that._y0, that._x1, that._y1];
  const dx = (x1 - x0) / 3;
  that._context.bezierCurveTo(x0 + dx, y0 + dx * t0, x1 - dx, y1 - dx * t1, x1, y1);
}

class MonotoneX {
  constructor(context) {
    this._context = context;
  }
  areaStart() {
    this._line = 0;
  }
  areaEnd() {
    this._line = NaN;
  }
  lineStart() {
    this._x0 = this._x1 = this._y0 = this._y1 = this._t0 = NaN;
    this._point = 0;
  }
  lineEnd() {
    if (this._point === 2) this._context.lineTo(this._x1, this._y1);
    else if (this._point === 3) hermite(this, this._t0, slope2(this, this._t0));
    if (this._line || (this._line !== 0 && this._point === 1)) this._context.closePath();
    this._line = 1 - this._line;
  }
  point(x, y) {
    let t1 = NaN;
    [x, y] = [+x, +y];
    // Coincident points are left out.
    if (x === this._x1 && y === this._y1) return;
    if (this._point === 0) {
      this._point = 1;
      if (this._line) this._context.lineTo(x, y);
      else this._context.moveTo(x, y);
    } else if (this._point === 1) {
      this._point = 2;
    } else if (this._point === 2) {
      this._point = 3;
      t1 = slope3(this, x, y);
      hermite(this, slope2(this, t1), t1);
    } else {
      t1 = slope3(this, x, y);
      hermite(this, this._t0, t1);
    }
    [this._x0, this._x1, this._y0, this._y1, this._t0] = [this._x1, x, this._y1, y, t1];
  }
}

export function curveMonotoneX(context) {
  return new MonotoneX(context);
}

export function line(x, y) {
  let defined = shapeConstant(true);
  let context = null;
  let curve = curveLinear;
  let output = null;
  const path = withPath(shape);
  x = accessor(x, pointX);
  y = accessor(y, pointY);

  function shape(data) {
    data = asArray(data);
    const n = data.length;
    let d;
    let defined0 = false;
    let buffer;
    if (context == null) output = curve((buffer = path()));
    for (let i = 0; i <= n; ++i) {
      if (!(i < n && defined((d = data[i]), i, data)) === defined0) {
        if ((defined0 = !defined0)) output.lineStart();
        else output.lineEnd();
      }
      if (defined0) output.point(+x(d, i, data), +y(d, i, data));
    }
    if (buffer) {
      output = null;
      return text(buffer);
    }
  }

  shape.x = (...a) =>
    a.length ? ((x = typeof a[0] === "function" ? a[0] : shapeConstant(+a[0])), shape) : x;
  shape.y = (...a) =>
    a.length ? ((y = typeof a[0] === "function" ? a[0] : shapeConstant(+a[0])), shape) : y;
  shape.defined = (...a) =>
    a.length
      ? ((defined = typeof a[0] === "function" ? a[0] : shapeConstant(!!a[0])), shape)
      : defined;
  shape.curve = (...a) => {
    if (!a.length) return curve;
    curve = a[0];
    if (context != null) output = curve(context);
    return shape;
  };
  shape.context = (...a) => {
    if (!a.length) return context;
    if (a[0] == null) context = output = null;
    else output = curve((context = a[0]));
    return shape;
  };
  return shape;
}

export function area(x0, y0, y1) {
  let x1 = null;
  let defined = shapeConstant(true);
  let context = null;
  let curve = curveLinear;
  let output = null;
  const path = withPath(shape);
  const number = (v) => (typeof v === "function" ? v : shapeConstant(+v));
  x0 = typeof x0 === "function" ? x0 : x0 === undefined ? pointX : shapeConstant(+x0);
  y0 = typeof y0 === "function" ? y0 : y0 === undefined ? shapeConstant(0) : shapeConstant(+y0);
  y1 = typeof y1 === "function" ? y1 : y1 === undefined ? pointY : shapeConstant(+y1);

  function shape(data) {
    data = asArray(data);
    const n = data.length;
    let j;
    let d;
    let defined0 = false;
    let buffer;
    const x0z = [];
    const y0z = [];
    if (context == null) output = curve((buffer = path()));
    for (let i = 0; i <= n; ++i) {
      if (!(i < n && defined((d = data[i]), i, data)) === defined0) {
        if ((defined0 = !defined0)) {
          j = i;
          output.areaStart();
          output.lineStart();
        } else {
          output.lineEnd();
          output.lineStart();
          for (let k = i - 1; k >= j; --k) output.point(x0z[k], y0z[k]);
          output.lineEnd();
          output.areaEnd();
        }
      }
      if (defined0) {
        [x0z[i], y0z[i]] = [+x0(d, i, data), +y0(d, i, data)];
        output.point(x1 ? +x1(d, i, data) : x0z[i], y1 ? +y1(d, i, data) : y0z[i]);
      }
    }
    if (buffer) {
      output = null;
      return text(buffer);
    }
  }

  const arealine = () => line().defined(defined).curve(curve).context(context);
  shape.x = (...a) => (a.length ? ((x0 = number(a[0])), (x1 = null), shape) : x0);
  shape.x0 = (...a) => (a.length ? ((x0 = number(a[0])), shape) : x0);
  shape.x1 = (...a) => (a.length ? ((x1 = a[0] == null ? null : number(a[0])), shape) : x1);
  shape.y = (...a) => (a.length ? ((y0 = number(a[0])), (y1 = null), shape) : y0);
  shape.y0 = (...a) => (a.length ? ((y0 = number(a[0])), shape) : y0);
  shape.y1 = (...a) => (a.length ? ((y1 = a[0] == null ? null : number(a[0])), shape) : y1);
  shape.lineX0 = shape.lineY0 = () => arealine().x(x0).y(y0);
  shape.lineY1 = () => arealine().x(x0).y(y1);
  shape.lineX1 = () => arealine().x(x1).y(y0);
  shape.defined = (...a) =>
    a.length
      ? ((defined = typeof a[0] === "function" ? a[0] : shapeConstant(!!a[0])), shape)
      : defined;
  shape.curve = (...a) => {
    if (!a.length) return curve;
    curve = a[0];
    if (context != null) output = curve(context);
    return shape;
  };
  shape.context = (...a) => {
    if (!a.length) return context;
    if (a[0] == null) context = output = null;
    else output = curve((context = a[0]));
    return shape;
  };
  return shape;
}

const halfPi = pi / 2;
const epsilon = 1e-12;
const clampedAcos = (x) => (x > 1 ? 0 : x < -1 ? pi : Math.acos(x));
const clampedAsin = (x) => (x >= 1 ? halfPi : x <= -1 ? -halfPi : Math.asin(x));

// Where lines 0–1 and 2–3 cross, if they do.
function intersect(x0, y0, x1, y1, x2, y2, x3, y3) {
  const [x10, y10, x32, y32] = [x1 - x0, y1 - y0, x3 - x2, y3 - y2];
  let t = y32 * x10 - x32 * y10;
  if (t * t < epsilon) return;
  t = (x32 * (y0 - y2) - y32 * (x0 - x2)) / t;
  return [x0 + t * x10, y0 + t * y10];
}

// The corner of radius `rc` tangent to the line 0–1 and the circle of radius `r1`.
function cornerTangents(x0, y0, x1, y1, r1, rc, cw) {
  const [x01, y01] = [x0 - x1, y0 - y1];
  const lo = (cw ? rc : -rc) / Math.sqrt(x01 * x01 + y01 * y01);
  const [ox, oy] = [lo * y01, -lo * x01];
  const [x11, y11, x10, y10] = [x0 + ox, y0 + oy, x1 + ox, y1 + oy];
  const [x00, y00] = [(x11 + x10) / 2, (y11 + y10) / 2];
  const [dx, dy] = [x10 - x11, y10 - y11];
  const d2 = dx * dx + dy * dy;
  const r = r1 - rc;
  const D = x11 * y10 - x10 * y11;
  const d = (dy < 0 ? -1 : 1) * Math.sqrt(Math.max(0, r * r * d2 - D * D));
  const [cx0, cy0] = [(D * dy - dx * d) / d2, (-D * dx - dy * d) / d2];
  const [cx1, cy1] = [(D * dy + dx * d) / d2, (-D * dx + dy * d) / d2];
  const [dx0, dy0, dx1, dy1] = [cx0 - x00, cy0 - y00, cx1 - x00, cy1 - y00];
  // The closer of the two. A select, not an if: which one it is changes nothing about the path
  // the arc draws, and a traced comparison would trace the scene again each time it changed.
  const near = dx0 * dx0 + dy0 * dy0 > dx1 * dx1 + dy1 * dy1;
  const [cx, cy] = near ? [cx1, cy1] : [cx0, cy0];
  return {
    cx,
    cy,
    x01: -ox,
    y01: -oy,
    x11: cx * (r1 / r - 1),
    y11: cy * (r1 / r - 1),
  };
}

const numberOr = (v) => (typeof v === "function" ? v : shapeConstant(+v));

export function arc() {
  let innerRadius = (d) => d.innerRadius;
  let outerRadius = (d) => d.outerRadius;
  let cornerRadius = shapeConstant(0);
  let padRadius = null;
  let startAngle = (d) => d.startAngle;
  let endAngle = (d) => d.endAngle;
  let padAngle = (d) => d && d.padAngle;
  let context = null;
  const path = withPath(shape);

  function shape(...args) {
    let buffer;
    let r0 = +innerRadius(...args);
    let r1 = +outerRadius(...args);
    const a0 = startAngle(...args) - halfPi;
    const a1 = endAngle(...args) - halfPi;
    const da = Math.abs(a1 - a0);
    const cw = a1 > a0;
    if (!context) context = buffer = path();
    if (r1 < r0) [r0, r1] = [r1, r0];
    if (!(r1 > epsilon)) {
      // A point.
      context.moveTo(0, 0);
    } else if (da > tau - epsilon) {
      // A circle or an annulus.
      context.moveTo(r1 * Math.cos(a0), r1 * Math.sin(a0));
      context.arc(0, 0, r1, a0, a1, !cw);
      if (r0 > epsilon) {
        context.moveTo(r0 * Math.cos(a1), r0 * Math.sin(a1));
        context.arc(0, 0, r0, a1, a0, cw);
      }
    } else {
      // A circular or an annular sector.
      let [a01, a11, a00, a10, da0, da1] = [a0, a1, a0, a1, da, da];
      const ap = padAngle(...args) / 2;
      const rp = ap > epsilon && (padRadius ? +padRadius(...args) : Math.sqrt(r0 * r0 + r1 * r1));
      const rc = Math.min(Math.abs(r1 - r0) / 2, +cornerRadius(...args));
      let [rc0, rc1] = [rc, rc];
      if (rp > epsilon) {
        let p0 = clampedAsin((rp / r0) * Math.sin(ap));
        let p1 = clampedAsin((rp / r1) * Math.sin(ap));
        if ((da0 -= p0 * 2) > epsilon) {
          p0 *= cw ? 1 : -1;
          [a00, a10] = [a00 + p0, a10 - p0];
        } else {
          da0 = 0;
          a00 = a10 = (a0 + a1) / 2;
        }
        if ((da1 -= p1 * 2) > epsilon) {
          p1 *= cw ? 1 : -1;
          [a01, a11] = [a01 + p1, a11 - p1];
        } else {
          da1 = 0;
          a01 = a11 = (a0 + a1) / 2;
        }
      }
      const [x01, y01] = [r1 * Math.cos(a01), r1 * Math.sin(a01)];
      const [x10, y10] = [r0 * Math.cos(a10), r0 * Math.sin(a10)];
      let x11, y11, x00, y00;
      if (rc > epsilon) {
        [x11, y11] = [r1 * Math.cos(a11), r1 * Math.sin(a11)];
        [x00, y00] = [r0 * Math.cos(a00), r0 * Math.sin(a00)];
        // The corner radius as the sector's angle allows; none if the sector is too small.
        if (da < pi) {
          const oc = intersect(x01, y01, x00, y00, x11, y11, x10, y10);
          if (oc) {
            const [ax, ay, bx, by] = [x01 - oc[0], y01 - oc[1], x11 - oc[0], y11 - oc[1]];
            const cos =
              (ax * bx + ay * by) / (Math.sqrt(ax * ax + ay * ay) * Math.sqrt(bx * bx + by * by));
            const kc = 1 / Math.sin(clampedAcos(cos) / 2);
            const lc = Math.sqrt(oc[0] * oc[0] + oc[1] * oc[1]);
            rc0 = Math.min(rc, (r0 - lc) / (kc - 1));
            rc1 = Math.min(rc, (r1 - lc) / (kc + 1));
          } else {
            rc0 = rc1 = 0;
          }
        }
      }
      if (!(da1 > epsilon)) {
        // Collapsed to a line.
        context.moveTo(x01, y01);
      } else if (rc1 > epsilon) {
        const t0 = cornerTangents(x00, y00, x01, y01, r1, rc1, cw);
        const t1 = cornerTangents(x11, y11, x10, y10, r1, rc1, cw);
        context.moveTo(t0.cx + t0.x01, t0.cy + t0.y01);
        if (rc1 < rc) {
          // The corners merged.
          const [s, e] = [Math.atan2(t0.y01, t0.x01), Math.atan2(t1.y01, t1.x01)];
          context.arc(t0.cx, t0.cy, rc1, s, e, !cw);
        } else {
          let [s, e] = [Math.atan2(t0.y01, t0.x01), Math.atan2(t0.y11, t0.x11)];
          context.arc(t0.cx, t0.cy, rc1, s, e, !cw);
          s = Math.atan2(t0.cy + t0.y11, t0.cx + t0.x11);
          e = Math.atan2(t1.cy + t1.y11, t1.cx + t1.x11);
          context.arc(0, 0, r1, s, e, !cw);
          [s, e] = [Math.atan2(t1.y11, t1.x11), Math.atan2(t1.y01, t1.x01)];
          context.arc(t1.cx, t1.cy, rc1, s, e, !cw);
        }
      } else {
        context.moveTo(x01, y01);
        context.arc(0, 0, r1, a01, a11, !cw);
      }
      if (!(r0 > epsilon) || !(da0 > epsilon)) {
        // No inner ring, or one collapsed by padding.
        context.lineTo(x10, y10);
      } else if (rc0 > epsilon) {
        const t0 = cornerTangents(x10, y10, x11, y11, r0, -rc0, cw);
        const t1 = cornerTangents(x01, y01, x00, y00, r0, -rc0, cw);
        context.lineTo(t0.cx + t0.x01, t0.cy + t0.y01);
        if (rc0 < rc) {
          const [s, e] = [Math.atan2(t0.y01, t0.x01), Math.atan2(t1.y01, t1.x01)];
          context.arc(t0.cx, t0.cy, rc0, s, e, !cw);
        } else {
          let [s, e] = [Math.atan2(t0.y01, t0.x01), Math.atan2(t0.y11, t0.x11)];
          context.arc(t0.cx, t0.cy, rc0, s, e, !cw);
          s = Math.atan2(t0.cy + t0.y11, t0.cx + t0.x11);
          e = Math.atan2(t1.cy + t1.y11, t1.cx + t1.x11);
          context.arc(0, 0, r0, s, e, cw);
          [s, e] = [Math.atan2(t1.y11, t1.x11), Math.atan2(t1.y01, t1.x01)];
          context.arc(t1.cx, t1.cy, rc0, s, e, !cw);
        }
      } else {
        context.arc(0, 0, r0, a10, a00, cw);
      }
    }
    context.closePath();
    if (buffer) {
      context = null;
      return text(buffer);
    }
  }

  shape.centroid = (...args) => {
    const r = (+innerRadius(...args) + +outerRadius(...args)) / 2;
    const a = (+startAngle(...args) + +endAngle(...args)) / 2 - pi / 2;
    return [Math.cos(a) * r, Math.sin(a) * r];
  };
  shape.innerRadius = (...a) => (a.length ? ((innerRadius = numberOr(a[0])), shape) : innerRadius);
  shape.outerRadius = (...a) => (a.length ? ((outerRadius = numberOr(a[0])), shape) : outerRadius);
  shape.cornerRadius = (...a) =>
    a.length ? ((cornerRadius = numberOr(a[0])), shape) : cornerRadius;
  shape.padRadius = (...a) =>
    a.length ? ((padRadius = a[0] == null ? null : numberOr(a[0])), shape) : padRadius;
  shape.startAngle = (...a) => (a.length ? ((startAngle = numberOr(a[0])), shape) : startAngle);
  shape.endAngle = (...a) => (a.length ? ((endAngle = numberOr(a[0])), shape) : endAngle);
  shape.padAngle = (...a) => (a.length ? ((padAngle = numberOr(a[0])), shape) : padAngle);
  shape.context = (...a) => (a.length ? ((context = a[0] == null ? null : a[0]), shape) : context);
  return shape;
}

const descending = (a, b) => (b < a ? -1 : b > a ? 1 : b >= a ? 0 : NaN);

export function pie() {
  let value = identity;
  let sortValues = descending;
  let sort = null;
  let startAngle = shapeConstant(0);
  let endAngle = shapeConstant(tau);
  let padAngle = shapeConstant(0);

  function shape(data, ...rest) {
    data = asArray(data);
    const n = data.length;
    const args = [data, ...rest];
    let sum = 0;
    const index = [];
    const values = [];
    let a0 = +startAngle(...args);
    const da = Math.min(tau, Math.max(-tau, endAngle(...args) - a0));
    const p = Math.min(Math.abs(da) / n, padAngle(...args));
    const pa = p * (da < 0 ? -1 : 1);
    for (let i = 0; i < n; ++i) {
      const v = +value(data[i], i, data);
      index.push(i);
      values.push(v);
      if (v > 0) sum += v;
    }
    if (sortValues != null) index.sort((i, j) => sortValues(values[i], values[j]));
    else if (sort != null) index.sort((i, j) => sort(data[i], data[j]));
    // The arcs, in the data's order.
    const arcs = [];
    const k = sum ? (da - n * pa) / sum : 0;
    for (let i = 0; i < n; ++i) {
      const j = index[i];
      const v = values[j];
      const a1 = a0 + (v > 0 ? v * k : 0) + pa;
      arcs[j] = { data: data[j], index: i, value: v, startAngle: a0, endAngle: a1, padAngle: p };
      a0 = a1;
    }
    return arcs;
  }

  shape.value = (...a) => (a.length ? ((value = numberOr(a[0])), shape) : value);
  shape.sortValues = (...a) =>
    a.length ? ((sortValues = a[0]), (sort = null), shape) : sortValues;
  shape.sort = (...a) => (a.length ? ((sort = a[0]), (sortValues = null), shape) : sort);
  shape.startAngle = (...a) => (a.length ? ((startAngle = numberOr(a[0])), shape) : startAngle);
  shape.endAngle = (...a) => (a.length ? ((endAngle = numberOr(a[0])), shape) : endAngle);
  shape.padAngle = (...a) => (a.length ? ((padAngle = numberOr(a[0])), shape) : padAngle);
  return shape;
}

// Stacks the series in `order` on each other, from 0; a missing value keeps the one below.
function stackOffsetNone(series, order) {
  const n = series.length;
  if (!(n > 1)) return;
  let s1 = series[order[0]];
  const m = s1.length;
  for (let i = 1; i < n; ++i) {
    const s0 = s1;
    s1 = series[order[i]];
    for (let j = 0; j < m; ++j) {
      const below = s0[j][1];
      s1[j][0] = traced(below) || !isNaN(below) ? below : s0[j][0];
      s1[j][1] += s1[j][0];
    }
  }
}

const stackOrderNone = (series) => series.map((_, i) => i);

export function stack() {
  let keys = shapeConstant([]);
  let order = stackOrderNone;
  let offset = stackOffsetNone;
  let value = (d, key) => d[key];

  function shape(data, ...rest) {
    const series = Array.from(keys(data, ...rest), (key) => {
      const s = [];
      s.key = key;
      return s;
    });
    const n = series.length;
    let j = -1;
    for (const d of data) {
      ++j;
      for (let i = 0; i < n; ++i) {
        const point = [0, +value(d, series[i].key, j, data)];
        point.data = d;
        series[i][j] = point;
      }
    }
    const oz = asArray(order(series));
    for (let i = 0; i < n; ++i) series[oz[i]].index = i;
    offset(series, oz);
    return series;
  }

  shape.keys = (...a) =>
    a.length
      ? ((keys = typeof a[0] === "function" ? a[0] : shapeConstant(Array.from(a[0]))), shape)
      : keys;
  shape.value = (...a) => (a.length ? ((value = numberOr(a[0])), shape) : value);
  shape.order = (...a) => {
    if (!a.length) return order;
    const o = a[0];
    order = o == null ? stackOrderNone : typeof o === "function" ? o : shapeConstant(Array.from(o));
    return shape;
  };
  shape.offset = (...a) =>
    a.length ? ((offset = a[0] == null ? stackOffsetNone : a[0]), shape) : offset;
  return shape;
}

// ------------------------------------------------------------------------------ d3-geo

// Its streams keep what they gather in themselves, made new for each call, where d3's are shared.
const geoEpsilon = 1e-6;
const geoEpsilon2 = 1e-12;
const quarterPi = pi / 4;
const degrees = 180 / pi;
const radians = pi / 180;
const noop = () => {};

// d3-array's Adder: a sum of doubles without their rounding errors.
class Adder {
  constructor() {
    this._partials = [];
    this._n = 0;
  }
  add(x) {
    const p = this._partials;
    let i = 0;
    for (let j = 0; j < this._n && j < 32; j++) {
      const y = p[j];
      const hi = x + y;
      const lo = Math.abs(x) < Math.abs(y) ? x - (hi - y) : y - (hi - x);
      if (lo) p[i++] = lo;
      x = hi;
    }
    p[i] = x;
    this._n = i + 1;
    return this;
  }
  valueOf() {
    const p = this._partials;
    let n = this._n;
    let x, y, lo;
    let hi = 0;
    if (n > 0) {
      hi = p[--n];
      while (n > 0) {
        x = hi;
        y = p[--n];
        hi = x + y;
        lo = y - (hi - x);
        if (lo) break;
      }
      if (n > 0 && ((lo < 0 && p[n - 1] < 0) || (lo > 0 && p[n - 1] > 0))) {
        y = lo * 2;
        x = hi + y;
        if (y === x - hi) hi = x;
      }
    }
    return hi;
  }
}

// GeoJSON into a stream: its points, lines and polygons as calls.
function geoStream(object, stream) {
  if (object && Object.hasOwn(streamObjectType, object.type)) {
    streamObjectType[object.type](object, stream);
  } else {
    streamGeometry(object, stream);
  }
}

function streamGeometry(geometry, stream) {
  if (geometry && Object.hasOwn(streamGeometryType, geometry.type)) {
    streamGeometryType[geometry.type](geometry, stream);
  }
}

const streamObjectType = {
  Feature: (object, stream) => streamGeometry(object.geometry, stream),
  FeatureCollection: (object, stream) => {
    for (const feature of object.features) streamGeometry(feature.geometry, stream);
  },
};

const streamGeometryType = {
  Sphere: (object, stream) => stream.sphere(),
  Point: (object, stream) => {
    const c = object.coordinates;
    stream.point(c[0], c[1], c[2]);
  },
  MultiPoint: (object, stream) => {
    for (const c of object.coordinates) stream.point(c[0], c[1], c[2]);
  },
  LineString: (object, stream) => streamLine(object.coordinates, stream, 0),
  MultiLineString: (object, stream) => {
    for (const c of object.coordinates) streamLine(c, stream, 0);
  },
  Polygon: (object, stream) => streamPolygon(object.coordinates, stream),
  MultiPolygon: (object, stream) => {
    for (const c of object.coordinates) streamPolygon(c, stream);
  },
  GeometryCollection: (object, stream) => {
    for (const g of object.geometries) streamGeometry(g, stream);
  },
};

function streamLine(coordinates, stream, closed) {
  const n = coordinates.length - closed;
  stream.lineStart();
  for (let i = 0; i < n; ++i) {
    const c = coordinates[i];
    stream.point(c[0], c[1], c[2]);
  }
  stream.lineEnd();
}

function streamPolygon(coordinates, stream) {
  stream.polygonStart();
  for (const ring of coordinates) streamLine(ring, stream, 1);
  stream.polygonEnd();
}

function cartesian(spherical) {
  const [lambda, phi] = spherical;
  const cosPhi = Math.cos(phi);
  return [cosPhi * Math.cos(lambda), cosPhi * Math.sin(lambda), Math.sin(phi)];
}

function cartesianCross(a, b) {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function cartesianNormalizeInPlace(d) {
  const l = Math.sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
  d[0] /= l;
  d[1] /= l;
  d[2] /= l;
}

function compose(a, b) {
  const composed = (x, y) => {
    const p = a(x, y);
    return b(p[0], p[1]);
  };
  if (a.invert && b.invert) {
    composed.invert = (x, y) => {
      const p = b.invert(x, y);
      return p && a.invert(p[0], p[1]);
    };
  }
  return composed;
}

function rotationIdentity(lambda, phi) {
  if (Math.abs(lambda) > pi) lambda -= Math.round(lambda / tau) * tau;
  return [lambda, phi];
}
rotationIdentity.invert = rotationIdentity;

function rotateRadians(deltaLambda, deltaPhi, deltaGamma) {
  deltaLambda %= tau;
  if (deltaLambda) {
    return deltaPhi || deltaGamma
      ? compose(rotationLambda(deltaLambda), rotationPhiGamma(deltaPhi, deltaGamma))
      : rotationLambda(deltaLambda);
  }
  return deltaPhi || deltaGamma ? rotationPhiGamma(deltaPhi, deltaGamma) : rotationIdentity;
}

function forwardRotationLambda(deltaLambda) {
  return (lambda, phi) => {
    lambda += deltaLambda;
    if (Math.abs(lambda) > pi) lambda -= Math.round(lambda / tau) * tau;
    return [lambda, phi];
  };
}

function rotationLambda(deltaLambda) {
  const rotation = forwardRotationLambda(deltaLambda);
  rotation.invert = forwardRotationLambda(-deltaLambda);
  return rotation;
}

function rotationPhiGamma(deltaPhi, deltaGamma) {
  const cosDeltaPhi = Math.cos(deltaPhi);
  const sinDeltaPhi = Math.sin(deltaPhi);
  const cosDeltaGamma = Math.cos(deltaGamma);
  const sinDeltaGamma = Math.sin(deltaGamma);
  const rotation = (lambda, phi) => {
    const cosPhi = Math.cos(phi);
    const x = Math.cos(lambda) * cosPhi;
    const y = Math.sin(lambda) * cosPhi;
    const z = Math.sin(phi);
    const k = z * cosDeltaPhi + x * sinDeltaPhi;
    return [
      Math.atan2(y * cosDeltaGamma - k * sinDeltaGamma, x * cosDeltaPhi - z * sinDeltaPhi),
      clampedAsin(k * cosDeltaGamma + y * sinDeltaGamma),
    ];
  };
  rotation.invert = (lambda, phi) => {
    const cosPhi = Math.cos(phi);
    const x = Math.cos(lambda) * cosPhi;
    const y = Math.sin(lambda) * cosPhi;
    const z = Math.sin(phi);
    const k = z * cosDeltaGamma - y * sinDeltaGamma;
    return [
      Math.atan2(y * cosDeltaGamma + z * sinDeltaGamma, x * cosDeltaPhi + k * sinDeltaPhi),
      clampedAsin(k * cosDeltaPhi - x * sinDeltaPhi),
    ];
  };
  return rotation;
}

// A rotation of [longitude, latitude] in degrees.
function geoRotation(rotate) {
  const r = rotateRadians(
    rotate[0] * radians,
    rotate[1] * radians,
    rotate.length > 2 ? rotate[2] * radians : 0,
  );
  const forward = (c) => {
    const p = r(c[0] * radians, c[1] * radians);
    return [p[0] * degrees, p[1] * degrees];
  };
  forward.invert = (c) => {
    const p = r.invert(c[0] * radians, c[1] * radians);
    return [p[0] * degrees, p[1] * degrees];
  };
  return forward;
}

// A stream that passes its calls to `stream`, but for those `methods` give.
class TransformStream {
  point(x, y) {
    this.stream.point(x, y);
  }
  sphere() {
    this.stream.sphere();
  }
  lineStart() {
    this.stream.lineStart();
  }
  lineEnd() {
    this.stream.lineEnd();
  }
  polygonStart() {
    this.stream.polygonStart();
  }
  polygonEnd() {
    this.stream.polygonEnd();
  }
}

function transformer(methods) {
  return (stream) => {
    const s = new TransformStream();
    Object.assign(s, methods);
    s.stream = stream;
    return s;
  };
}

const pointEqual = (a, b) =>
  Math.abs(a[0] - b[0]) < geoEpsilon && Math.abs(a[1] - b[1]) < geoEpsilon;

// Lines cut by a clip, kept for the clip to join.
function clipBuffer() {
  let lines = [];
  let line = null;
  return {
    point: (x, y, m) => line.push([x, y, m]),
    lineStart: () => lines.push((line = [])),
    lineEnd: noop,
    rejoin: () => {
      if (lines.length > 1) lines.push(lines.pop().concat(lines.shift()));
    },
    result: () => {
      const result = lines;
      lines = [];
      line = null;
      return result;
    },
  };
}

class Intersection {
  constructor(point, points, other, entry) {
    this.x = point;
    this.z = points;
    this.o = other;
    this.e = entry;
    this.v = false;
    this.n = this.p = null;
  }
}

// Joins a polygon cut into its visible segments, along the clip's edge.
function clipRejoin(segments, compareIntersection, startInside, interpolate, stream) {
  const subject = [];
  const clip = [];
  for (const segment of segments) {
    const n = segment.length - 1;
    if (n <= 0) continue;
    let p0 = segment[0];
    const p1 = segment[n];
    if (pointEqual(p0, p1)) {
      if (!p0[2] && !p1[2]) {
        stream.lineStart();
        for (let i = 0; i < n; ++i) {
          p0 = segment[i];
          stream.point(p0[0], p0[1]);
        }
        stream.lineEnd();
        continue;
      }
      // A degenerate segment: its end moves.
      p1[0] += 2 * geoEpsilon;
    }
    let x = new Intersection(p0, segment, null, true);
    subject.push(x);
    clip.push((x.o = new Intersection(p0, null, x, false)));
    x = new Intersection(p1, segment, null, false);
    subject.push(x);
    clip.push((x.o = new Intersection(p1, null, x, true)));
  }
  if (!subject.length) return;
  clip.sort(compareIntersection);
  link(subject);
  link(clip);
  for (const c of clip) c.e = startInside = !startInside;
  const start = subject[0];
  for (;;) {
    // The first intersection not visited.
    let current = start;
    let isSubject = true;
    while (current.v) if ((current = current.n) === start) return;
    let points = current.z;
    stream.lineStart();
    do {
      current.v = current.o.v = true;
      if (current.e) {
        if (isSubject) {
          for (const point of points) stream.point(point[0], point[1]);
        } else {
          interpolate(current.x, current.n.x, 1, stream);
        }
        current = current.n;
      } else {
        if (isSubject) {
          points = current.p.z;
          for (let i = points.length - 1; i >= 0; --i) stream.point(points[i][0], points[i][1]);
        } else {
          interpolate(current.x, current.p.x, -1, stream);
        }
        current = current.p;
      }
      current = current.o;
      points = current.z;
      isSubject = !isSubject;
    } while (!current.v);
    stream.lineEnd();
  }
}

function link(array) {
  const n = array.length;
  if (!n) return;
  let a = array[0];
  for (let i = 1; i < n; ++i) {
    const b = array[i];
    a.n = b;
    b.p = a;
    a = b;
  }
  a.n = array[0];
  array[0].p = a;
}

function longitude(point) {
  return Math.abs(point[0]) <= pi
    ? point[0]
    : Math.sign(point[0]) * (((Math.abs(point[0]) + pi) % tau) - pi);
}

// Whether `point` is inside the spherical `polygon`.
function polygonContains(polygon, point) {
  const lambda = longitude(point);
  let phi = point[1];
  const sinPhi = Math.sin(phi);
  const normal = [Math.sin(lambda), -Math.cos(lambda), 0];
  let angle = 0;
  let winding = 0;
  const sum = new Adder();
  if (sinPhi === 1) phi = halfPi + geoEpsilon;
  else if (sinPhi === -1) phi = -halfPi - geoEpsilon;
  for (const ring of polygon) {
    const m = ring.length;
    if (!m) continue;
    let point0 = ring[m - 1];
    let lambda0 = longitude(point0);
    const phi0 = point0[1] / 2 + quarterPi;
    let sinPhi0 = Math.sin(phi0);
    let cosPhi0 = Math.cos(phi0);
    for (let j = 0; j < m; ++j) {
      const point1 = ring[j];
      const lambda1 = longitude(point1);
      const phi1 = point1[1] / 2 + quarterPi;
      const sinPhi1 = Math.sin(phi1);
      const cosPhi1 = Math.cos(phi1);
      const delta = lambda1 - lambda0;
      const sign = delta >= 0 ? 1 : -1;
      const absDelta = sign * delta;
      const antimeridian = absDelta > pi;
      const k = sinPhi0 * sinPhi1;
      sum.add(
        Math.atan2(k * sign * Math.sin(absDelta), cosPhi0 * cosPhi1 + k * Math.cos(absDelta)),
      );
      angle += antimeridian ? delta + sign * tau : delta;
      // Whether the longitudes are either side of the point's meridian.
      if ((antimeridian !== lambda0 >= lambda) !== lambda1 >= lambda) {
        const arc = cartesianCross(cartesian(point0), cartesian(point1));
        cartesianNormalizeInPlace(arc);
        const intersection = cartesianCross(normal, arc);
        cartesianNormalizeInPlace(intersection);
        const turn = antimeridian !== delta >= 0 ? -1 : 1;
        const phiArc = turn * clampedAsin(intersection[2]);
        if (phi > phiArc || (phi === phiArc && (arc[0] || arc[1]))) winding -= turn;
      }
      [lambda0, sinPhi0, cosPhi0, point0] = [lambda1, sinPhi1, cosPhi1, point1];
    }
  }
  const south = angle < -geoEpsilon || (angle < geoEpsilon && sum.valueOf() < -geoEpsilon2);
  return south !== ((winding & 1) === 1);
}

// A clip on the sphere: lines cut where `clipLine` cuts them, polygons joined along its edge.
function clip(pointVisible, clipLine, interpolate, start) {
  return (sink) => {
    const line = clipLine(sink);
    const ringBuffer = clipBuffer();
    const ringSink = clipLine(ringBuffer);
    let polygonStarted = false;
    let polygon, segments, ring;
    const begin = () => {
      if (!polygonStarted) {
        sink.polygonStart();
        polygonStarted = true;
      }
    };
    const point = (lambda, phi) => {
      if (pointVisible(lambda, phi)) sink.point(lambda, phi);
    };
    const pointLine = (lambda, phi) => line.point(lambda, phi);
    const lineStart = () => {
      stream.point = pointLine;
      line.lineStart();
    };
    const lineEnd = () => {
      stream.point = point;
      line.lineEnd();
    };
    const pointRing = (lambda, phi) => {
      ring.push([lambda, phi]);
      ringSink.point(lambda, phi);
    };
    const ringStart = () => {
      ringSink.lineStart();
      ring = [];
    };
    const ringEnd = () => {
      pointRing(ring[0][0], ring[0][1]);
      ringSink.lineEnd();
      const clean = ringSink.clean();
      const ringSegments = ringBuffer.result();
      const n = ringSegments.length;
      ring.pop();
      polygon.push(ring);
      ring = null;
      if (!n) return;
      // No intersections.
      if (clean & 1) {
        const segment = ringSegments[0];
        const m = segment.length - 1;
        if (m > 0) {
          begin();
          sink.lineStart();
          for (let i = 0; i < m; ++i) sink.point(segment[i][0], segment[i][1]);
          sink.lineEnd();
        }
        return;
      }
      // Connected segments, joined.
      if (n > 1 && clean & 2) ringSegments.push(ringSegments.pop().concat(ringSegments.shift()));
      segments.push(ringSegments.filter((s) => s.length > 1));
    };
    const stream = {
      point,
      lineStart,
      lineEnd,
      polygonStart: () => {
        stream.point = pointRing;
        stream.lineStart = ringStart;
        stream.lineEnd = ringEnd;
        segments = [];
        polygon = [];
      },
      polygonEnd: () => {
        stream.point = point;
        stream.lineStart = lineStart;
        stream.lineEnd = lineEnd;
        segments = segments.flat();
        const startInside = polygonContains(polygon, start);
        if (segments.length) {
          begin();
          clipRejoin(segments, compareIntersection, startInside, interpolate, sink);
        } else if (startInside) {
          begin();
          sink.lineStart();
          interpolate(null, null, 1, sink);
          sink.lineEnd();
        }
        if (polygonStarted) {
          sink.polygonEnd();
          polygonStarted = false;
        }
        segments = polygon = null;
      },
      sphere: () => {
        sink.polygonStart();
        sink.lineStart();
        interpolate(null, null, 1, sink);
        sink.lineEnd();
        sink.polygonEnd();
      },
    };
    return stream;
  };
}

// Intersections in their order along the clip's edge.
function compareIntersection(a, b) {
  const [p, q] = [a.x, b.x];
  return (
    (p[0] < 0 ? p[1] - halfPi - geoEpsilon : halfPi - p[1]) -
    (q[0] < 0 ? q[1] - halfPi - geoEpsilon : halfPi - q[1])
  );
}

// Cuts lines at the antimeridian. `clean()`: 0 when cut or empty, 1 when not cut, 2 when cut
// with the first and last segments to join.
function clipAntimeridianLine(stream) {
  let lambda0 = NaN;
  let phi0 = NaN;
  let sign0 = NaN;
  let clean;
  return {
    lineStart: () => {
      stream.lineStart();
      clean = 1;
    },
    point: (lambda1, phi1) => {
      const sign1 = lambda1 > 0 ? pi : -pi;
      const delta = Math.abs(lambda1 - lambda0);
      if (Math.abs(delta - pi) < geoEpsilon) {
        // Across a pole.
        phi0 = (phi0 + phi1) / 2 > 0 ? halfPi : -halfPi;
        stream.point(lambda0, phi0);
        stream.point(sign0, phi0);
        stream.lineEnd();
        stream.lineStart();
        stream.point(sign1, phi0);
        stream.point(lambda1, phi0);
        clean = 0;
      } else if (sign0 !== sign1 && delta >= pi) {
        // Across the antimeridian.
        if (Math.abs(lambda0 - sign0) < geoEpsilon) lambda0 -= sign0 * geoEpsilon;
        if (Math.abs(lambda1 - sign1) < geoEpsilon) lambda1 -= sign1 * geoEpsilon;
        phi0 = clipAntimeridianIntersect(lambda0, phi0, lambda1, phi1);
        stream.point(sign0, phi0);
        stream.lineEnd();
        stream.lineStart();
        stream.point(sign1, phi0);
        clean = 0;
      }
      stream.point((lambda0 = lambda1), (phi0 = phi1));
      sign0 = sign1;
    },
    lineEnd: () => {
      stream.lineEnd();
      lambda0 = phi0 = NaN;
    },
    clean: () => 2 - clean,
  };
}

function clipAntimeridianIntersect(lambda0, phi0, lambda1, phi1) {
  const sinLambda0Lambda1 = Math.sin(lambda0 - lambda1);
  if (!(Math.abs(sinLambda0Lambda1) > geoEpsilon)) return (phi0 + phi1) / 2;
  const cosPhi0 = Math.cos(phi0);
  const cosPhi1 = Math.cos(phi1);
  return Math.atan(
    (Math.sin(phi0) * cosPhi1 * Math.sin(lambda1) - Math.sin(phi1) * cosPhi0 * Math.sin(lambda0)) /
      (cosPhi0 * cosPhi1 * sinLambda0Lambda1),
  );
}

function clipAntimeridianInterpolate(from, to, direction, stream) {
  if (from == null) {
    const phi = direction * halfPi;
    stream.point(-pi, phi);
    stream.point(0, phi);
    stream.point(pi, phi);
    stream.point(pi, 0);
    stream.point(pi, -phi);
    stream.point(0, -phi);
    stream.point(-pi, -phi);
    stream.point(-pi, 0);
    stream.point(-pi, phi);
  } else if (Math.abs(from[0] - to[0]) > geoEpsilon) {
    const lambda = from[0] < to[0] ? pi : -pi;
    const phi = (direction * lambda) / 2;
    stream.point(-lambda, phi);
    stream.point(0, phi);
    stream.point(lambda, phi);
  } else {
    stream.point(to[0], to[1]);
  }
}

const clipAntimeridian = clip(() => true, clipAntimeridianLine, clipAntimeridianInterpolate, [
  -pi,
  -halfPi,
]);

// Liang–Barsky: the part of the line a–b inside the rectangle, `a` and `b` moved to its ends.
function clipLine(a, b, x0, y0, x1, y1) {
  const [ax, ay] = a;
  const dx = b[0] - ax;
  const dy = b[1] - ay;
  let t0 = 0;
  let t1 = 1;
  // One side: whether the line can be inside of it, `t0` and `t1` narrowed.
  const side = (r, d, lower) => {
    if (!d && (lower ? r > 0 : r < 0)) return false;
    r /= d;
    if (lower ? d < 0 : d > 0) {
      if (r < t0) return false;
      if (r < t1) t1 = r;
    } else if (lower ? d > 0 : d < 0) {
      if (r > t1) return false;
      if (r > t0) t0 = r;
    }
    return true;
  };
  const inside =
    side(x0 - ax, dx, true) &&
    side(x1 - ax, dx, false) &&
    side(y0 - ay, dy, true) &&
    side(y1 - ay, dy, false);
  if (!inside) return false;
  if (t0 > 0) [a[0], a[1]] = [ax + t0 * dx, ay + t0 * dy];
  if (t1 < 1) [b[0], b[1]] = [ax + t1 * dx, ay + t1 * dy];
  return true;
}

const clipMax = 1e9;
const clipMin = -clipMax;
const clipBound = (v) => Math.max(clipMin, Math.min(clipMax, v));

// A clip to the rectangle of projected points from x0, y0 to x1, y1.
function clipRectangle(x0, y0, x1, y1) {
  const visible = (x, y) => x0 <= x && x <= x1 && y0 <= y && y <= y1;
  const corner = (p, direction) =>
    Math.abs(p[0] - x0) < geoEpsilon
      ? direction > 0
        ? 0
        : 3
      : Math.abs(p[0] - x1) < geoEpsilon
        ? direction > 0
          ? 2
          : 1
        : Math.abs(p[1] - y0) < geoEpsilon
          ? direction > 0
            ? 1
            : 0
          : direction > 0
            ? 3
            : 2;
  const comparePoint = (a, b) => {
    const ca = corner(a, 1);
    const cb = corner(b, 1);
    return ca !== cb
      ? ca - cb
      : ca === 0
        ? b[1] - a[1]
        : ca === 1
          ? a[0] - b[0]
          : ca === 2
            ? a[1] - b[1]
            : b[0] - a[0];
  };
  const compareIntersection = (a, b) => comparePoint(a.x, b.x);
  const interpolate = (from, to, direction, stream) => {
    let a = 0;
    let a1 = 0;
    if (
      from == null ||
      (a = corner(from, direction)) !== (a1 = corner(to, direction)) ||
      comparePoint(from, to) < 0 !== direction > 0
    ) {
      do stream.point(a === 0 || a === 3 ? x0 : x1, a > 1 ? y1 : y0);
      while ((a = (a + direction + 4) % 4) !== a1);
    } else {
      stream.point(to[0], to[1]);
    }
  };

  return (stream) => {
    let activeStream = stream;
    const bufferStream = clipBuffer();
    let segments, polygon, ring;
    let x__, y__, v__;
    let x_, y_, v_;
    let first, clean;

    const point = (x, y) => {
      if (visible(x, y)) activeStream.point(x, y);
    };
    const polygonInside = () => {
      let winding = 0;
      for (const r of polygon) {
        let [b0, b1] = r[0];
        for (let j = 1; j < r.length; ++j) {
          const [a0, a1] = [b0, b1];
          [b0, b1] = r[j];
          if (a1 <= y1) {
            if (b1 > y1 && (b0 - a0) * (y1 - a1) > (b1 - a1) * (x0 - a0)) ++winding;
          } else if (b1 <= y1 && (b0 - a0) * (y1 - a1) < (b1 - a1) * (x0 - a0)) {
            --winding;
          }
        }
      }
      return winding;
    };
    const linePoint = (x, y) => {
      const v = visible(x, y);
      if (polygon) ring.push([x, y]);
      if (first) {
        [x__, y__, v__] = [x, y, v];
        first = false;
        if (v) {
          activeStream.lineStart();
          activeStream.point(x, y);
        }
      } else if (v && v_) {
        activeStream.point(x, y);
      } else {
        x_ = clipBound(x_);
        y_ = clipBound(y_);
        x = clipBound(x);
        y = clipBound(y);
        const a = [x_, y_];
        const b = [x, y];
        if (clipLine(a, b, x0, y0, x1, y1)) {
          if (!v_) {
            activeStream.lineStart();
            activeStream.point(a[0], a[1]);
          }
          activeStream.point(b[0], b[1]);
          if (!v) activeStream.lineEnd();
          clean = false;
        } else if (v) {
          activeStream.lineStart();
          activeStream.point(x, y);
          clean = false;
        }
      }
      [x_, y_, v_] = [x, y, v];
    };
    const clipStream = {
      point,
      lineStart: () => {
        clipStream.point = linePoint;
        if (polygon) polygon.push((ring = []));
        first = true;
        v_ = false;
        x_ = y_ = NaN;
      },
      lineEnd: () => {
        if (segments) {
          linePoint(x__, y__);
          if (v__ && v_) bufferStream.rejoin();
          segments.push(bufferStream.result());
        }
        clipStream.point = point;
        if (v_) activeStream.lineEnd();
      },
      // A polygon is buffered, then clipped whole.
      polygonStart: () => {
        activeStream = bufferStream;
        segments = [];
        polygon = [];
        clean = true;
      },
      polygonEnd: () => {
        const startInside = polygonInside();
        const cleanInside = clean && startInside;
        segments = segments.flat();
        const visible = segments.length;
        if (cleanInside || visible) {
          stream.polygonStart();
          if (cleanInside) {
            stream.lineStart();
            interpolate(null, null, 1, stream);
            stream.lineEnd();
          }
          if (visible) clipRejoin(segments, compareIntersection, startInside, interpolate, stream);
          stream.polygonEnd();
        }
        activeStream = stream;
        segments = polygon = ring = null;
      },
    };
    return clipStream;
  };
}

const maxDepth = 16;
const cosMinDistance = Math.cos(30 * radians);

// Projects a stream's points, adding points along its lines where the projection bends them by
// more than the square root of `delta2`.
function resample(project, delta2) {
  return +delta2 ? resampleAdaptive(project, delta2) : resampleNone(project);
}

function resampleNone(project) {
  return transformer({
    point(x, y) {
      const p = project(x, y);
      this.stream.point(p[0], p[1]);
    },
  });
}

function resampleAdaptive(project, delta2) {
  function resampleLineTo(x0, y0, lambda0, a0, b0, c0, x1, y1, lambda1, a1, b1, c1, depth, stream) {
    const dx = x1 - x0;
    const dy = y1 - y0;
    const d2 = dx * dx + dy * dy;
    if (d2 > 4 * delta2 && depth--) {
      let a = a0 + a1;
      let b = b0 + b1;
      let c = c0 + c1;
      const m = Math.sqrt(a * a + b * b + c * c);
      c /= m;
      const phi2 = clampedAsin(c);
      const lambda2 =
        Math.abs(Math.abs(c) - 1) < geoEpsilon || Math.abs(lambda0 - lambda1) < geoEpsilon
          ? (lambda0 + lambda1) / 2
          : Math.atan2(b, a);
      const p = project(lambda2, phi2);
      const [x2, y2] = p;
      const dx2 = x2 - x0;
      const dy2 = y2 - y0;
      const dz = dy * dx2 - dx * dy2;
      if (
        (dz * dz) / d2 > delta2 ||
        Math.abs((dx * dx2 + dy * dy2) / d2 - 0.5) > 0.3 ||
        a0 * a1 + b0 * b1 + c0 * c1 < cosMinDistance
      ) {
        a /= m;
        b /= m;
        resampleLineTo(x0, y0, lambda0, a0, b0, c0, x2, y2, lambda2, a, b, c, depth, stream);
        stream.point(x2, y2);
        resampleLineTo(x2, y2, lambda2, a, b, c, x1, y1, lambda1, a1, b1, c1, depth, stream);
      }
    }
  }
  return (stream) => {
    let lambda00, x00, y00, a00, b00, c00;
    let lambda0, x0, y0, a0, b0, c0;
    const point = (x, y) => {
      const p = project(x, y);
      stream.point(p[0], p[1]);
    };
    const lineStart = () => {
      x0 = NaN;
      resampleStream.point = linePoint;
      stream.lineStart();
    };
    const linePoint = (lambda, phi) => {
      const c = cartesian([lambda, phi]);
      const p = project(lambda, phi);
      const [x1, y1, a1, b1, c1] = [p[0], p[1], c[0], c[1], c[2]];
      resampleLineTo(x0, y0, lambda0, a0, b0, c0, x1, y1, lambda, a1, b1, c1, maxDepth, stream);
      [x0, y0, lambda0, a0, b0, c0] = [x1, y1, lambda, a1, b1, c1];
      stream.point(x0, y0);
    };
    const lineEnd = () => {
      resampleStream.point = point;
      stream.lineEnd();
    };
    const ringStart = () => {
      lineStart();
      resampleStream.point = ringPoint;
      resampleStream.lineEnd = ringEnd;
    };
    const ringPoint = (lambda, phi) => {
      linePoint((lambda00 = lambda), phi);
      [x00, y00, a00, b00, c00] = [x0, y0, a0, b0, c0];
      resampleStream.point = linePoint;
    };
    const ringEnd = () => {
      resampleLineTo(
        x0,
        y0,
        lambda0,
        a0,
        b0,
        c0,
        x00,
        y00,
        lambda00,
        a00,
        b00,
        c00,
        maxDepth,
        stream,
      );
      resampleStream.lineEnd = lineEnd;
      lineEnd();
    };
    const resampleStream = {
      point,
      lineStart,
      lineEnd,
      polygonStart: () => {
        stream.polygonStart();
        resampleStream.lineStart = ringStart;
      },
      polygonEnd: () => {
        stream.polygonEnd();
        resampleStream.lineStart = lineStart;
      },
    };
    return resampleStream;
  };
}

const transformRadians = transformer({
  point(x, y) {
    this.stream.point(x * radians, y * radians);
  },
});

function transformRotate(rotate) {
  return transformer({
    point(x, y) {
      const r = rotate(x, y);
      this.stream.point(r[0], r[1]);
    },
  });
}

function scaleTranslate(k, dx, dy, sx, sy) {
  const transform = (x, y) => [dx + k * x * sx, dy - k * y * sy];
  transform.invert = (x, y) => [((x - dx) / k) * sx, ((dy - y) / k) * sy];
  return transform;
}

function scaleTranslateRotate(k, dx, dy, sx, sy, alpha) {
  if (!alpha) return scaleTranslate(k, dx, dy, sx, sy);
  const cosAlpha = Math.cos(alpha);
  const sinAlpha = Math.sin(alpha);
  const a = cosAlpha * k;
  const b = sinAlpha * k;
  const ai = cosAlpha / k;
  const bi = sinAlpha / k;
  const ci = (sinAlpha * dy - cosAlpha * dx) / k;
  const fi = (sinAlpha * dx + cosAlpha * dy) / k;
  const transform = (x, y) => {
    x *= sx;
    y *= sy;
    return [a * x - b * y + dx, dy - b * x - a * y];
  };
  transform.invert = (x, y) => [sx * (ai * x - bi * y + ci), sy * (fi - bi * x - ai * y)];
  return transform;
}

// A projection of the raw projection `projectAt(...)` gives, made again when called.
function projectionMutator(projectAt) {
  let project;
  let k = 150;
  let x = 480;
  let y = 250;
  let lambda = 0;
  let phi = 0;
  let deltaLambda = 0;
  let deltaPhi = 0;
  let deltaGamma = 0;
  let rotate;
  let alpha = 0;
  let sx = 1;
  let sy = 1;
  let x0 = null;
  let y0, x1, y1;
  let postclip = identity;
  let delta2 = 0.5;
  let projectResample, projectTransform, projectRotateTransform;

  const projection = (point) => projectRotateTransform(point[0] * radians, point[1] * radians);
  const invert = (point) => {
    const p = projectRotateTransform.invert(point[0], point[1]);
    return p && [p[0] * degrees, p[1] * degrees];
  };
  projection.stream = (stream) =>
    transformRadians(transformRotate(rotate)(clipAntimeridian(projectResample(postclip(stream)))));
  projection.clipExtent = (...a) => {
    if (!a.length) {
      return x0 == null
        ? null
        : [
            [x0, y0],
            [x1, y1],
          ];
    }
    const e = a[0];
    if (e == null) {
      x0 = y0 = x1 = y1 = null;
      postclip = identity;
    } else {
      [x0, y0, x1, y1] = [+e[0][0], +e[0][1], +e[1][0], +e[1][1]];
      postclip = clipRectangle(x0, y0, x1, y1);
    }
    return projection;
  };
  projection.scale = (...a) => (a.length ? ((k = +a[0]), recenter()) : k);
  projection.translate = (...a) =>
    a.length ? ((x = +a[0][0]), (y = +a[0][1]), recenter()) : [x, y];
  projection.center = (...a) => {
    if (!a.length) return [lambda * degrees, phi * degrees];
    lambda = (a[0][0] % 360) * radians;
    phi = (a[0][1] % 360) * radians;
    return recenter();
  };
  projection.rotate = (...a) => {
    if (!a.length) return [deltaLambda * degrees, deltaPhi * degrees, deltaGamma * degrees];
    const r = a[0];
    deltaLambda = (r[0] % 360) * radians;
    deltaPhi = (r[1] % 360) * radians;
    deltaGamma = r.length > 2 ? (r[2] % 360) * radians : 0;
    return recenter();
  };
  projection.angle = (...a) =>
    a.length ? ((alpha = (a[0] % 360) * radians), recenter()) : alpha * degrees;
  projection.reflectX = (...a) => (a.length ? ((sx = a[0] ? -1 : 1), recenter()) : sx < 0);
  projection.reflectY = (...a) => (a.length ? ((sy = a[0] ? -1 : 1), recenter()) : sy < 0);
  projection.precision = (...a) => {
    if (!a.length) return Math.sqrt(delta2);
    delta2 = a[0] * a[0];
    projectResample = resample(projectTransform, delta2);
    return projection;
  };
  projection.fitExtent = (extent, object) => fitExtent(projection, extent, object);
  projection.fitSize = (size, object) => fitSize(projection, size, object);
  projection.fitWidth = (width, object) => fitWidth(projection, width, object);
  projection.fitHeight = (height, object) => fitHeight(projection, height, object);

  function recenter() {
    const center = scaleTranslateRotate(k, 0, 0, sx, sy, alpha)(...project(lambda, phi));
    const transform = scaleTranslateRotate(k, x - center[0], y - center[1], sx, sy, alpha);
    rotate = rotateRadians(deltaLambda, deltaPhi, deltaGamma);
    projectTransform = compose(project, transform);
    projectRotateTransform = compose(rotate, projectTransform);
    projectResample = resample(projectTransform, delta2);
    return projection;
  }

  return (...args) => {
    project = projectAt(...args);
    projection.invert = project.invert && invert;
    return recenter();
  };
}

// Scales and translates `projection` so that `object` fits the bounds `fitBounds` makes.
function fit(projection, fitBounds, object) {
  const clip = projection.clipExtent();
  projection.scale(150).translate([0, 0]);
  if (clip != null) projection.clipExtent(null);
  const bounds = boundsStream();
  geoStream(object, projection.stream(bounds));
  fitBounds(bounds.result());
  if (clip != null) projection.clipExtent(clip);
  return projection;
}

function fitExtent(projection, extent, object) {
  return fit(
    projection,
    (b) => {
      const w = extent[1][0] - extent[0][0];
      const h = extent[1][1] - extent[0][1];
      const k = Math.min(w / (b[1][0] - b[0][0]), h / (b[1][1] - b[0][1]));
      const x = +extent[0][0] + (w - k * (b[1][0] + b[0][0])) / 2;
      const y = +extent[0][1] + (h - k * (b[1][1] + b[0][1])) / 2;
      projection.scale(150 * k).translate([x, y]);
    },
    object,
  );
}

function fitSize(projection, size, object) {
  return fitExtent(projection, [[0, 0], size], object);
}

function fitWidth(projection, width, object) {
  return fit(
    projection,
    (b) => {
      const w = +width;
      const k = w / (b[1][0] - b[0][0]);
      const x = (w - k * (b[1][0] + b[0][0])) / 2;
      const y = -k * b[0][1];
      projection.scale(150 * k).translate([x, y]);
    },
    object,
  );
}

function fitHeight(projection, height, object) {
  return fit(
    projection,
    (b) => {
      const h = +height;
      const k = h / (b[1][1] - b[0][1]);
      const x = -k * b[0][0];
      const y = (h - k * (b[1][1] + b[0][1])) / 2;
      projection.scale(150 * k).translate([x, y]);
    },
    object,
  );
}

function mercatorRaw(lambda, phi) {
  return [lambda, Math.log(Math.tan((halfPi + phi) / 2))];
}
mercatorRaw.invert = (x, y) => [x, 2 * Math.atan(Math.exp(y)) - halfPi];

export function geoMercator() {
  const m = projectionMutator(() => mercatorRaw)();
  const { center, scale, translate, clipExtent } = m;
  let x0 = null;
  let y0, x1, y1;
  // The clip extent follows the scale and translation, so the poles never show.
  const reclip = () => {
    const k = pi * scale();
    const t = m(geoRotation(m.rotate()).invert([0, 0]));
    return clipExtent(
      x0 == null
        ? [
            [t[0] - k, t[1] - k],
            [t[0] + k, t[1] + k],
          ]
        : [
            [Math.max(t[0] - k, x0), y0],
            [Math.min(t[0] + k, x1), y1],
          ],
    );
  };
  m.scale = (...a) => (a.length ? (scale(a[0]), reclip()) : scale());
  m.translate = (...a) => (a.length ? (translate(a[0]), reclip()) : translate());
  m.center = (...a) => (a.length ? (center(a[0]), reclip()) : center());
  m.clipExtent = (...a) => {
    if (!a.length) {
      return x0 == null
        ? null
        : [
            [x0, y0],
            [x1, y1],
          ];
    }
    const e = a[0];
    if (e == null) x0 = y0 = x1 = y1 = null;
    else [x0, y0, x1, y1] = [+e[0][0], +e[0][1], +e[1][0], +e[1][1]];
    return reclip();
  };
  return reclip().scale(961 / tau);
}

function cylindricalEqualAreaRaw(phi0) {
  const cosPhi0 = Math.cos(phi0);
  const forward = (lambda, phi) => [lambda * cosPhi0, Math.sin(phi) / cosPhi0];
  forward.invert = (x, y) => [x / cosPhi0, clampedAsin(y * cosPhi0)];
  return forward;
}

function conicEqualAreaRaw(y0, y1) {
  const sy0 = Math.sin(y0);
  const n = (sy0 + Math.sin(y1)) / 2;
  // Parallels symmetric about the equator.
  if (Math.abs(n) < geoEpsilon) return cylindricalEqualAreaRaw(y0);
  const c = 1 + sy0 * (2 * n - sy0);
  const r0 = Math.sqrt(c) / n;
  const project = (x, y) => {
    const r = Math.sqrt(c - 2 * n * Math.sin(y)) / n;
    x *= n;
    return [r * Math.sin(x), r0 - r * Math.cos(x)];
  };
  project.invert = (x, y) => {
    const r0y = r0 - y;
    let l = Math.atan2(x, Math.abs(r0y)) * Math.sign(r0y);
    if (r0y * n < 0) l -= pi * Math.sign(x) * Math.sign(r0y);
    return [l / n, clampedAsin((c - (x * x + r0y * r0y) * n * n) / (2 * n))];
  };
  return project;
}

export function geoAlbers() {
  let phi0 = 0;
  let phi1 = pi / 3;
  const m = projectionMutator(conicEqualAreaRaw);
  const p = m(phi0, phi1);
  p.parallels = (...a) =>
    a.length
      ? m((phi0 = a[0][0] * radians), (phi1 = a[0][1] * radians))
      : [phi0 * degrees, phi1 * degrees];
  return p
    .scale(155.424)
    .center([0, 33.6442])
    .parallels([29.5, 45.5])
    .scale(1070)
    .translate([480, 250])
    .rotate([96, 0])
    .center([-0.6, 38.7]);
}

// Path data of a stream, as d3's PathString writes it.
class GeoPathString {
  constructor(digits) {
    let k = null;
    if (digits != null) {
      const d = Math.floor(digits);
      if (!(d >= 0)) throw new RangeError(`invalid digits: ${digits}`);
      if (d <= 15) k = 10 ** d;
    }
    this._k = k;
    this._radius = 4.5;
    this._parts = [];
    this._line = NaN;
    this._point = NaN;
  }
  // Text, and numbers, rounded.
  _append(...items) {
    for (const [i, item] of items.entries()) {
      const number = i % 2 === 1;
      this._parts.push(number && this._k !== null ? Math.round(item * this._k) / this._k : item);
    }
  }
  pointRadius(r) {
    this._radius = +r;
    return this;
  }
  polygonStart() {
    this._line = 0;
  }
  polygonEnd() {
    this._line = NaN;
  }
  lineStart() {
    this._point = 0;
  }
  lineEnd() {
    if (this._line === 0) this._parts.push("Z");
    this._point = NaN;
  }
  point(x, y) {
    if (this._point === 0) {
      this._append("M", x, ",", y);
      this._point = 1;
    } else if (this._point === 1) {
      this._append("L", x, ",", y);
    } else {
      const r = this._radius;
      this._append("M", x, ",", y, "m0,", r, "a", r, ",", r, " 0 1,1 0,", -2 * r, "a", r, ",");
      this._append("", r, " 0 1,1 0,", 2 * r, "z");
    }
  }
  result() {
    const t = concat(this._parts);
    this._parts = [];
    return traced(t) ? t : t || null;
  }
}

function boundsStream() {
  let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
  return {
    point: (x, y) => {
      if (x < x0) x0 = x;
      if (x > x1) x1 = x;
      if (y < y0) y0 = y;
      if (y > y1) y1 = y;
    },
    lineStart: noop,
    lineEnd: noop,
    polygonStart: noop,
    polygonEnd: noop,
    result: () => [
      [x0, y0],
      [x1, y1],
    ],
  };
}

function areaStream() {
  const areaSum = new Adder();
  let ringSum = new Adder();
  let x00, y00, x0, y0;
  const pointFirst = (x, y) => {
    stream.point = point;
    [x00, y00, x0, y0] = [x, y, x, y];
  };
  const point = (x, y) => {
    ringSum.add(y0 * x - x0 * y);
    [x0, y0] = [x, y];
  };
  const stream = {
    point: noop,
    lineStart: noop,
    lineEnd: noop,
    polygonStart: () => {
      stream.lineStart = () => (stream.point = pointFirst);
      stream.lineEnd = () => point(x00, y00);
    },
    polygonEnd: () => {
      stream.lineStart = stream.lineEnd = stream.point = noop;
      areaSum.add(Math.abs(ringSum.valueOf()));
      ringSum = new Adder();
    },
    result: () => areaSum.valueOf() / 2,
  };
  return stream;
}

// The centroid of the polygons, else of the lines, else of the points.
function centroidStream() {
  let [X0, Y0, Z0, X1, Y1, Z1, X2, Y2, Z2] = [0, 0, 0, 0, 0, 0, 0, 0, 0];
  let x00, y00, x0, y0;
  const point = (x, y) => {
    X0 += x;
    Y0 += y;
    ++Z0;
  };
  const along = (x, y) => {
    const dx = x - x0;
    const dy = y - y0;
    const z = Math.sqrt(dx * dx + dy * dy);
    X1 += (z * (x0 + x)) / 2;
    Y1 += (z * (y0 + y)) / 2;
    Z1 += z;
  };
  const pointFirstLine = (x, y) => {
    stream.point = pointLine;
    point((x0 = x), (y0 = y));
  };
  const pointLine = (x, y) => {
    along(x, y);
    point((x0 = x), (y0 = y));
  };
  const lineStart = () => (stream.point = pointFirstLine);
  const lineEnd = () => (stream.point = point);
  const pointFirstRing = (x, y) => {
    stream.point = pointRing;
    [x00, y00] = [x, y];
    point((x0 = x), (y0 = y));
  };
  const pointRing = (x, y) => {
    along(x, y);
    const z = y0 * x - x0 * y;
    X2 += z * (x0 + x);
    Y2 += z * (y0 + y);
    Z2 += z * 3;
    point((x0 = x), (y0 = y));
  };
  const stream = {
    point,
    lineStart,
    lineEnd,
    polygonStart: () => {
      stream.lineStart = () => (stream.point = pointFirstRing);
      stream.lineEnd = () => pointRing(x00, y00);
    },
    polygonEnd: () => {
      stream.point = point;
      stream.lineStart = lineStart;
      stream.lineEnd = lineEnd;
    },
    result: () =>
      Z2 ? [X2 / Z2, Y2 / Z2] : Z1 ? [X1 / Z1, Y1 / Z1] : Z0 ? [X0 / Z0, Y0 / Z0] : [NaN, NaN],
  };
  return stream;
}

// Path data of GeoJSON through a projection, null for none. Scenes draw path data, so it has no
// context.
export function geoPath(projection, context) {
  let digits = 3;
  let pointRadius = 4.5;
  let projectionStream = identity;
  const noContext = () => {
    throw new Error("geoPath() gives path data for <path d>; it draws on no context");
  };
  if (context != null) noContext();

  function path(object, ...rest) {
    if (!object) return null;
    const out = new GeoPathString(digits);
    out.pointRadius(
      typeof pointRadius === "function" ? +pointRadius(object, ...rest) : pointRadius,
    );
    geoStream(object, projectionStream(out));
    return out.result();
  }
  const measure = (stream) => (object) => {
    const s = stream();
    geoStream(object, projectionStream(s));
    return s.result();
  };
  path.area = measure(areaStream);
  path.bounds = measure(boundsStream);
  path.centroid = measure(centroidStream);
  path.projection = (...a) => {
    if (!a.length) return projection;
    projection = a[0] == null ? null : a[0];
    projectionStream = projection == null ? identity : projection.stream;
    return path;
  };
  path.context = (...a) => {
    if (!a.length) return null;
    if (a[0] != null) noContext();
    return path;
  };
  path.pointRadius = (...a) => {
    if (!a.length) return pointRadius;
    pointRadius = typeof a[0] === "function" ? a[0] : +a[0];
    return path;
  };
  path.digits = (...a) => {
    if (!a.length) return digits;
    if (a[0] == null) {
      digits = null;
    } else {
      const d = Math.floor(a[0]);
      if (!(d >= 0)) throw new RangeError(`invalid digits: ${a[0]}`);
      digits = d;
    }
    return path;
  };
  return path.projection(projection ?? null);
}

// ------------------------------------------------------------------------------ Valle's own

// Boxes of `size` placed next to their `point`s without overlapping each other or leaving
// `bounds`, `[left, top, right, bottom]`: by `priority`, highest first, each takes the first of
// nine places around its point that is free, or is hidden. Each gives `{ x, y, visible }`, its
// box's top left, in the labels' order.
export function placeLabels(labels, options = {}) {
  const { bounds = [-Infinity, -Infinity, Infinity, Infinity], padding = 4 } = options;
  const [left, top, right, bottom] = bounds;
  const gap = Math.max(0, padding);
  const order = labels.map((_, i) => i);
  order.sort((a, b) => (labels[b].priority ?? 0) - (labels[a].priority ?? 0) || a - b);
  const placed = [];
  const out = [];
  const free = (r) =>
    r.left >= left &&
    r.top >= top &&
    r.right <= right &&
    r.bottom <= bottom &&
    placed.every(
      (o) =>
        r.right + gap <= o.left ||
        o.right + gap <= r.left ||
        r.bottom + gap <= o.top ||
        o.bottom + gap <= r.top,
    );
  for (const i of order) {
    const [x, y] = labels[i].point;
    const [w, h] = labels[i].size;
    const places = [
      [x - w / 2, y - h - gap],
      [x - w / 2, y + gap],
      [x + gap, y - h / 2],
      [x - w - gap, y - h / 2],
      [x - w / 2, y - h / 2],
      [x + gap, y - h - gap],
      [x - w - gap, y - h - gap],
      [x + gap, y + gap],
      [x - w - gap, y + gap],
    ];
    out[i] = { x: x - w / 2, y: y - h / 2, visible: false };
    for (const [px, py] of places) {
      const r = { left: px, top: py, right: px + w, bottom: py + h };
      if (free(r)) {
        placed.push(r);
        out[i] = { x: px, y: py, visible: true };
        break;
      }
    }
  }
  return out;
}

// A layered layout of a graph of nodes of `sizes`, `[width, height]`, and `edges`, `[from, to]`
// node indices, flowing `direction` "down" or "right": ranks by longest path with the edges that
// close cycles left out, rows ordered by their neighbours, and nodes lined up with them. It gives
// the nodes' `centers`, `[x, y]`, in a box from 0, 0 of `width` and `height`, their `ranks`, the
// `rows` of node indices in order, and which edges are `backEdges`.
export function graphLayout(options) {
  const { sizes, edges = [], direction = "down", nodeGap = 40, rankGap = 64 } = options;
  if (direction !== "down" && direction !== "right") {
    throw new RangeError(`graphLayout's direction is "down" or "right", not ${direction}`);
  }
  const n = sizes.length;
  for (const [from, to] of edges) {
    for (const v of [from, to]) {
      if (!(Number.isInteger(v) && v >= 0 && v < n)) {
        throw new RangeError(
          `graphLayout's edge [${from}, ${to}] names ${v}, and the nodes are 0 to ${n - 1}`,
        );
      }
    }
  }
  // Across and along the flow.
  const size = sizes.map(([w, h]) => (direction === "down" ? [w, h] : [h, w]));
  const backEdges = graphBackEdges(n, edges);
  const ranks = graphRanks(n, edges, backEdges);
  const rows = graphRows(n, edges, ranks);
  const across = graphCenters(rows, size, edges, nodeGap, rankGap);
  // From 0 on both axes.
  const left = Math.min(...across.map(([c], v) => c - size[v][0] / 2));
  const wide = Math.max(0, ...across.map(([c], v) => c + size[v][0] / 2 - left));
  const deep = Math.max(0, ...across.map(([, f], v) => f + size[v][1] / 2));
  const centers = across.map(([c, f]) => (direction === "down" ? [c - left, f] : [f, c - left]));
  const [width, height] = direction === "down" ? [wide, deep] : [deep, wide];
  return { centers, width, height, ranks, rows, backEdges };
}

// Which edges go back to a node on the path to them, in a depth-first walk.
function graphBackEdges(n, edges) {
  const out = sequence(n).map(() => []);
  for (const [e, [from]] of edges.entries()) out[from].push(e);
  const state = sequence(n).map(() => 0);
  const back = edges.map(() => false);
  for (let root = 0; root < n; root++) {
    if (state[root] !== 0) continue;
    const stack = [[root, 0]];
    state[root] = 1;
    while (stack.length) {
      const top = stack[stack.length - 1];
      const u = top[0];
      if (top[1] < out[u].length) {
        const e = out[u][top[1]++];
        const v = edges[e][1];
        if (state[v] === 0) {
          state[v] = 1;
          stack.push([v, 0]);
        } else if (state[v] === 1) {
          back[e] = true;
        }
      } else {
        state[u] = 2;
        stack.pop();
      }
    }
  }
  return back;
}

// Longest-path ranks without the back edges; a node with no edges is at 0.
function graphRanks(n, edges, back) {
  const indegree = sequence(n).map(() => 0);
  const out = sequence(n).map(() => []);
  for (const [e, [from, to]] of edges.entries()) {
    if (back[e] || from === to) continue;
    out[from].push(to);
    indegree[to]++;
  }
  const rank = sequence(n).map(() => 0);
  const queue = sequence(n).filter((v) => indegree[v] === 0);
  for (let i = 0; i < queue.length; i++) {
    const u = queue[i];
    for (const v of out[u]) {
      rank[v] = Math.max(rank[v], rank[u] + 1);
      if (--indegree[v] === 0) queue.push(v);
    }
  }
  return rank;
}

// Each rank's nodes, ordered by the mean place of their neighbours in the rank before, then
// after, twice over.
function graphRows(n, edges, rank) {
  const rows = sequence(Math.max(0, ...rank) + 1).map(() => []);
  for (let v = 0; v < n; v++) rows[rank[v]].push(v);
  const pos = [];
  const sync = () => rows.forEach((row) => row.forEach((v, i) => (pos[v] = i)));
  sync();
  for (let sweep = 0; sweep < 4; sweep++) {
    const down = sweep % 2 === 0;
    for (let k = 0; k < rows.length; k++) {
      const r = down ? k : rows.length - 1 - k;
      const want = down ? r - 1 : r + 1;
      const keyed = rows[r].map((v) => {
        let s = 0;
        let c = 0;
        for (const [from, to] of edges) {
          const other = from === v ? to : to === v ? from : -1;
          if (other >= 0 && rank[other] === want) {
            s += pos[other];
            c++;
          }
        }
        return [c === 0 ? pos[v] : s / c, pos[v], v];
      });
      keyed.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
      rows[r] = keyed.map((k) => k[2]);
      sync();
    }
  }
  return rows;
}

// Centers across and along the flow: ranks one after another, each as deep as its deepest node;
// across, nodes at the mean of their neighbours as far as their order and gaps let them, then
// chains lined up straight where there is room.
function graphCenters(rows, size, edges, nodeGap, rankGap) {
  const n = size.length;
  const center = sequence(n).map(() => [0, 0]);
  let along = 0;
  for (const row of rows) {
    const depth = Math.max(0, ...row.map((v) => size[v][1]));
    for (const v of row) center[v][1] = along + depth / 2;
    along += depth + rankGap;
  }
  for (const row of rows) {
    let x = 0;
    for (const v of row) {
      center[v][0] = x + size[v][0] / 2;
      x += size[v][0] + nodeGap;
    }
  }
  for (let pass = 0; pass < 3; pass++) {
    for (const row of rows) {
      const want = row.map((v) => {
        let s = 0;
        let c = 0;
        for (const [from, to] of edges) {
          const other = from === v ? to : to === v ? from : -1;
          if (other >= 0) {
            s += center[other][0];
            c++;
          }
        }
        return c === 0 ? center[v][0] : s / c;
      });
      const placed = [];
      let edge = -Infinity;
      for (const [i, v] of row.entries()) {
        const half = size[v][0] / 2;
        const p = Math.max(want[i], edge + half);
        placed.push(p);
        edge = p + half + nodeGap;
      }
      const mean = (xs) => xs.reduce((a, b) => a + b, 0) / Math.max(1, xs.length);
      const d = mean(placed) - mean(want);
      for (const [i, v] of row.entries()) center[v][0] = placed[i] - d;
    }
  }
  const into = sequence(n).map(() => []);
  const from = sequence(n).map(() => []);
  for (const [f, t] of edges) {
    from[f].push(t);
    into[t].push(f);
  }
  const rowOf = [];
  for (const [r, row] of rows.entries()) for (const [i, v] of row.entries()) rowOf[v] = [r, i];
  // Moves `v` across to `x` unless that crowds its row's neighbours.
  const snap = (v, x) => {
    const [r, i] = rowOf[v];
    const row = rows[r];
    const half = size[v][0] / 2;
    if (i > 0) {
      const l = row[i - 1];
      if (x - half < center[l][0] + size[l][0] / 2 + nodeGap) return;
    }
    if (i + 1 < row.length) {
      const k = row[i + 1];
      if (x + half > center[k][0] - size[k][0] / 2 - nodeGap) return;
    }
    center[v][0] = x;
  };
  for (let pass = 0; pass < 2; pass++) {
    // Under a single parent, or parents lined up.
    for (const row of rows) {
      for (const v of row) {
        const ps = into[v];
        if (!ps.length) continue;
        const x0 = center[ps[0]][0];
        if (ps.every((u) => Math.abs(center[u][0] - x0) < 1e-6)) snap(v, x0);
      }
    }
    // A source over its single child.
    for (const row of rows) {
      for (const v of row) {
        if (!into[v].length && from[v].length === 1) snap(v, center[from[v][0]][0]);
      }
    }
  }
  return center;
}
