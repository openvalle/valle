import { expect, test } from "bun:test";

import type { GoodMotionContext } from "./motion-preview.ts";
import { buildMotionWorkspaceModel } from "./motion-workspace.ts";

test("Motion workspace exposes editable source-range cue authoring", () => {
  const context = {
    input: "components/Card.tsx",
    generation: 7,
    artifactDigest: `sha256:${"a".repeat(64)}`,
    artifact: {
      formatVersion: 1,
      component: "Card",
      controls: {
        props: {
          intensity: {
            control: { kind: "number", min: 0, max: 1, step: 0.1 },
            default: { kind: "number", value: 0.5 },
          },
        },
        data: {},
        timing: { enterFrames: {}, holdCycleFrames: {}, exitFrames: {} },
        cues: {},
        assets: {},
        camera: { values: {} },
      },
    },
    preparedData: {},
    dataSource: null,
    durationFrames: 120,
    sourceMap: { version: 1, component: "Card", nodes: [], objects: [] },
    diagnostics: [],
  } as unknown as GoodMotionContext;
  const cues: GoodMotionContext["cueBindings"] = {
    beat: {
      type: "sourceRange",
      startFrame: 10,
      endFrame: 50,
      enterFrames: 4,
      exitFrames: 6,
    },
    title: {
      type: "sourceRange",
      startFrame: 60,
      endFrame: 90,
      enterFrames: 3,
      exitFrames: 5,
    },
  };

  const model = buildMotionWorkspaceModel(
    context,
    {},
    { enterFrames: 0, exitFrames: 0 },
    cues,
    true,
  );

  expect(model.cues.filter((cue) => cue.cue === "beat").map(({ key, readOnly }) => ({ key, readOnly }))).toEqual([
    { key: "startFrame", readOnly: false },
    { key: "endFrame", readOnly: false },
    { key: "enterFrames", readOnly: false },
    { key: "exitFrames", readOnly: false },
  ]);
  expect(model.cues.filter((cue) => cue.cue === "title").map(({ key, readOnly }) => ({ key, readOnly }))).toEqual([
    { key: "startFrame", readOnly: false },
    { key: "endFrame", readOnly: false },
    { key: "enterFrames", readOnly: false },
    { key: "exitFrames", readOnly: false },
  ]);
  expect(model.cues.find((cue) => cue.cue === "title" && cue.key === "enterFrames")).toMatchObject({
    value: 3,
    max: 25,
  });
  expect(model.cues.find((cue) => cue.cue === "beat" && cue.key === "enterFrames")).toMatchObject({
    value: 4,
    max: 34,
  });
  expect(model.props).toContainEqual({
    name: "intensity",
    kind: "number",
    value: 0.5,
    min: 0,
    max: 1,
    step: 0.1,
    values: undefined,
  });
});
