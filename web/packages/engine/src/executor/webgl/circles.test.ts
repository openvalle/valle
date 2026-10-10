import { expect, test } from "bun:test";
import { GpuCircles } from "./circles.js";

function batch() {
  const count = 64, view = new DataView(new ArrayBuffer(count * 72));
  for (let i = 0; i < count; i++) {
    view.setFloat64(i * 48, 1, true);
    view.setFloat64(i * 48 + 24, 1, true);
    view.setFloat64(i * 48 + 32, i * 3, true);
    view.setFloat32(count * 48 + i * 16 + 12, 1, true);
    view.setFloat32(count * 64 + i * 4, 1, true);
  }
  return { nodes: [{ kind: "instanceBatch", value: {
    shape: { kind: "circle" }, instances: { start: 0, count },
  }}], batchInstances: { count, view } };
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
