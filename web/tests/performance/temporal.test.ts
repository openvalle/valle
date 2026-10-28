import { expect, test } from "bun:test";
import { compareFrames } from "./pixels.ts";
import { compareSequence, type TemporalFrame } from "./temporal.ts";

const rule = { windowFrames: 4, rgbRange: 2, alphaRange: 2, minimumPixels: 1, minimumWindowRun: 2 };
const image = (gray: number, alpha = 255) => ({
  width: 1,
  height: 1,
  data: Uint8Array.from([gray, gray, gray, alpha]),
});
const sequence = (errors: number[]): TemporalFrame[] =>
  errors.map((e, frame) => ({ frame, reference: image(80), actual: image(80 + e) }));

test("normal plus/minus one quantization and a constant bias pass; sign alone never fails", () => {
  const normal = compareSequence(sequence([1, -1, 1, -1, 1, -1, 1, -1]), rule);
  expect(normal.pass).toBe(true);
  expect(normal.regions[0]!.rgbRangeMax).toBe(2);
  expect(normal.regions[0]!.signChanges).toBeGreaterThan(0);
  expect(compareSequence(sequence([2, 2, 2, 2, 2, 2, 2, 2]), rule).pass).toBe(true);
});

test("periodic errors inside the single-frame tolerance fail with amplitude, area and duration", () => {
  const frames = sequence([2, 2, -2, -2, 2, 2, -2, -2]);
  expect(frames.every((f) => compareFrames(f.reference, f.actual, { rgb: 2, alpha: 2 }).pass)).toBe(
    true,
  );
  const result = compareSequence(frames, rule);
  expect(result.pass).toBe(false);
  expect(result.regions[0]!.rgbRangeMax).toBe(4);
  expect(result.regions[0]!.affectedPixels).toBe(1);
  expect(result.regions[0]!.maximumAffectedSpanFrames).toBe(8);
});

test("transparent RGB is ignored, but alpha-only flicker is measured independently and on white", () => {
  const frames = sequence([2, 2, -2, -2, 2, 2, -2, -2]);
  for (const f of frames) {
    f.reference.data[3] = 0;
    f.actual.data[3] = 0;
  }
  expect(compareSequence(frames, rule).pass).toBe(true);
  for (const f of frames) {
    f.reference = image(0, 128);
    f.actual = image(0, f.frame % 4 < 2 ? 130 : 126);
  }
  const result = compareSequence(frames, rule);
  expect(result.pass).toBe(false);
  expect(result.regions[0]!.alphaRangeMax).toBe(4);
  expect(result.regions[0]!.whiteRangeMax).toBe(4);
});

test("known motion separates constant object-edge residuals from real object-local flicker", () => {
  const row = (values: number[]) => ({
    width: 6,
    height: 1,
    data: Uint8Array.from(values.flatMap((v) => [v, v, v, 255])),
  });
  const frames = Array.from({ length: 8 }, (_, frame) => {
    const x = frame % 2;
    const reference = row([80, 80, 80, 80, 80, 80]),
      actual = row([80, 80, 80, 80, 80, 80]);
    for (let c = 0; c < 3; c++) {
      actual.data[x * 4 + c] = 82;
      actual.data[(x + 1) * 4 + c] = 78;
    }
    return { frame, reference, actual };
  });
  const track = {
    name: "box",
    width: 2,
    height: 1,
    positions: frames.map((f) => ({ frame: f.frame, x: f.frame % 2, y: 0 })),
  };
  expect(compareSequence(frames, rule).pass).toBe(false);
  expect(compareSequence(frames, rule, [track]).pass).toBe(true);
  for (const f of frames)
    for (let c = 0; c < 3; c++) f.actual.data[(f.frame % 2) * 4 + c] = f.frame % 4 < 2 ? 82 : 78;
  expect(compareSequence(frames, rule, [track]).regions[1]!.pass).toBe(false);
});

test("missing frames, malformed tracks, dimensions and vacuous comparisons fail closed", () => {
  const frames = sequence([0, 0, 0, 0, 0, 0, 0, 0]);
  expect(() => compareSequence(frames.slice(0, 2), rule)).toThrow();
  expect(() => compareSequence([frames[0]!, ...frames.slice(2)], rule)).toThrow();
  expect(() => compareSequence(frames, { ...rule, minimumPixels: 0 })).toThrow();
  expect(() => compareSequence(frames, { ...rule, rgbRange: NaN })).toThrow();
  expect(() =>
    compareSequence(frames, rule, [
      {
        name: "box",
        width: 2,
        height: 1,
        positions: frames.map((f) => ({ frame: f.frame, x: 0, y: 0 })),
      },
    ]),
  ).toThrow();
  frames[0]!.actual = { ...frames[0]!.actual, width: 2 };
  expect(() => compareSequence(frames, rule)).toThrow();
});
