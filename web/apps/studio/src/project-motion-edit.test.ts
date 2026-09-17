import { expect, test } from "bun:test";
import type { TimelineDocument } from "valle-engine/internal";

import {
  motionContextWithTimelineFrames,
  preparedMotionProps,
} from "./project-motion-edit.ts";

test("prepared Motion props combine canonical overrides with admitted defaults", () => {
  const timeline = {
    document: {
      visual: { tracks: [{ id: "visual:main", items: [{
        type: "clip",
        id: "hero",
        source: {
          type: "motion",
          props: { amount: { type: "constant", value: 2 } },
        },
      }] }] },
    },
  } as unknown as TimelineDocument;
  const motion = {
    structures: [{
      clipId: "hero",
      artifact: { controls: { props: {} } },
      authoring: { props: { amount: { kind: "number", value: 1 } } },
    }],
  };
  const verifiedArtifact = { controls: { props: {
    amount: { control: { kind: "number" }, default: { kind: "number", value: 1 } },
  } } } as never;
  expect(preparedMotionProps(timeline, motion, "hero", verifiedArtifact)).toEqual({
    amount: { kind: "number", value: 2 },
  });
});

test("prepared phase layout wins over individually projected duration frames", () => {
  const context = {
    status: "ok",
    timing: { enterFrames: 0, exitFrames: 3 },
    cueBindings: {
      beat: { type: "sourceRange", startFrame: 1, endFrame: 10, enterFrames: 0, exitFrames: 0 },
    },
    durationFrames: 110,
  } as never;
  const next = motionContextWithTimelineFrames(context, {
    sourceDurationFrames: 110,
    enterFrames: null,
    exitFrames: 4,
    exitDuration: 0.15,
    cues: {
      beat: { type: "sourceRange", startFrame: 6, endFrame: 20, enterFrames: 2, exitFrames: 3 },
    },
  });
  expect(next.durationFrames).toBe(110);
  expect(next.timing).toEqual({ enterFrames: 0, exitFrames: 3, enterDuration: undefined, exitDuration: 0.15 });
  expect(next.cueBindings.beat).toEqual({
    type: "sourceRange",
    startFrame: 6,
    endFrame: 20,
    enterFrames: 2,
    exitFrames: 3,
  });
});
