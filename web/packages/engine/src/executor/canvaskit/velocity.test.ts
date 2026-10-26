import { expect, test } from "bun:test";
import { velocityBlurUniforms } from "./velocity.ts";

const input = { x: -7, y: -3, width: 83, height: 61 };
const output = { x: 0, y: 0, width: 73, height: 51 };

test("velocity GPU admission preserves half-texel ties and oversized filter resolution", () => {
  expect(velocityBlurUniforms(input, output, [-10, -2.5], [10, 2.5])).toBeNull();
  expect(velocityBlurUniforms(input, output, [-0.5005, 0], [0, 0])).toBeNull();
  expect(velocityBlurUniforms({ ...input, x: 65536 }, output, [-0.51, 0], [0, 0])).toBeNull();
  expect(velocityBlurUniforms({ ...input, width: 4096 }, output, [-4, 0], [4, 0])).toBeNull();
  expect(velocityBlurUniforms(input, output, [0, 0], [0, 0])).toBeNull();
});

test("velocity GPU uniforms keep source placement, the original alpha matrix and shutter order", () => {
  const uniforms = velocityBlurUniforms(input, output, [-4, 0], [4, 0])!;
  expect(Array.from(uniforms.slice(0, 6))).toEqual([83, 61, -7, -3, 9, 1]);
  expect(uniforms[21]).toBe(Math.fround(1 / 9));
  expect(Array.from(uniforms.slice(26, 44))).toEqual([
    -4, 0, -3, 0, -2, 0, -1, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0,
  ]);
  expect(Array.from(uniforms.slice(44))).toEqual(Array.from({ length: 46 }, () => 0));
});
