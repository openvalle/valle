import type { RectWire } from "./draw-program.ts";

/** A bounded integer device translation cancels exactly against the scheduled ROI origin
 * in Skia's f32 canvas matrix. Keep fractional phase, projective transforms and crop sizes.
 * Only use this for destination-independent raster passes, never filters or backdrop reads. */
export function localRasterPlacement(matrix: number[], roi: RectWire) {
  const integer = (value: number) => Number.isInteger(value) && Math.abs(value) <= 1_048_576;
  if (
    matrix[6] !== 0 ||
    matrix[7] !== 0 ||
    matrix[8] !== 1 ||
    !integer(matrix[2]!) ||
    !integer(matrix[5]!) ||
    !integer(roi.x) ||
    !integer(roi.y)
  ) {
    return { matrix, roi };
  }
  const local = matrix.slice();
  local[2] = matrix[2]! - roi.x;
  local[5] = matrix[5]! - roi.y;
  return { matrix: local, roi: { ...roi, x: 0, y: 0 } };
}
