import { expect, test } from "bun:test";

import { initialStudioShellState, reduceStudioIntent } from "./shell-state.ts";

test("Studio shell owns selection and dirty/conflict state", () => {
  const selected = reduceStudioIntent(initialStudioShellState, { type: "select", clipId: "c1" });
  const dirty = reduceStudioIntent(selected, { type: "dirty", value: true });
  const conflict = reduceStudioIntent(dirty, {
    type: "conflict",
    value: true,
    message: "external change",
  });
  expect(conflict).toMatchObject({
    selectedClipId: "c1",
    dirty: true,
    conflict: true,
    conflictMessage: "external change",
  });
  expect(initialStudioShellState).toMatchObject({
    selectedClipId: null,
    dirty: false,
    conflict: false,
    previewStatus: "loading",
    inspectorVisible: true,
  });
});

test("preview status and history flags stay independent from save dirty", () => {
  const updating = reduceStudioIntent(initialStudioShellState, {
    type: "preview-status",
    status: "updating",
    message: "compiling",
  });
  const history = reduceStudioIntent(updating, {
    type: "history",
    canUndo: true,
    canRedo: false,
  });
  expect(history).toMatchObject({
    previewStatus: "updating",
    previewMessage: "compiling",
    canUndo: true,
    canRedo: false,
    dirty: false,
  });
});
