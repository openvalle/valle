import { expect, test } from "bun:test";

import { hitAtDisplayPoint, topHitForClip } from "./timeline-selection.ts";

const hits = [
  { clipId: "back", rect: { x: 0, y: 0, w: 100, h: 100 } },
  { clipId: "front", rect: { x: 20, y: 20, w: 40, h: 40 } },
  { clipId: "front", rect: { x: 25, y: 25, w: 10, h: 10 } },
];

test("Timeline hit map resolves topmost display and clip hits", () => {
  expect(hitAtDisplayPoint(hits, 125, 75, { left: 100, top: 50, width: 100, height: 100 }, 100, 100))
    .toBe(hits[2]);
  expect(hitAtDisplayPoint(hits, 195, 145, { left: 100, top: 50, width: 100, height: 100 }, 100, 100))
    .toBe(hits[0]);
  expect(topHitForClip(hits, "front")).toBe(hits[2]);
  expect(topHitForClip(hits, "missing")).toBeNull();
});

test("Timeline hit map rejects a collapsed display surface", () => {
  expect(hitAtDisplayPoint(hits, 0, 0, { left: 0, top: 0, width: 0, height: 100 }, 100, 100)).toBeNull();
});
