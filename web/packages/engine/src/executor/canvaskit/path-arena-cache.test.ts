import { expect, test } from "bun:test";
import { PathArenaCache } from "./path-arena-cache.ts";
import { encodeTestPaths } from "./path-arena.test-helper.ts";
import type { PathWire } from "./draw-program.ts";

const wire: PathWire = {
  verbs: ["moveTo", "lineTo"],
  points: [
    [Math.PI, -0],
    [Number.MIN_VALUE, 14.123456789012344],
  ],
};

test("binary contours retain exact f64 values and cache independently copied buffers", () => {
  const cache = new PathArenaCache(),
    bytes = encodeTestPaths([wire]);
  const a = cache.decode(bytes, 1),
    b = cache.decode(bytes.slice(), 1);
  expect(a).toBe(b);
  expect(a).toEqual([wire]);
  expect(Object.is(a[0]!.points[0]![1], -0)).toBe(true);
  expect(() => {
    a[0]!.points[0]![0] = 100;
  }).toThrow();
  const changed = encodeTestPaths([
    {
      ...wire,
      points: [
        [7, 8],
        [9, 10],
      ],
    },
  ]);
  expect(cache.decode(changed, 1)).not.toBe(a);
  cache.clear();
  expect(cache.decode(bytes, 1)).not.toBe(a);
});

test("changing one path shares every unchanged contour while byte mutations remain observable", () => {
  const cache = new PathArenaCache(),
    empty: PathWire = { verbs: [], points: [] };
  const bytes = encodeTestPaths([wire, empty]),
    a = cache.decode(bytes, 2);
  new DataView(bytes.buffer).setFloat64(10, 8, true);
  const b = cache.decode(bytes, 2);
  expect(b[0]).not.toBe(a[0]);
  expect(b[1]).toBe(a[1]);
  expect(a[0]!.points[0]![0]).toBe(Math.PI);
  expect(b[0]!.points[0]![0]).toBe(8);
  expect(cache.contourHits).toBe(1);
});

test("path row lengths, counts, verbs, coordinates and aggregate budgets fail before reuse", () => {
  for (const mutate of [
    (b: Uint8Array) => {
      b[8] = 255;
    },
    (b: Uint8Array) => {
      new DataView(b.buffer).setUint32(4, 1, true);
    },
    (b: Uint8Array) => {
      new DataView(b.buffer).setFloat64(10, Infinity, true);
    },
    (b: Uint8Array) => {
      new DataView(b.buffer).setUint32(4, 0xffffffff, true);
    },
  ]) {
    const cache = new PathArenaCache(),
      bytes = encodeTestPaths([wire]);
    cache.decode(bytes, 1);
    mutate(bytes);
    expect(() => cache.decode(bytes, 1)).toThrow();
  }
  const bytes = encodeTestPaths([wire]),
    cache = new PathArenaCache();
  expect(() => cache.decode(bytes.subarray(0, -1), 1)).toThrow();
  expect(() => cache.decode(bytes, 0)).toThrow();
  expect(() => cache.decode(bytes, 2)).toThrow();
  expect(() => cache.decode(new Uint8Array(), -1)).toThrow();
  expect(cache.decode(new Uint8Array(), 0)).toEqual([]);
});

test("an insufficient cache budget preserves admission and transfers no mutable packet ownership", () => {
  const cache = new PathArenaCache(1),
    bytes = encodeTestPaths([wire]);
  expect(cache.decode(bytes, 1)).not.toBe(cache.decode(bytes, 1));
  expect(cache.hits).toBe(0);
  const storage = new Uint8Array(bytes.length + 6);
  storage.set(bytes, 3);
  expect(cache.decode(storage.subarray(3, -3), 1)).toEqual([wire]);
});

test("a deliberate hash collision cannot alias different contour bytes", () => {
  const path: PathWire = {
    verbs: ["moveTo", "lineTo", "lineTo", "close"],
    points: [
      [1, 2],
      [3, 4],
      [5, 6],
    ],
  };
  const a = encodeTestPaths([path]),
    b = a.slice(),
    view = new DataView(b.buffer);
  const digest = (bytes: Uint8Array, end = bytes.length) => {
    const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
    let h = 2166136261;
    for (let at = 0; at < end; at += 4) h = Math.imul(h ^ v.getUint32(at, true), 16777619);
    return h >>> 0;
  };
  view.setUint32(12, 12345, true);
  const inverse = 899433627;
  const priorToLast = Math.imul(digest(a), inverse) ^ view.getUint32(56, true);
  view.setUint32(52, digest(b, 52) ^ Math.imul(priorToLast, inverse), true);
  expect(digest(b)).toBe(digest(a));
  const cache = new PathArenaCache(),
    first = cache.decode(a, 1),
    second = cache.decode(b, 1);
  expect(second).not.toBe(first);
  expect(second[0]).not.toBe(first[0]);
  expect(second[0]!.points[0]![0]).toBe(view.getFloat64(12, true));
  expect(cache.decode(a, 1)).toEqual([path]);
});

test("large arenas with identical index samples retain separate exact collision candidates", () => {
  const path: PathWire = {
    verbs: Array.from({ length: 4100 }, (_, i) => (i === 0 ? "moveTo" : "lineTo")),
    points: Array.from({ length: 4100 }, (_, i) => [i, i % 17]),
  };
  const bytes = encodeTestPaths([path]),
    changed = bytes.slice();
  const coordinate = 8 + path.verbs.length + 500 * 16;
  new DataView(changed.buffer).setFloat64(coordinate, 501.25, true);
  const cache = new PathArenaCache(),
    first = cache.decode(bytes, 1),
    second = cache.decode(changed, 1);
  expect(first[0]!.points[500]![0]).toBe(500);
  expect(second[0]!.points[500]![0]).toBe(501.25);
  expect(cache.decode(bytes.slice(), 1)).toBe(first);
  expect(cache.decode(changed.slice(), 1)).toBe(second);
  new DataView(changed.buffer).setFloat64(coordinate, NaN, true);
  expect(() => cache.decode(changed, 1)).toThrow();
  expect(first[0]!.points[500]![0]).toBe(500);
});
