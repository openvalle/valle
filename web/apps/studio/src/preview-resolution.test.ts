import { expect, test } from "bun:test";
import { previewRasterScale } from "./preview-resolution.ts";

test("4K previews fit the stage at device resolution within a 1080p cap", () => {
  const canvas = { width: 3840, height: 2160 };
  expect(previewRasterScale(canvas, { width: 960, height: 540 }, 2)).toBe(0.5);
  expect(previewRasterScale(canvas, { width: 480, height: 270 }, 2)).toBe(0.25);
  expect(previewRasterScale(canvas, { width: 3840, height: 2160 }, 2)).toBe(0.5);
  expect(previewRasterScale({ width: 128, height: 72 }, canvas, 2)).toBe(1);
  expect(previewRasterScale(canvas, { width: 0, height: 0 })).toBe(0.5);
});
