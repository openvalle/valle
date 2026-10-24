import { expect, test } from "bun:test";
import { RenderTrace } from "./render-trace.ts";

test("trace capacity is bounded and reused planner results do not duplicate work", () => {
  const trace = new RenderTrace(3);
  const origin = performance.timeOrigin,
    start = performance.now();
  trace.worker("prepare.evaluate", origin, start, 1, 7);
  trace.worker("prepare.evaluate", origin, start, 1, 7);
  trace.span("prepare.wait", { frame: 7 })();
  trace.evidence("cache", { bytes: 128 });
  trace.span("execute.record", { frame: 7 })();
  const value = trace.snapshot();
  expect(value.traceEvents).toHaveLength(3);
  expect(value.metadata.droppedEvents).toBe(1);
  expect(value.traceEvents[0]!.tid).toBe(2);
  expect(value.traceEvents[1]!.tid).toBe(1);
  expect(Number(value.traceEvents[0]!.ts)).toBeGreaterThanOrEqual(0);
});
