import { expect, test } from "bun:test";
import type { DrawProgramWire } from "../canvaskit/draw-program.ts";
import { GpuCircles } from "./circles.ts";

function batch(): DrawProgramWire {
  const count = 64, view = new DataView(new ArrayBuffer(count * 72));
  for (let i = 0; i < count; i++) {
    view.setFloat64(i * 48, 1, true);
    view.setFloat64(i * 48 + 24, 1, true);
    view.setFloat64(i * 48 + 32, i * 3, true);
    view.setFloat32(count * 48 + i * 16 + 12, 1, true);
    view.setFloat32(count * 64 + i * 4, 1, true);
  }
  return { roots: [0], paths: [], paints: [], requirements: {},
    viewport: { x: 0, y: 0, width: 256, height: 256 },
    strokeColors: { count: 0, view: new DataView(new ArrayBuffer(0)), bytes: new Uint8Array(0) },
    dashOffsets: { count: 0, view: new DataView(new ArrayBuffer(0)), bytes: new Uint8Array(0) },
    nodes: [{ kind: "instanceBatch", value: {
    shape: { kind: "circle" }, instances: { start: 0, count },
  }}], batchInstances: { count, view, bytes: new Uint8Array(view.buffer) } };
}

test("ROI cropping rejects an entire run if a late row needs Skia coverage", () => {
  const gpu = Object.create(GpuCircles.prototype) as GpuCircles;
  const draw = batch();
  expect(gpu.canCrop(draw, [0])).toBe(true);
  draw.batchInstances.view.setFloat32(64 * 68 + 63 * 4, 1, true);
  expect(gpu.canCrop(draw, [0])).toBe(false);
  draw.batchInstances.view.setFloat32(64 * 68 + 63 * 4, 0, true);
  draw.batchInstances.view.setFloat64(63 * 48, 5, true);
  expect(gpu.canCrop(draw, [0])).toBe(false);
});

test("ROI admission includes the device transform and rejects perspective or skew", () => {
  const gpu = Object.create(GpuCircles.prototype) as GpuCircles;
  expect(gpu.canCrop(batch(), [0], [5,0,0,0,5,0,0,0,1])).toBe(false);
  expect(gpu.canCrop(batch(), [0], [1,.1,0,0,1,0,0,0,1])).toBe(false);
  expect(gpu.canCrop(batch(), [0], [1,0,0,0,1,0,.01,0,1])).toBe(false);
  expect(gpu.canCrop(batch(), [0], [1,0,0,0,1,0,0,0,1])).toBe(true);
});
