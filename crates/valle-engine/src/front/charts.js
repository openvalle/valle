// Charts: the subset of d3 v7 that scenes call as built-ins, by d3's names and with d3's results
// (design §2.8.8). It is scene code to the frontend: rewritten for tracing as a scene's modules
// are, so a value that depends on the time goes through it as ops. It keeps no state.
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
  let [cx0, cy0] = [(D * dy - dx * d) / d2, (-D * dx - dy * d) / d2];
  const [cx1, cy1] = [(D * dy + dx * d) / d2, (-D * dx + dy * d) / d2];
  const [dx0, dy0, dx1, dy1] = [cx0 - x00, cy0 - y00, cx1 - x00, cy1 - y00];
  // The closer of the two.
  if (dx0 * dx0 + dy0 * dy0 > dx1 * dx1 + dy1 * dy1) [cx0, cy0] = [cx1, cy1];
  return {
    cx: cx0,
    cy: cy0,
    x01: -ox,
    y01: -oy,
    x11: cx0 * (r1 / r - 1),
    y11: cy0 * (r1 / r - 1),
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
