import { expect, test } from "bun:test";
import type { TimelineDocumentView } from "@valle/player";
import type { TimelineDocument } from "@valle/engine/internal";

import { projectTimelineSequences } from "./timeline-sequence.ts";

function fixture(): TimelineDocument {
  return {
    document: {
      canvas: {
        width: 1920,
        height: 1080,
        fps: "30000/1001",
        duration: "10/1",
        sampleRate: 48_000,
        channelLayout: "stereo",
        colorSpace: "srgb",
      },
      background: { color: "#000000" },
      visual: {
        tracks: [{ id: "visual:main", items: [
          { type: "clip", id: "clip:a", duration: "3/2", source: { type: "solid", color: "#111111" }, layer: layer() },
          { type: "transition", id: "transition:ab", duration: "1/2", kernel: { type: "cross-fade" } },
          { type: "clip", id: "clip:b", duration: "5/2", source: { type: "solid", color: "#222222" }, layer: layer() },
          { type: "gap", id: "gap:tail", duration: "1/1" },
        ] }],
      },
      audio: { tracks: [{ id: "audio:main", items: [
        { type: "clip", id: "audio:a", duration: "2/1", source: { type: "media", resource: "asset:audio-a", sourceStart: "0/1", rate: "1/1", endBehavior: "hold" }, gain: { type: "constant", value: 1 }, pan: { type: "constant", value: 0 }, effects: [] },
        { type: "crossfade", id: "crossfade:ab", duration: "1/4" },
        { type: "clip", id: "audio:b", duration: "2/1", source: { type: "media", resource: "asset:audio-b", sourceStart: "0/1", rate: "1/1", endBehavior: "hold" }, gain: { type: "constant", value: 1 }, pan: { type: "constant", value: 0 }, effects: [] },
      ] }] },
      captions: { tracks: [{ id: "caption:main", items: [
        { type: "gap", id: "gap:caption-head", duration: "1/1" },
        { type: "clip", id: "caption:a", duration: "2/1", runs: [{ id: "run:a", text: "Hello", style: null, timing: null }], style: { font: "font:main", fontSize: 48, color: "#ffffff", shadow: null }, layout: { align: "bottom-center", region: [0, 0, 1, 1] }, behavior: null, presentation: { opacity: { type: "constant", value: 1 }, translation: { type: "constant", value: [0, 0] }, scale: { type: "constant", value: 1 }, rotation: { type: "constant", value: 0 }, blurSigma: { type: "constant", value: 0 }, clipInset: { type: "constant", value: [0, 0, 0, 0] } } },
      ] }] },
      adjustments: [{
        id: "adjustment:grade",
        start: "1/2",
        duration: "2/1",
        effect: { type: "color-grade", temperature: 0.2 },
      }, {
        id: "adjustment:synthetic",
        start: "3/1",
        duration: "1/1",
        effect: { type: "color-grade", temperature: -0.1 },
      }],
      camera: null,
      metadata: {},
    },
  } as TimelineDocument;
}

function layer() {
  return {
    transform: {
      position: { type: "constant" as const, value: [0, 0] as [number, number] },
      scale: { type: "constant" as const, value: [1, 1] as [number, number] },
      rotation: { type: "constant" as const, value: 0 },
      anchor: [0.5, 0.5] as [number, number],
    },
    opacity: { type: "constant" as const, value: 1 },
    blend: "normal" as const,
    mask: null,
    filters: [],
  };
}

