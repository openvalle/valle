import { expect, test } from "bun:test";
import type { SkPicture } from "canvaskit-wasm";
import type { DrawProgramWire } from "./draw-program.ts";
import { RasterTreeCache } from "./raster-tree-cache.ts";

test("vector recordings use a bounded cache and retire owned pictures once", () => {
  const cache = new RasterTreeCache(240);
  const deleted: string[] = [];
  const signature = (key: string) => ({ key, leaves: 8, bytes: 64 });
  const record = (key: string) => () => ({
    approximateBytesUsed: () => 64,
    delete: () => deleted.push(key),
  } as unknown as SkPicture);
  expect(cache.acquire(signature("a"), record("a"))).toBeNull();
  const a = cache.acquire(signature("a"), record("a"));
  expect(cache.acquire(signature("a"), record("unexpected"))).toBe(a);
  expect(cache.acquire(signature("b"), record("b"))).toBeNull();
  expect(cache.acquire(signature("b"), record("b"))).not.toBeNull();
  expect(deleted).toEqual(["a"]);
  cache.dispose();
  cache.dispose();
  expect(deleted).toEqual(["a", "b"]);

  const tiny = new RasterTreeCache(128);
  const large = () => ({ approximateBytesUsed: () => 128, delete: () => deleted.push("large") } as unknown as SkPicture);
  expect(tiny.acquire(signature("c"), large)).toBeNull();
  expect(tiny.acquire(signature("c"), large)).toBeNull();
  expect(deleted).toEqual(["a", "b", "large"]);
  tiny.dispose();
});

test("scope identity includes transforms, clip rules and changed fill or stroke paints", () => {
  const cache = new RasterTreeCache();
  const path = Object.freeze({
    verbs: Object.freeze(["moveTo", "lineTo", "lineTo", "close"]),
    points: Object.freeze([[0, 0], [8, 0], [0, 8]].map(point => Object.freeze(point))),
  });
  const group = {
    children: [0], transform: [1, 0, 0, 0, 1, 0, 0, 0, 1], opacity: 1,
    clip: {kind: "path", value: {path: 0, fillRule: "nonZero"}}, filters: [],
    mask: null, shader: null, transition: null,
    backdrop: null, internalBlend: "normal",
  };
  const draw = {
    paths: [path], paints: [{kind: "solid", value: [1, 0, 0, 1]}, {kind: "solid", value: [0, 1, 0, 1]}],
    nodes: [{kind: "path", value: {path: 0, fill: 0, fillRule: "nonZero", stroke: {paint: 1, width: 1}}},
      {kind: "group", value: group}],
  } as unknown as DrawProgramWire;
  const key = () => cache.signature(draw, 1, new Map())?.key;
  const original = key();
  expect(original).toBeDefined();
  group.transform[2] = 1;
  expect(key()).not.toBe(original);
  group.transform[2] = 0;
  group.clip.value.fillRule = "evenOdd";
  expect(key()).not.toBe(original);
  group.clip.value.fillRule = "nonZero";
  (draw.paints[0]!.value as number[])[0] = 0.5;
  expect(key()).not.toBe(original);
  (draw.paints[0]!.value as number[])[0] = 1;
  (draw.paints[1]!.value as number[])[1] = 0.5;
  expect(key()).not.toBe(original);
  (draw.paints[1]!.value as number[])[1] = 1;
  expect(key()).toBe(original);
  // A new frame gets a new memo; a resource-backed child cannot become a vector recording.
  draw.nodes[0] = {kind: "image", value: {}};
  expect(key()).toBeUndefined();
  cache.dispose();
});
