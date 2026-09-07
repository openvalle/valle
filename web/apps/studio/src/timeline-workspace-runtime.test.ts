import { expect, test } from "bun:test";

import type { Timeline } from "@valle/engine";
import type { CanonicalTimelineDocument } from "@valle/player-core";
import type { TimelineDocument } from "@valle/engine/internal";

import {
  initializeTimelineWorkspaceRuntime,
  type TimelineWorkspaceRuntimeConfig,
} from "./timeline-workspace-runtime.ts";

const timeline = {
  document: { canvas: { width: 640, height: 360 } },
} as unknown as TimelineDocument;
const authored = {
  canvas: { width: 640, height: 360, fps: 30 },
  tracks: {},
} as Timeline;
const resourceManifest = {
  entries: {},
};

const config: TimelineWorkspaceRuntimeConfig = {
  timelineJson: JSON.stringify(authored),
  timeline: authored,
  render: {
    timelineJson: JSON.stringify(timeline),
    timeline,
  },
  runtimeAssets: {
    engine: { glue: "engine.js", wasm: "engine.wasm" },
    canvasKit: {
      base: { glue: "canvaskit.js", wasm: "canvaskit.wasm" },
      full: { glue: "canvaskit-full.js", wasm: "canvaskit-full.wasm" },
    },
    fonts: { defaultSans: "default-sans.ttf" },
    workers: { productFrame: "product-frame.js" },
  },
  runtimeBaseUrl: "http://studio.test/",
  preview: {
    status: "unavailable",
    code: "verified_binding_bundle_unavailable",
  },
};

const canonical: CanonicalTimelineDocument = {
  timeline,
  timelineJson: JSON.stringify(timeline),
  view: {
    canvas: {
      width: 640,
      height: 360,
      durationSeconds: 1,
      framesPerSecond: 30,
      frameCount: 30,
      sampleRate: 48_000,
      sampleCount: 48_000,
    },
    sequences: [],
  },
};

test("Timeline compiler boots without fixed fulfillment and does not open preview", async () => {
  let previewLoads = 0;
  let compilerLoads = 0;
  const runtime = await initializeTimelineWorkspaceRuntime(
    config,
    {
      load: async () => {
        previewLoads += 1;
        throw new Error("preview must stay fail-closed");
      },
    },
    async () => {
      compilerLoads += 1;
      return {
        normalizeTimeline: (value) => value,
        timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
        timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
          frames / Number(fps) * (rate ?? 1)
        ),
        compileTimeline: () => canonical,
        canonicalizeTimelineDocument: () => canonical,
      };
    },
  );

  expect(previewLoads).toBe(0);
  expect(compilerLoads).toBe(1);
  expect(runtime.previewAvailable).toBe(false);
  expect(runtime.rendererMessage).toContain("verified binding fulfillment");
  expect(runtime.compiledTimeline).toEqual(canonical);
  expect(runtime.normalizeTimeline(authored)).toEqual(authored);
  expect(runtime.compileTimeline(authored)).toEqual(canonical);
});

test("Timeline compiler rejects a hosted render projection from another working copy", async () => {
  await expect(initializeTimelineWorkspaceRuntime(
    config,
    {
      load: async () => undefined,
    },
    async () => ({
      normalizeTimeline: (value) => value,
      timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
      timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
        frames / Number(fps) * (rate ?? 1)
      ),
      compileTimeline: () => canonical,
      canonicalizeTimelineDocument: () => ({ ...canonical, timelineJson: "different" }),
    }),
  )).rejects.toThrow("render projection does not match");
});

test("Timeline preview receives the exact fixed-package bytes", async () => {
  const previewConfig = {
    ...config,
    render: {
      ...config.render,
      fixedPackageManifestJson: "{\"format\":\"valle.fixed-render-package@1\"}",
      resourceManifestJson: JSON.stringify(resourceManifest),
      resourceManifest,
      verifiedBindingBundleJson: "{\"bindings\":{}}",
    },
  } as unknown as TimelineWorkspaceRuntimeConfig;
  const loads: Record<string, unknown>[] = [];
  const runtime = await initializeTimelineWorkspaceRuntime(
    previewConfig,
    { load: async (options) => { loads.push(options as unknown as Record<string, unknown>); } },
    async () => ({
      normalizeTimeline: (value) => value,
      timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
      timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
        frames / Number(fps) * (rate ?? 1)
      ),
      compileTimeline: () => canonical,
      canonicalizeTimelineDocument: () => canonical,
    }),
  );

  expect(runtime.previewAvailable).toBe(true);
  expect(loads[0]?.fixedPackageManifestJson).toBe(
    "{\"format\":\"valle.fixed-render-package@1\"}",
  );
  expect(loads[0]).not.toHaveProperty("resourceManifest");
  expect(loads[0]).not.toHaveProperty("timelineRevision");
});

test("Timeline preview rejects partial fixed render fulfillment", async () => {
  const partial = {
    ...config,
    render: {
      ...config.render,
      resourceManifestJson: "{}",
    },
  } as unknown as TimelineWorkspaceRuntimeConfig;
  await expect(initializeTimelineWorkspaceRuntime(
    partial,
    { load: async () => undefined },
    async () => ({
      normalizeTimeline: (value) => value,
      timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
      timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
        frames / Number(fps) * (rate ?? 1)
      ),
      compileTimeline: () => canonical,
      canonicalizeTimelineDocument: () => canonical,
    }),
  )).rejects.toThrow("complete fixed package manifest and members together");
});

test("Timeline preview rejects an empty fixed render fulfillment", async () => {
  const emptyBundle = {
    ...config,
    render: {
      ...config.render,
      fixedPackageManifestJson: "{}",
      resourceManifestJson: "{}",
      resourceManifest,
      verifiedBindingBundleJson: "",
    },
  } as unknown as TimelineWorkspaceRuntimeConfig;
  await expect(initializeTimelineWorkspaceRuntime(
    emptyBundle,
    { load: async () => undefined },
    async () => ({
      normalizeTimeline: (value) => value,
      timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
      timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
        frames / Number(fps) * (rate ?? 1)
      ),
      compileTimeline: () => canonical,
      canonicalizeTimelineDocument: () => canonical,
    }),
  )).rejects.toThrow("fixed package strings must not be empty");
});
