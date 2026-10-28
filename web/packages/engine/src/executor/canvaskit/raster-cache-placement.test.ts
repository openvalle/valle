import { expect, test } from "bun:test";
import { localRasterPlacement } from "./raster-cache-placement.ts";

test("bounded integer translation keeps identical local raster coordinates", () => {
  let expected: ReturnType<typeof localRasterPlacement> | undefined;
  for (const delta of [-1_000_000, -512, -1, 0, 17, 999_999]) {
    const matrix = [1.25, 0.125, 30 + delta, -0.25, 0.75, 44 - delta, 0, 0, 1];
    const roi = { x: 20 + delta, y: 36 - delta, width: 128, height: 96 };
    const before = structuredClone({ matrix, roi });
    const local = localRasterPlacement(matrix, roi);
    expect({ matrix, roi }).toEqual(before);
    if (expected) expect(local).toEqual(expected);
    else expected = local;
    expect(Math.fround(Math.fround(matrix[2]!) - Math.fround(roi.x))).toBe(local.matrix[2]!);
    expect(Math.fround(Math.fround(matrix[5]!) - Math.fround(roi.y))).toBe(local.matrix[5]!);
  }
});

test("fractional, projective, large and cropped placements cannot share normalized keys", () => {
  const matrix = [1, 0, 30, 0, 1, 44, 0, 0, 1];
  const roi = { x: 20, y: 36, width: 128, height: 96 };
  const key = localRasterPlacement(matrix, roi);
  for (const [index, value] of [
    [2, 30.5],
    [5, 44.125],
    [2, 2_000_000],
    [6, 0.001],
    [8, 0.5],
  ]) {
    const changed = matrix.slice();
    changed[index!] = value!;
    expect(localRasterPlacement(changed, roi)).toEqual({ matrix: changed, roi });
    expect(localRasterPlacement(changed, roi)).not.toEqual(key);
  }
  for (const changed of [
    { ...roi, width: 127 },
    { ...roi, x: 21 },
  ])
    expect(localRasterPlacement(matrix, changed)).not.toEqual(key);
});
