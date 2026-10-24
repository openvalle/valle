import { expect, test } from "bun:test";
import { samplingFootprint } from "./sampling-footprint.ts";

test("zero lens coefficients preserve the shader identity outside its frame", () => {
  const output = { x: -700, y: 240, width: 1900, height: 1100 };
  expect(
    samplingFootprint({ kind: "lensDistortion", k1: 0, k2: 0 }, output, [0, 0, 10, 10]),
  ).toEqual(output);
});
test("chromatic bounds include both shifted bilinear taps for negative offsets", () => {
  expect(
    samplingFootprint(
      { kind: "chromaticAberration", offset: [-12, 8] },
      { x: -100, y: 20, width: 160, height: 90 },
    ),
  ).toEqual({ x: -108, y: 14, width: 176, height: 102 });
});
test("lens polynomial footprint contains sample taps over random subrectangles and signs", () => {
  let state = 1729;
  const random = () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
  for (let trial = 0; trial < 1000; trial++) {
    const frame = [
      Math.fround(random() * 4000 - 2000),
      Math.fround(random() * 4000 - 2000),
      Math.fround(32 + random() * 4000),
      Math.fround(32 + random() * 4000),
    ];
    const [fx, fy, w, h] = frame,
      output = {
        x: Math.floor(fx + random() * w * 0.8),
        y: Math.floor(fy + random() * h * 0.8),
        width: Math.ceil(w * (0.05 + random() * 0.8)),
        height: Math.ceil(h * (0.05 + random() * 0.8)),
      };
    const filter = {
      kind: "lensDistortion",
      k1: Math.fround(random() - 0.5),
      k2: Math.fround(random() - 0.5),
    };
    const roi = samplingFootprint(filter, output, frame);
    expect(roi).toBeTruthy();
    for (let i = 0; i < 100; i++) {
      const x = output.x + Math.floor(random() * output.width) + 0.5,
        y = output.y + Math.floor(random() * output.height) + 0.5;
      if (x < fx || x >= fx + w || y < fy || y >= fy + h) continue;
      const dx = x - fx - w / 2,
        dy = y - fy - h / 2,
        r = Math.min((dx * dx + dy * dy) / ((w * w + h * h) / 4), 1);
      const s = 1 + filter.k1 * r + filter.k2 * r * r;
      const sx = fx + w / 2 + dx * s - 0.5,
        sy = fy + h / 2 + dy * s - 0.5;
      expect(Math.floor(sx) >= roi!.x && Math.floor(sx) + 1 < roi!.x + roi!.width).toBeTruthy();
      expect(Math.floor(sy) >= roi!.y && Math.floor(sy) + 1 < roi!.y + roi!.height).toBeTruthy();
    }
  }
});
test("unknown or malformed sampling operators keep their original input", () => {
  expect(samplingFootprint({ kind: "radialBlur" }, { x: 0, y: 0, width: 20, height: 20 })).toBe(
    null,
  );
  expect(
    samplingFootprint(
      { kind: "lensDistortion", k1: NaN, k2: 0 },
      { x: 0, y: 0, width: 20, height: 20 },
      [0, 0, 20, 20],
    ),
  ).toBe(null);
  for (const offset of [[], [1], [1, 2, 3], [NaN, 0], [0, Infinity]]) {
    expect(
      samplingFootprint(
        { kind: "chromaticAberration", offset },
        { x: 0, y: 0, width: 20, height: 20 },
      ),
    ).toBe(null);
  }
});
