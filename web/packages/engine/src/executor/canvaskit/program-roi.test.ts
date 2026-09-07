import { describe, expect, test } from "bun:test";

import { projectProgramResourceRoi } from "./executor.ts";

describe("program resource ROI admission", () => {
  test("mirrors Rust's conservative program-AABB then device projection", () => {
    // local x'=x+y turns the square into a parallelogram whose program-space AABB is [0,20]x[0,10].
    // The device transform contains the opposing shear. Directly composing both transforms would
    // produce x=[50,150], but Rust deliberately projects the intermediate AABB to x=[0,200].
    expect(projectProgramResourceRoi(
      { x: 0, y: 0, width: 20, height: 10 },
      { x: 0, y: 0, width: 10, height: 10 },
      [1, 1, 0, 0, 1, 0, 0, 0, 1],
      [100, -100, 100, 0, 100, 0, 0, 0, 1],
      { width: 200, height: 100 },
    )).toEqual({ x: 0, y: 0, width: 200, height: 100 });
  });
});
