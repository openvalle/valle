import { expect, test } from "bun:test";
import { halfValue, inspectWorkingF16, compareWorkingF16 } from "./working-f16.ts";
const pixel = (...words: number[]) => {
  const b = new Uint8Array(words.length * 2),
    v = new DataView(b.buffer);
  words.forEach((w, i) => v.setUint16(i * 2, w, true));
  return b;
};
test("binary16 captures retain HDR, negative values and subnormals", () => {
  expect(halfValue(0x4000)).toBe(2);
  expect(halfValue(0xb000)).toBe(-0.125);
  expect(halfValue(1)).toBe(2 ** -24);
  const data = pixel(0x4000, 0xb000, 0x3400, 0x3800);
  expect(inspectWorkingF16(data)).toEqual({
    minimum: -0.125,
    maximum: 2,
    nonfinite: 0,
    negative: 1,
    hdr: 1,
  });
  expect(compareWorkingF16(data, data, { rgb: 0, alpha: 0 }).differentBits).toBe(0);
  // Both values would clip to the same white code in an 8-bit SDR delivery.
  expect(
    compareWorkingF16(data, pixel(0x4200, 0xb000, 0x3400, 0x3800), { rgb: 0.002, alpha: 0.001 })
      .pass,
  ).toBe(false);
});
test("working comparison rejects lost alpha, invisible RGB changes and nonfinite payloads", () => {
  const base = pixel(0x3800, 0, 0, 0x3800);
  for (const changed of [
    pixel(0x3800, 0, 0, 0x3400),
    pixel(0x3c00, 0, 0, 0x3800),
    pixel(0x7e00, 0, 0, 0x3800),
    pixel(0x7c00, 0, 0, 0x3800),
  ])
    expect(compareWorkingF16(base, changed, { rgb: 0.002, alpha: 0.001 }).pass).toBe(false);
  expect(() => inspectWorkingF16(new Uint8Array(7))).toThrow();
  expect(() => compareWorkingF16(base, new Uint8Array(4), { rgb: 0, alpha: 0 })).toThrow();
});
