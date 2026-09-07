import { expect, test } from "bun:test";

import { initialStudioShellState, reduceStudioIntent } from "./shell-state.ts";

test("Studio shell owns selection, dirty/conflict and workspace return state", () => {
  const selected = reduceStudioIntent(initialStudioShellState, { type: "select", clipId: "c1" });
  const dirty = reduceStudioIntent(selected, { type: "dirty", value: true });
  const conflict = reduceStudioIntent(dirty, { type: "conflict", value: true });
  const motion = reduceStudioIntent(conflict, {
    type: "workspace",
    workspace: { kind: "motion", clipId: "c1", source: "component://Card" },
    returnPoint: {
      timeS: 1.25,
      playing: false,
      selectedClipId: "c1",
      scrollLeft: 320,
      zoom: 84,
      dirty: true,
    },
  });

  expect(motion).toMatchObject({
    selectedClipId: "c1",
    dirty: true,
    conflict: true,
    workspace: { kind: "motion", clipId: "c1" },
    returnPoint: { timeS: 1.25, scrollLeft: 320, zoom: 84 },
  });
  const returned = reduceStudioIntent(motion, {
    type: "workspace",
    workspace: { kind: "timeline" },
  });
  expect(returned).toMatchObject({ workspace: { kind: "timeline" }, returnPoint: null });
  expect(initialStudioShellState).toMatchObject({
    selectedClipId: null,
    dirty: false,
    conflict: false,
    workspace: { kind: "timeline" },
  });
});
