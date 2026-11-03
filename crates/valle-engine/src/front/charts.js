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
