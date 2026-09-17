import { expect, test } from "bun:test";

import type { GoodMotionContext } from "./motion-preview.ts";
import { buildMotionWorkspaceModel, syncMotionDraftFrames } from "./motion-workspace.ts";

test("preview refreshes cue phase frames and retains authored cue seconds", () => {
  const timing = { enterFrames: 4, exitFrames: 4, enterDuration: 0.2, exitDuration: 0.15 };
  const cues = { beat: {
    type: "sourceRange" as const, startFrame: 0, endFrame: 24,
    enterFrames: 6, exitFrames: 6,
    start: 0, end: 1, enterDuration: 1, exitDuration: 1,
  } };
  const response = {
    timing: { enterFrames: 5, exitFrames: 3, enterDuration: 0.21, exitDuration: 0.125 },
    cueBindings: { beat: {
      type: "sourceRange", startFrame: 0, endFrame: 24,
      enterFrames: 12, exitFrames: 12,
      start: 0, end: 1, enterDuration: 0.5, exitDuration: 0.5,
    } },
  } as unknown as GoodMotionContext;
  const synced = syncMotionDraftFrames(timing, cues, response);
  expect(synced.timing).toEqual({ enterFrames: 5, exitFrames: 3, enterDuration: 0.2, exitDuration: 0.15 });
  expect(synced.cues.beat).toMatchObject({
    startFrame: 0, endFrame: 24, enterFrames: 12, exitFrames: 12,
    start: 0, end: 1, enterDuration: 1, exitDuration: 1,
  });
});

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
    max: Number.MAX_SAFE_INTEGER,
  });
  expect(model.cues.find((cue) => cue.cue === "beat" && cue.key === "enterFrames")).toMatchObject({
    value: 4,
    max: Number.MAX_SAFE_INTEGER,
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

test("compressed cue phases do not make its edit handles impossible", () => {
  const context = {
    durationFrames: 24,
    artifact: { controls: {
      props: {}, data: {}, cues: {}, assets: {}, camera: { values: {} },
      timing: { enterFrames: {}, holdCycleFrames: {}, exitFrames: {} },
    } },
    sourceMap: { nodes: [], objects: [] },
    diagnostics: [],
  } as unknown as GoodMotionContext;
  const cues = {
    short: { type: "sourceRange", startFrame: 0, endFrame: 12, enterFrames: 6, exitFrames: 6 },
  } as GoodMotionContext["cueBindings"];
  const handles = buildMotionWorkspaceModel(context, {}, { enterFrames: 0, exitFrames: 0 }, cues, true).cues;
  expect(handles.find((handle) => handle.key === "endFrame")).toMatchObject({ value: 12, min: 1, max: 24 });
  expect(handles.find((handle) => handle.key === "startFrame")).toMatchObject({ value: 0, max: 11 });
  expect(handles.find((handle) => handle.key === "enterFrames")).toMatchObject({ value: 6, max: Number.MAX_SAFE_INTEGER });
  expect(handles.find((handle) => handle.key === "exitFrames")).toMatchObject({ value: 6, max: Number.MAX_SAFE_INTEGER });
  const phases = buildMotionWorkspaceModel(context, {}, { enterFrames: 24, exitFrames: 24 }, cues, true).phases;
  expect(phases).toEqual([
    { key: "enterFrames", label: "enter", value: 24, min: 0, max: Number.MAX_SAFE_INTEGER },
    { key: "exitFrames", label: "exit", value: 24, min: 0, max: Number.MAX_SAFE_INTEGER },
  ]);
});
