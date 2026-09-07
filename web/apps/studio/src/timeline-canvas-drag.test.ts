import { expect, test } from "bun:test";

import { canvasDragPosition, crossedCanvasDragThreshold } from "./timeline-canvas-drag.ts";

const origin = { startX: 100, startY: 50, baseX: 0.5, baseY: 0.25 };

test("canvas drag maps display pixels to relative transform coordinates", () => {
  expect(crossedCanvasDragThreshold(origin, 102, 52, 3)).toBe(false);
  expect(crossedCanvasDragThreshold(origin, 103, 50, 3)).toBe(true);
  expect(canvasDragPosition(origin, 164, 86, 640, 360)).toEqual({ x: 0.6, y: 0.35 });
});

test("canvas drag rejects a collapsed preview surface", () => {
  expect(canvasDragPosition(origin, 120, 70, 0, 360)).toBeNull();
});
