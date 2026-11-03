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
      this._append("A", r, ",", r, ",0,0,", +(y01 * x20 > x01 * y20), ",");
      this._append("", (this._x1 = x1 + t21 * x21), ",", (this._y1 = y1 + t21 * y21));
    }
  }
  arc(x, y, r, a0, a1, ccw) {
    [x, y, r, ccw] = [+x, +y, +r, !!ccw];
    if (r < 0) throw new Error(`negative radius: ${r}`);
    const [dx, dy] = [r * Math.cos(a0), r * Math.sin(a0)];
    const [x0, y0] = [x + dx, y + dy];
    const cw = 1 ^ ccw;
    let da = ccw ? a0 - a1 : a1 - a0;
    if (this._x1 === null) {
      this._append("M", x0, ",", y0);
    } else if (Math.abs(this._x1 - x0) > pathEpsilon || Math.abs(this._y1 - y0) > pathEpsilon) {
      this._append("L", x0, ",", y0);
    }
    if (!r) return;
    if (da < 0) da = (da % tau) + tau;
    if (da > tauEpsilon) {
      this._append("A", r, ",", r, ",0,1,", cw, ",", x - dx, ",", y - dy);
      this._append("A", r, ",", r, ",0,1,", cw, ",", (this._x1 = x0), ",");
      this._append("", (this._y1 = y0));
    } else if (da > pathEpsilon) {
      const [x1, y1] = [x + r * Math.cos(a1), y + r * Math.sin(a1)];
      if (da >= pi && this._equal(x0, y0, x1, y1)) {
        da /= 2;
        const a00 = a0 + da;
        this._append("A", r, ",", r, ",0,", +(da >= pi), ",", cw, ",");
        this._append("", x + r * Math.cos(a00), ",", y + r * Math.sin(a00));
      }
      this._append("A", r, ",", r, ",0,", +(da >= pi), ",", cw, ",", (this._x1 = x1), ",");
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
