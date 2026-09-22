import { expect, test } from "bun:test";
import type { TimelineDocument } from "valle-engine/internal";

import {
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
