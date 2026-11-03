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
const { traced, fixed, color: parseColor } = __charts;
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
