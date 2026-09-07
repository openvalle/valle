import { expect, test } from "bun:test";
import type { Timeline } from "@valle/engine";
import type { TimelineDocument } from "@valle/engine/internal";

import {
  canonicalizeTimelineDocumentWithWasm,
  compileTimelineWithWasm,
  normalizeTimelineWithWasm,
  timelineSourceTimeDeltaFromFramesWithWasm,
  timelineTimeFromFramesWithWasm,
} from "./timeline-compiler.ts";

const timeline = {
  document: { metadata: {} },
} as unknown as TimelineDocument;

const authored = {
  canvas: { width: 640, height: 360, fps: 30 },
  tracks: {},
} as Timeline;

const view = {
  canvas: {
    width: 640,
    height: 360,
    durationSeconds: 2,
    framesPerSecond: 30,
    frameCount: 60,
    sampleRate: 48_000,
    sampleCount: 96_000,
  },
  sequences: [],
};

test("sparse Timeline compiles through the read-only Rust projection", () => {
  let received = "";
  const canonical = compileTimelineWithWasm({
    compile_timeline: (timelineJson) => {
      received = timelineJson;
      return JSON.stringify(timeline);
    },
    timeline_document_view: () => JSON.stringify(view),
  }, authored);

  expect(JSON.parse(received)).toEqual(authored);
  expect(canonical).toEqual({
    timeline,
    timelineJson: JSON.stringify(timeline),
    view,
  });
});

test("renderer canonicalization remains read-only", () => {
  let received = "";
  const canonical = canonicalizeTimelineDocumentWithWasm({
    canonicalize_timeline_document: (timelineJson) => {
      received = timelineJson;
      return timelineJson;
    },
    timeline_document_view: () => JSON.stringify(view),
  }, timeline);

  expect(JSON.parse(received)).toEqual(timeline);
  expect(canonical).toEqual({ timeline, timelineJson: JSON.stringify(timeline), view });
});

test("sparse normalization and frame conversion delegate to Rust/WASM", () => {
  let normalizedInput = "";
  const normalized = normalizeTimelineWithWasm({
    normalize_timeline: (timelineJson) => {
      normalizedInput = timelineJson;
      return JSON.stringify({ ...authored, resources: { main: "https://example.test/main.mp4" } });
    },
  }, authored);
  expect(JSON.parse(normalizedInput)).toEqual(authored);
  expect(normalized.resources?.main).toBe("https://example.test/main.mp4");

  let receivedFrames = -1;
  let receivedFpsJson = "";
  const seconds = timelineTimeFromFramesWithWasm({
    timeline_time_from_frames: (frames, fpsJson) => {
      receivedFrames = frames;
      receivedFpsJson = fpsJson;
      return 0.033367;
    },
  }, 1, "30000/1001");
  expect(receivedFrames).toBe(1);
  expect(receivedFpsJson).toBe('"30000/1001"');
  expect(seconds).toBe(0.033367);

  let receivedRateJson = "";
  const sourceSeconds = timelineSourceTimeDeltaFromFramesWithWasm({
    timeline_source_time_delta_from_frames: (frames, fpsJson, rateJson) => {
      receivedFrames = frames;
      receivedFpsJson = fpsJson;
      receivedRateJson = rateJson;
      return 0.066733;
    },
  }, 1, "30000/1001", 2);
  expect(receivedFrames).toBe(1);
  expect(receivedFpsJson).toBe('"30000/1001"');
  expect(receivedRateJson).toBe("2");
  expect(sourceSeconds).toBe(0.066733);
});
