import { expect, test } from "bun:test";

import type { Timeline } from "@valle/engine";
import type { StudioTimelineRevision } from "./host.ts";
import {
  assertReloadedTimelineSave,
  captureTimelineSaveSnapshot,
  type ReloadedTimelineSnapshot,
} from "./timeline-save-snapshot.ts";

function document(background: string): Timeline {
  return {
    canvas: { width: 640, height: 360, fps: 30, background },
    tracks: {},
  } as Timeline;
}

function revision(overrides: Record<string, unknown> = {}): StudioTimelineRevision {
  return {
    revision: 8,
    parentRevision: 7,
    actor: "studio",
    createdAt: "2026-08-30T00:00:00.000Z",
    intent: "studio save",
    cause: { type: "timelineEdit" },
    ...overrides,
  } as StudioTimelineRevision;
}

function reloaded(overrides: Partial<ReloadedTimelineSnapshot> = {}): ReloadedTimelineSnapshot {
  const timeline = document("#14532dff");
  return {
    timelineRevision: revision(),
    timeline,
    timelineJson: JSON.stringify(timeline),
    ...overrides,
  };
}

const response = {
  outcome: "unchanged" as const,
  revision: 8,
};

test("save captures a detached complete Timeline", () => {
  const workingCopy = document("#14532dff");
  const snapshot = captureTimelineSaveSnapshot(workingCopy);
  workingCopy.canvas.background = "#000000ff";

  expect(snapshot.timeline).toEqual(document("#14532dff"));
  expect(snapshot.timelineJson).toBe(JSON.stringify(document("#14532dff")));
});

test("save accepts the exact sparse Timeline reloaded from the accepted revision", () => {
  const submitted = captureTimelineSaveSnapshot(document("#14532dff"));
  expect(assertReloadedTimelineSave(response, reloaded(), submitted)).toEqual(submitted.timeline);
});

test("save rejects revision and complete Timeline drift", () => {
  const submitted = captureTimelineSaveSnapshot(document("#14532dff"));
  expect(() => assertReloadedTimelineSave(
    response,
    reloaded({ timelineRevision: revision({ revision: 9 }) }),
    submitted,
  )).toThrow("Timeline revision drift");
  const other = document("#000000ff");
  expect(() => assertReloadedTimelineSave(
    response,
    reloaded({ timeline: other, timelineJson: JSON.stringify(other) }),
    submitted,
  )).toThrow("reloaded complete Timeline drift");
});

test("save rejects timelineJson that disagrees with the reloaded Timeline", () => {
  const submitted = captureTimelineSaveSnapshot(document("#14532dff"));
  expect(() => assertReloadedTimelineSave(
    response,
    reloaded({ timelineJson: JSON.stringify(document("#000000ff")) }),
    submitted,
  )).toThrow("reloaded timelineJson drift");
});
