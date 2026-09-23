import { expect, test } from "bun:test";
import type { Timeline } from "./timeline.ts";
import type { TimelineDocument } from "./internal-timeline.ts";

import {
  canonicalizeTimelineDocumentWithWasm,
  compileMotionJsxWithWasm,
  compileMotionModulesWithWasm,
  compileTimelineWithWasm,
  MotionCompileError,
  normalizeTimelineWithWasm,
  timelineSourceTimeDeltaFromFramesWithWasm,
  timelineTimeFromFramesWithWasm,
} from "./compiler.ts";

test("Motion compiler forwards explicit bytes and preserves structured diagnostics", () => {
  const font = new Uint8Array([1, 2, 3]);
  const shader = { frozenBytes: new Uint8Array([4]) };
  const options = {
    resources: [{ control: "logo", contentHash: `sha256:${"a".repeat(64)}` }],
    data: { source: "fixture", value: { count: 2 } },
    fonts: [font],
    fontAliases: { "asset://brand": font },
    shaders: [shader],
  };
  const compiled = compileMotionJsxWithWasm({
    compile_motion_jsx: (source, optionsJson, fonts, aliases, shaders) => {
      expect(source).toBe("source");
      expect(JSON.parse(optionsJson)).toEqual({ resources: options.resources, data: options.data });
      expect(fonts).toEqual([font]);
      expect(aliases).toEqual([["asset://brand", font]]);
      expect(shaders).toEqual([shader]);
      return JSON.stringify({
        status: "ok", artifact: { component: "Card" }, artifactDigest: "sha256:artifact", sourceMap: { entry: "Card.tsx" },
        normalizedSource: "source", normalizedAstDigest: "sha256:one", preparedDataDigest: "sha256:two",
      });
    },
  }, "source", options);
  expect(compiled.artifact.component).toBe("Card");
  expect(compiled.artifactDigest).toBe("sha256:artifact");

  const diagnostic = {
    class: "error", code: "syntaxError", span: { start: 0, end: 1, line: 1, column: 1 },
    sourcePath: "card.motion.tsx", message: "invalid Motion source",
  };
  expect(() => compileMotionModulesWithWasm({
    compile_motion_modules: (entry, modulesJson) => {
      expect(entry).toBe("card.motion.tsx");
      expect(JSON.parse(modulesJson)).toEqual({ "card.motion.tsx": "bad" });
      return JSON.stringify({ status: "error", diagnostics: [diagnostic] });
    },
  }, "card.motion.tsx", { "card.motion.tsx": "bad" })).toThrow(MotionCompileError);
});

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
