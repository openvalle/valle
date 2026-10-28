import { expect, test } from "bun:test";
import { decodeScene3dGpuPacket as decode } from "./scene3d-input.ts";

function packet(blend = false, shadow = false) {
  const bytes = new Uint8Array(256 + 112 + 16 + 16 + 16 + (shadow ? 96 : 0));
  const h = new Uint32Array(bytes.buffer, 0, 64);
  const f = new Float32Array(bytes.buffer, 0, 64);
  h.set([0x33534756, 16, 16, 3, blend ? 13 : 9, blend ? 3 : 0]);
  f.set([1, 5, 0], 6);
  h.set([7, 1, 256, 1, 1, 368], 9);
  h.set([1, 1, 16, 1, 1, 384, 1, 1, 400], 15);
  h[24] = Number(shadow);
  h[25] = bytes.byteLength;
  h[27] = Number(blend);
  f.set([0, 0, 16, 16], 44);
  if (blend) {
    const expanded = new Uint8Array(bytes.length + 48);
    expanded.set(bytes.subarray(0, 368));
    expanded.set(bytes.subarray(368), 416);
    const eh = new Uint32Array(expanded.buffer, 0, 64);
    eh[9] = 10;
    for (const at of [14, 20, 23]) eh[at]! += 48;
    eh[25] = expanded.length;
    new Uint32Array(expanded.buffer, eh[23], 4).set([0, 1, 0, 0]);
    if (shadow) setShadow(expanded, 464);
    return expanded;
  }
  if (shadow) setShadow(bytes, 416);
  return bytes;
}

function setShadow(bytes: Uint8Array, offset: number) {
  new Uint32Array(bytes.buffer, 0, 64).set([3, 6, 1, offset], 48);
  new Float32Array(bytes.buffer, 0, 64).set([-2, 0.1], 52);
}

test("triangle uploads borrow their transferred storage, including tiled transparency and shadows", () => {
  const bytes = packet(true, true),
    p = decode(bytes);
  expect(p.vertexData.words.buffer).toBe(bytes.buffer);
  expect(p).toMatchObject({
    width: 16,
    height: 16,
    vertices: 3,
    stride: 13,
    lit: true,
    blend: true,
    bounds: [0, 0, 16, 16],
  });
  expect(p.shadows[0]).toMatchObject({ width: 6, height: 1, vertices: 3, minimum: -2 });
  expect([...p.tiles.words.subarray(0, 2)]).toEqual([0, 1]);
  expect(decode(packet()).lit).toBe(false);
});

test("truncated, overlapping, oversized and invalid triangle or tile packets are rejected", () => {
  expect(() => decode(packet().subarray(0, 400))).toThrow();
  expect(() => decode(new Uint8Array(packet().length + 1).subarray(1))).toThrow();
  for (const [at, value] of [
    [0, 0],
    [1, 4097],
    [3, 2],
    [4, 8],
    [5, 16],
    [11, 0],
    [14, 256],
    [24, 3],
    [25, 0],
    [27, 0],
  ]) {
    const bytes = packet(true);
    new Uint32Array(bytes.buffer)[at!] = value!;
    expect(() => decode(bytes)).toThrow();
  }
  const bytes = packet(true),
    h = new Uint32Array(bytes.buffer);
  h[h[23]! / 4 + 1] = 5;
  expect(() => decode(bytes)).toThrow("tile references");
  const shadow = packet(false, true);
  new Float32Array(shadow.buffer)[53] = 0;
  expect(() => decode(shadow)).toThrow("shadow data");
});

test("immutable mesh packets separate source storage from per-frame parameters", () => {
  const bytes = packet(true),
    h = new Uint32Array(bytes.buffer);
  h[3] = 30;
  h[5] = 17;
  h[27] = 0;
  h.set([16, 4, 1024], 48);
  const p = decode(bytes);
  expect(p.vertices).toBe(30);
  expect(p.mesh).toMatchObject({ width: 16, height: 4, bytes: 1024 });
  for (const [at, value] of [
    [5, 19],
    [24, 1],
    [48, 0],
    [49, 4097],
    [50, 16],
    [3, 66],
  ]) {
    const invalid = bytes.slice();
    new Uint32Array(invalid.buffer)[at!] = value!;
    expect(() => decode(invalid)).toThrow();
  }
});
