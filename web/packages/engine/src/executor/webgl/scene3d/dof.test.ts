import { expect, test } from "bun:test";
import { needsDofTiles } from "./dof.ts";
test("wholly defocused geometry omits classification while focus-crossing ranges retain it", () => {
  const floats = new Float32Array(27);
  [4.25, 9, 9].forEach((z, i) => (floats[i * 9 + 2] = 1 / z));
  const packet = {
    radius: 16,
    focus: 3,
    stride: 9,
    vertices: 3,
    vertexData: { width: 7, height: 1, words: new Uint32Array(floats.buffer) },
  };
  expect(needsDofTiles(packet)).toBe(false);
  expect(needsDofTiles({ ...packet, focus: 9 })).toBe(true);
  expect(needsDofTiles({ ...packet, radius: 0.4 })).toBe(true);
  expect(needsDofTiles({ ...packet, radius: 0 })).toBe(false);
});

test("GPU-deformed meshes classify real depth without scanning absent projected vertices", () => {
  const packet = {
    radius: 8,
    focus: 5,
    stride: 13,
    vertices: 300,
    vertexData: { width: 1, height: 1, words: new Uint32Array(4) },
    mesh: { width: 64, height: 64, bytes: 65536, key: "source" },
  };
  expect(needsDofTiles(packet)).toBe(true);
  expect(needsDofTiles({ ...packet, radius: 0 })).toBe(false);
});
