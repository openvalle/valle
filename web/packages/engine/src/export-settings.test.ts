import { expect, test } from "bun:test";
import { resolveExportPlan, type BrowserVideoExportSettings } from "./export-settings.ts";

const receipt = { canvasWidth: 1920, canvasHeight: 1080, frameRate: "30/1", frameCount: 300,
  sampleRate: 48_000, sampleCount: 480_000 };

test("default delivery preserves the admitted frame and sample schedule", () => {
  const plan = resolveExportPlan(receipt, {});
  expect([plan.width, plan.height, plan.frameCount, plan.frameRate, plan.durationS, plan.resize]).toEqual([1920, 1080, 300, "30/1", 10, false]);
  for (let frame = 0; frame < 300; frame++) {
    expect(plan.sourceFrame(frame)).toBe(frame);
    expect(plan.timestamp(frame)).toBe(Math.round(frame / 30 * 1_000_000) / 1_000_000);
  }
  expect(plan.endTimestamp(299)).toBe(10);
});

test("changing FPS drops or repeats source frames without changing work duration", () => {
  const lower = resolveExportPlan(receipt, { frameRate: 24 });
  expect(lower.frameCount).toBe(240);
  expect([0, 1, 2, 3, 4, 239].map((frame) => lower.sourceFrame(frame))).toEqual([0, 1, 2, 3, 5, 298]);
  expect(lower.endTimestamp(239)).toBe(10);
  const higher = resolveExportPlan(receipt, { frameRate: 60 });
  expect(higher.frameCount).toBe(600);
  expect([0, 1, 2, 3, 599].map((frame) => higher.sourceFrame(frame))).toEqual([0, 0, 1, 1, 299]);
  expect(higher.endTimestamp(599)).toBe(10);
});

test("fractional source clocks and partial final frames retain exact duration", () => {
  const info = { ...receipt, frameRate: "30000/1001", frameCount: 30, sampleCount: 48_048 };
  const original = resolveExportPlan(info, {});
  expect(original.frameRate).toBe("30000/1001");
  expect(original.frameCount).toBe(30);
  expect(original.endTimestamp(29)).toBe(1.001);
  const converted = resolveExportPlan(info, { frameRate: 25 });
  expect(converted.frameCount).toBe(26);
  expect(converted.timestamp(25)).toBe(1);
  expect(converted.endTimestamp(25)).toBe(1.001);
  expect(converted.sourceFrame(25)).toBe(29);
  expect(converted.partialFinalFrame).toBe(true);
  expect(original.partialFinalFrame).toBe(false);
  const short = resolveExportPlan({ ...receipt, frameCount: 1, sampleCount: 1600 }, { frameRate: 24 });
  expect(short.frameCount).toBe(1);
  expect(short.endTimestamp(0)).toBe(0.033333);
});

test("output resizing contains the whole source and preserves independent audio settings", () => {
  const plan = resolveExportPlan(receipt, { width: 320, height: 240, videoBitrate: 700_000, audioBitrate: 128_000 });
  expect([plan.width, plan.height, plan.rasterScale, plan.resize, plan.audioBitrate]).toEqual([320, 240, 1 / 6, true, 128_000]);
  expect(plan.durationS).toBe(10);
});

test("invalid delivery settings fail before producing an encoding plan", () => {
  const invalid: BrowserVideoExportSettings[] = [{ width: 640 }, { width: 641, height: 360 },
    { width: 9000, height: 5000 }, { frameRate: NaN }, { frameRate: 0 }, { frameRate: 121 },
    { videoBitrate: 0 }, { videoBitrate: 200_000_001 }, { audioBitrate: 31_999 },
    { includeAudio: "yes" as unknown as boolean }];
  for (const settings of invalid) expect(() => resolveExportPlan(receipt, settings)).toThrow();
});
