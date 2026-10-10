import { expect, test } from "bun:test";
import { decodeScene3dLightingInput as decode } from "./scene3d-input.ts";
function packet() {
  return Float32Array.from([
    1,
    2,
    2,
    1,
    0,
    0,
    5,
    0.03125,
    ...Array(12).fill(0),
    0,
    0,
    2,
    2,
    1,
    3,
    0,
    0,
    1,
    4,
    1,
    1,
    1,
  ]);
}
test("lighting spans retain transfer ownership and exact pixel/depth facts", () => {
  const p = packet(),
    v = decode(p, 2, 2, 1);
  expect(v.spans.buffer).toBe(p.buffer);
  expect([...v.spans]).toEqual([3, 0, 0, 1, 4, 1, 1, 1]);
  expect(v.eye.length).toBe(3);
  expect(v.columns.length).toBe(6);
  expect(v.rows.length).toBe(6);
  expect(v).toMatchObject({ x: 0, y: 0, width: 2, height: 2, count: 1 });
});
test("wrong version, dimensions, cropped bounds and truncated data are rejected", () => {
  expect(() => decode(undefined, 2, 2, 1)).toThrow();
  expect(() => decode(packet(), 3, 2, 1)).toThrow();
  for (const [i, n] of [
    [0, 2],
    [3, 2],
    [4, NaN],
    [7, 0],
    [20, -1],
    [22, 3],
    [24, 5],
  ]) {
    const p = packet();
    p[i!] = n!;
    expect(() => decode(p, 2, 2, 1)).toThrow();
  }
  expect(() => decode(packet().subarray(0, 31), 2, 2, 1)).toThrow();
});