function view(): TimelineDocumentView {
  const item = (
    itemId: string,
    timelinePath: string | null,
    itemIndex: number,
    startSeconds: number,
    durationSeconds: number,
    advancesCursor = true,
    sourceStartSeconds: number | null = null,
    sourceRate: number | null = null,
  ) => ({
    itemId,
    timelinePath,
    itemIndex,
    startSeconds,
    durationSeconds,
    endSeconds: advancesCursor ? startSeconds + durationSeconds : startSeconds,
    startFrame: Math.round(startSeconds * 30),
    durationFrames: Math.round(durationSeconds * 30),
    endFrame: Math.round((advancesCursor ? startSeconds + durationSeconds : startSeconds) * 30),
    advancesCursor,
    sourceStartSeconds,
    sourceStartFrame: sourceStartSeconds == null ? null : Math.round(sourceStartSeconds * 30),
    sourceRate,
  });
  return {
    canvas: {
      width: 1920,
      height: 1080,
      durationSeconds: 10,
      framesPerSecond: 30_000 / 1001,
      frameCount: 300,
      sampleRate: 48_000,
      sampleCount: 480_000,
    },
    sequences: [
      {
        band: "visual",
        trackId: "visual:main",
        trackIndex: 0,
        durationSeconds: 5,
        items: [
          item("clip:a", "/tracks/visual/0/clips/0", 0, 0, 1.5),
          item("transition:ab", null, 1, 1.5, 0.5, false),
          item("clip:b", "/tracks/visual/0/clips/1", 2, 1.5, 2.5),
          item("gap:tail", null, 3, 4, 1),
        ],
      },
      {
        band: "audio",
        trackId: "audio:main",
        trackIndex: 0,
        durationSeconds: 4,
        items: [
          item("audio:a", "/tracks/audio/0/clips/0", 0, 0, 2, true, 0, 1),
          item("crossfade:ab", null, 1, 2, 0.25, false),
          item("audio:b", "/tracks/audio/0/clips/1", 2, 2, 2, true, 0, 1),
        ],
      },
      {
        band: "caption",
        trackId: "caption:main",
        trackIndex: 0,
        durationSeconds: 3,
        items: [
          item("gap:caption-head", null, 0, 0, 1),
          item("caption:a", "/tracks/caption/0/clips/0", 1, 1, 2),
        ],
      },
      {
        band: "adjustment",
        trackId: "timeline:adjustment-track:0",
        trackIndex: 0,
        durationSeconds: 2.5,
        items: [{
          ...item(
            "adjustment:grade",
            "/tracks/adjustment/0/clips/0",
            0,
            0.5,
            2,
            false,
          ),
          endSeconds: 2.5,
          endFrame: 75,
        }],
      },
      {
        band: "adjustment",
        trackId: "canonical:adjustments",
        trackIndex: 1,
        durationSeconds: 4,
        items: [{
          ...item("adjustment:synthetic", null, 1, 3, 1, false),
          endSeconds: 4,
          endFrame: 120,
        }],
      },
    ],
  };
}

test("Sequence projection derives starts and keeps transition/crossfade cursor-neutral", () => {
  const tracks = projectTimelineSequences(fixture(), view());
  const visual = tracks.find((track) => track.band === "visual")!;
  expect(visual.items.map((entry) => [entry.item.id, entry.startSeconds, entry.advancesCursor])).toEqual([
    ["clip:a", 0, true],
    ["transition:ab", 1.5, false],
    ["clip:b", 1.5, true],
    ["gap:tail", 4, true],
  ]);
  expect(visual.items[1]).toMatchObject({ endSeconds: 1.5, durationSeconds: 0.5 });
  expect(visual.durationSeconds).toBe(5);
  const audio = tracks.find((track) => track.band === "audio")!;
  expect(audio.items[1]).toMatchObject({ startSeconds: 2, endSeconds: 2, advancesCursor: false });
  expect(audio.items[2]).toMatchObject({ startSeconds: 2, sourceStartSeconds: 0, sourceRate: 1 });
  expect(visual.items.map((entry) => entry.timelinePath)).toEqual([
    "/tracks/visual/0/clips/0",
    null,
    "/tracks/visual/0/clips/1",
    null,
  ]);
  const adjustmentTracks = tracks.filter((track) => track.band === "adjustment");
  expect(adjustmentTracks.map((track) => [track.id, track.index])).toEqual([
    ["timeline:adjustment-track:0", 0],
    ["canonical:adjustments", 1],
  ]);
  const adjustment = adjustmentTracks[0]!;
  expect(adjustment.items[0]).toMatchObject({
    timelinePath: "/tracks/adjustment/0/clips/0",
    item: { id: "adjustment:grade", effect: { type: "color-grade" } },
  });
  expect(adjustmentTracks[1]?.items[0]).toMatchObject({
    timelinePath: null,
    item: { id: "adjustment:synthetic" },
  });
});
