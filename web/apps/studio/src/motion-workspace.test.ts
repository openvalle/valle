import { expect, test } from "bun:test";
import type { GoodMotionContext } from "./motion-preview.ts";
import { buildMotionWorkspaceModel } from "./motion-workspace.ts";

test("Motion workspace exposes general props and structured data without timing controls", () => {
  const context = {
    input: "Card.motion.tsx", generation: 7, artifactDigest: `sha256:${"a".repeat(64)}`,
    artifact: { controls: {
      props: { intensity: { control: { kind: "number", min: 0, max: 1, step: 0.1 }, default: { kind: "number", value: 0.5 } } },
      data: { rows: { kind: "array", maxItems: 4 } }, assets: {},
    } },
    preparedData: { rows: [{ id: "first" }] }, dataSource: "timeline:clip",
    sourceMap: { nodes: [], objects: [] }, diagnostics: [],
  } as unknown as GoodMotionContext;
  const model = buildMotionWorkspaceModel(context, {}, { rows: [{ id: "second" }] }, true);
  expect(model.props[0]).toMatchObject({ name: "intensity", kind: "number", value: 0.5, min: 0, max: 1, step: 0.1 });
  expect(model.dataJson).toBe(JSON.stringify({ rows: [{ id: "second" }] }, null, 2));
  expect(model.data[0]).toMatchObject({ name: "rows", maxItems: 4, value: [{ id: "second" }] });
  expect(model.canReturn).toBe(true);
});
