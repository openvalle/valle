import { expect, test } from "bun:test";
import type { Paint } from "canvaskit-wasm";
import { RasterPaintCache } from "./raster-paint-cache.ts";

test("exact gradients share native paints and changing a stop creates a separate paint", () => {
  let deleted = 0;
  const cache = new RasterPaintCache(),
    create = () =>
      ({
        delete() {
          deleted++;
        },
      }) as unknown as Paint;
  const first = {
    kind: "linearGradient",
    value: { start: [0, 0], end: [10, 10], stops: [{ offset: 0, color: [1, 0, 0, 1] }] },
  };
  const a = cache.acquire(first, create),
    b = cache.acquire(JSON.parse(JSON.stringify(first)), create);
  expect(b.paint).toBe(a.paint);
  a.release();
  b.release();
  const c = cache.acquire({ ...first, value: { ...first.value, end: [20, 10] } }, create);
  expect(c.paint).not.toBe(a.paint);
  c.release();
  expect(cache.hits).toBe(1);
  cache.dispose();
  expect(deleted).toBe(2);
});

test("live leases cannot be evicted and an oversized fill remains caller-owned", () => {
  let deleted = 0;
  const create = () =>
      ({
        delete() {
          deleted++;
        },
      }) as unknown as Paint,
    cache = new RasterPaintCache(650);
  const a = cache.acquire({ r: 1 }, create),
    b = cache.acquire({ r: 2 }, create);
  expect(deleted).toBe(0);
  b.release();
  b.release();
  expect(deleted).toBe(1);
  expect(cache.acquire({ r: 1 }, create).paint).toBe(a.paint);
  a.release();
  cache.dispose();
  expect(deleted).toBe(2);
  const small = new RasterPaintCache(1),
    c = small.acquire({}, create);
  small.dispose();
  expect(deleted).toBe(2);
  c.release();
  expect(deleted).toBe(3);
});
