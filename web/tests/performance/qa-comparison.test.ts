import { expect, test } from "bun:test";
import { comparisonCases } from "./qa-comparison.ts";
import { compareFrames } from "./pixels.ts";

const run = () => ({
  settings: { qa: { width: 3, height: 1, order: [0, 2, 0] } },
  identities: { cases: { rectangles: { source: "same-fixture" } } },
  qa: [{ case: "rectangles", backend: "native-cpu", optimizationRule: { rgb: 2, alpha: 2 } }],
});

test("optimization admission never weakens the exact repeat gate", () => {
  const before = run(),
    after = run();
  const frame = (red: number) => ({ width: 1, height: 1, data: Uint8Array.from([red, 0, 0, 255]) });
  expect(
    compareFrames(frame(100), frame(102), comparisonCases(before, after, "registered")[0]!.rule)
      .pass,
  ).toBe(true);
  expect(
    compareFrames(frame(100), frame(102), comparisonCases(before, after, "exact")[0]!.rule).pass,
  ).toBe(false);
});

test("missing, duplicate, and extra backend captures fail admission", () => {
  for (const change of [
    (r: ReturnType<typeof run>) => {
      r.qa = [];
    },
    (r: ReturnType<typeof run>) => {
      r.qa.push(r.qa[0]!);
    },
    (r: ReturnType<typeof run>) => {
      r.qa[0]!.backend = "web-gpu";
    },
  ]) {
    const after = run();
    change(after);
    expect(() => comparisonCases(run(), after, "registered")).toThrow();
  }
});

test("a changed fixture, frame order, or tolerance fails instead of blessing new pixels", () => {
  const missingIdentity = { ...run(), identities: { cases: {} } };
  expect(() => comparisonCases(missingIdentity, missingIdentity, "registered")).toThrow();
  for (const change of [
    (r: ReturnType<typeof run>) => {
      r.identities.cases.rectangles.source = "different";
    },
    (r: ReturnType<typeof run>) => {
      r.settings.qa.order = [2, 0, 2];
    },
    (r: ReturnType<typeof run>) => {
      r.qa[0]!.optimizationRule.rgb = 64;
    },
  ]) {
    const after = run();
    change(after);
    expect(() => comparisonCases(run(), after, "registered")).toThrow();
  }
});

test("registered comparison still rejects lost detail, wrong frames and wrong colors", () => {
  const rule = comparisonCases(run(), run(), "registered")[0]!.rule;
  const row = (values: number[]) => ({
    width: 3,
    height: 1,
    data: Uint8Array.from(values.flatMap((v) => [v, v, v, 255])),
  });
  const reference = row([16, 255, 16]);
  for (const actual of [row([16, 16, 16]), row([255, 16, 16]), row([16, 128, 16])])
    expect(compareFrames(reference, actual, rule).pass).toBe(false);
});

test("continuous acceptance requires the same registered temporal thresholds and trajectory", () => {
  const make = () => ({
    ...run(),
    settings: { qa: { continuousFrames: 8 } },
    qa: [
      {
        ...run().qa[0]!,
        temporal: {
          rule: {
            windowFrames: 4,
            rgbRange: 2,
            alphaRange: 2,
            minimumPixels: 1,
            minimumWindowRun: 2,
          },
          tracks: [{ name: "moving", width: 1, height: 1, positions: [{ frame: 0, x: 1, y: 0 }] }],
        },
      },
    ],
  });
  expect(comparisonCases(make(), make(), "registered")).toHaveLength(1);
  for (const change of [
    (r: ReturnType<typeof make>) => {
      r.qa[0]!.temporal.rule.rgbRange = 4;
    },
    (r: ReturnType<typeof make>) => {
      r.qa[0]!.temporal.tracks[0]!.positions[0]!.x = 2;
    },
  ]) {
    const after = make();
    change(after);
    expect(() => comparisonCases(make(), after, "registered")).toThrow();
  }
  const missing = { ...make(), qa: run().qa };
  expect(() => comparisonCases(missing, missing, "registered")).toThrow();
});
