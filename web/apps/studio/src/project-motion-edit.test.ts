import { expect, test } from "bun:test";
import type { TimelineDocument } from "@valle/engine/internal";

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

test("canonical Rust frame projection overrides stale admitted Motion defaults", () => {
  const context = {
    status: "ok",
    timing: { enterFrames: 4, exitFrames: 5 },
    cueBindings: {
      beat: { type: "sourceRange", startFrame: 1, endFrame: 10, enterFrames: 0, exitFrames: 0 },
    },
    durationFrames: 60,
  } as never;
  const next = motionContextWithTimelineFrames(context, {
    sourceDurationFrames: 120,
    enterFrames: null,
    exitFrames: 8,
    cues: {
      beat: { type: "sourceRange", startFrame: 6, endFrame: 20, enterFrames: 2, exitFrames: 3 },
    },
  });
  expect(next.durationFrames).toBe(120);
  expect(next.timing).toEqual({ enterFrames: 4, exitFrames: 8 });
  expect(next.cueBindings.beat).toEqual({
    type: "sourceRange",
    startFrame: 6,
    endFrame: 20,
    enterFrames: 2,
    exitFrames: 3,
  });
});
