import { describe, expect, test } from "bun:test";

import { CanvasKitCompositorError, admitOptimizationReport } from "./executor.ts";

const hash = (value: string) => `sha256:${value.repeat(64)}`;

function optimizedPlan() {
  return {
    optimization: {
      inputPhysicalHash: hash("a"),
      outputPhysicalHash: hash("b"),
      rewrites: [{
        kind: "noOpGroupElimination",
        pass: 2,
        output: 3,
        source: 1,
      }],
    },
    resources: [
      { id: 1, kind: { kind: "surface", slot: 1 } },
      { id: 2, kind: { kind: "surface", slot: 2 } },
      { id: 3, kind: { kind: "alias", source: 1, reason: "noOpGroup" } },
    ],
    passes: [
      { id: 1, kind: { kind: "clearRegion", output: 1 } },
      { id: 2, kind: { kind: "aliasResource", input: 1, output: 3, reason: "noOpGroup" } },
    ],
  };
}

describe("CanvasKit optimizer proof admission", () => {
  test("accepts an exact reason-coded final alias", () => {
    expect(() => admitOptimizationReport(optimizedPlan())).not.toThrow();
  });

  test("rejects an optimizer alias omitted from the proof", () => {
    const plan = optimizedPlan();
    plan.optimization.rewrites = [];
    plan.optimization.inputPhysicalHash = plan.optimization.outputPhysicalHash;
    expect(() => admitOptimizationReport(plan)).toThrow(CanvasKitCompositorError);
  });

  test("rejects a rewrite whose claimed physical owner differs from the pass", () => {
    const plan = optimizedPlan();
    plan.optimization.rewrites[0]!.source = 2;
    expect(() => admitOptimizationReport(plan)).toThrow("does not match the final plan");
  });
});
