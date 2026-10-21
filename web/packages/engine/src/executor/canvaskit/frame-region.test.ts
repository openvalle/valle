import { expect, test } from "bun:test";
import { frameRegion } from "./executor.ts";

// The vectors of the native `frame_regions_round_each_edge_to_the_nearest_pixel_inside_the_frame`.
test("frame regions round each edge to the nearest pixel inside the frame", () => {
  const affine = (m: number[]) => [...m, 0, 0, 1];
  const cases: Array<[number[], [number, number], [number, number, number, number]]> = [
    [[64, 0, 0, 0, 32, 0], [64, 32], [0, 0, 64, 32]],
    [[192, 0, 0, 0, 128, 32], [192, 192], [0, 32, 192, 128]],
    [[100, 0, 0, 0, 200 / 3, 50 / 3], [100, 100], [0, 17, 100, 66]],
    [[10, 0, 10.5, 0, 10, 0], [64, 32], [11, 0, 10, 10]],
    [[0, 20, 10, 20, 0, 5], [64, 32], [10, 5, 20, 20]],
    [[10, 0, 100, 0, 10, -40], [64, 32], [63, 0, 1, 1]],
  ];
  for (const [matrix, [width, height], [x, y, w, h]] of cases) {
    expect(frameRegion(affine(matrix), { width, height })).toEqual({ x, y, width: w, height: h });
  }
  expect(() => frameRegion([1, 0, 0, 0, 1, 0, 1, 0, -1], { width: 64, height: 32 })).toThrow("horizon");
});
