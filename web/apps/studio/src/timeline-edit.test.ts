import { expect, test } from "bun:test";
import type { Timeline } from "@valle/engine";

import {
  deleteTimelineClip,
  editTimelineClip,
  moveTimelineVisualClipBefore,
  setTimelineClipDurationFrames,
  timelineClipAddress,
  trimTimelineClipFrames,
} from "./timeline-edit.ts";

function timeline(): Timeline {
  return {
    canvas: { width: 640, height: 360, fps: "30000/1001" },
    tracks: {
      visual: [{
        clips: [
          { start: 0, duration: 1, kind: "solid", color: "#111111ff" },
          { start: 1.5, duration: 2, kind: "solid", color: "#222222ff" },
          { start: 4, duration: 1, kind: "solid", color: "#333333ff" },
        ],
      }],
    },
  } as unknown as Timeline;
}

test("compiler source pointers address Timeline clips without parsing internal ids", () => {
  expect(timelineClipAddress("/tracks/caption/3/clips/7")).toEqual({
    band: "caption",
    trackIndex: 3,
    clipIndex: 7,
  });
  expect(timelineClipAddress("author:track:3:clip:7")).toBeNull();
  expect(timelineClipAddress("/tracks/subtitle/3/clips/7")).toBeNull();
  const next = editTimelineClip(timeline(), "/tracks/visual/0/clips/1", (target) => {
    if (target.band !== "visual") throw new Error("expected visual clip");
    target.clip.opacity = 0.5;
  });
  expect(next.tracks.visual?.[0]?.clips[1]).toMatchObject({ opacity: 0.5 });
  expect(next).not.toHaveProperty("id");
});

test("frame edits delegate exact Timeline seconds to the Rust runtime", () => {
  const rustTime = (frames: number): number => {
    if (frames === 10) return 0.333667;
    if (frames === 1) return 0.033367;
    throw new Error(`unexpected frame fixture ${frames}`);
  };
  const duration = setTimelineClipDurationFrames(
    timeline(),
    "/tracks/visual/0/clips/0",
    10,
    rustTime,
  );
  expect(duration.tracks.visual?.[0]?.clips[0]?.duration).toBe(0.333667);

  const trimmed = trimTimelineClipFrames(
    duration,
    "/tracks/visual/0/clips/0",
    "start",
    1,
    rustTime,
    () => {
      throw new Error("solid clips have no source clock");
    },
  );
  expect(trimmed.tracks.visual?.[0]?.clips[0]).toMatchObject({
    start: 0.033367,
    duration: 0.3003,
  });
});

test("left trim applies exact source-rate deltas for audio, video, lottie, and Motion", () => {
  const sourceDeltaCalls: Array<{ frames: number; fps: number | string; rate: number | null | undefined }> = [];
  const sourceDelta = (
    frames: number,
    fps: number | string,
    rate?: number | null,
  ): number => {
    sourceDeltaCalls.push({ frames, fps, rate });
    return frames * (rate ?? 1);
  };

  for (const rate of [2, 0.5]) {
    for (const kind of ["audio", "video", "lottie", "motion"] as const) {
      const authored = kind === "audio"
        ? ({
            canvas: { width: 640, height: 360, fps: 1 },
            tracks: {
              audio: [{
                clips: [{ start: 1, duration: 4, src: "media", trimStart: 3, rate }],
              }],
            },
          } as Timeline)
        : ({
            canvas: { width: 640, height: 360, fps: 1 },
            tracks: {
              visual: [{
                clips: [{
                  start: 1,
                  duration: 4,
                  trimStart: 3,
                  rate,
                  ...(kind === "motion"
                    ? { kind, component: "component" }
                    : { kind, src: "media" }),
                }],
              }],
            },
          } as Timeline);
      const path = kind === "audio"
        ? "/tracks/audio/0/clips/0"
        : "/tracks/visual/0/clips/0";
      const trimmed = trimTimelineClipFrames(
        authored,
        path,
        "start",
        1,
        (frames) => frames,
        sourceDelta,
      );
      const clip = kind === "audio"
        ? trimmed.tracks.audio?.[0]?.clips[0]
        : trimmed.tracks.visual?.[0]?.clips[0];
      expect(clip).toMatchObject({
        start: 2,
        duration: 3,
        trimStart: 3 + rate,
        rate,
      });
    }
  }

  expect(sourceDeltaCalls).toEqual([
    { frames: 1, fps: 1, rate: 2 },
    { frames: 1, fps: 1, rate: 2 },
    { frames: 1, fps: 1, rate: 2 },
    { frames: 1, fps: 1, rate: 2 },
    { frames: 1, fps: 1, rate: 0.5 },
    { frames: 1, fps: 1, rate: 0.5 },
    { frames: 1, fps: 1, rate: 0.5 },
    { frames: 1, fps: 1, rate: 0.5 },
  ]);
});

test("delete and reorder operate on hard-typed arrays and preserve legal absolute starts", () => {
  const moved = moveTimelineVisualClipBefore(
    timeline(),
    "/tracks/visual/0/clips/2",
    "/tracks/visual/0/clips/0",
  );
  expect(moved.tracks.visual?.[0]?.clips.map((clip) => clip.kind === "solid" ? clip.color : null)).toEqual([
    "#333333ff",
    "#111111ff",
    "#222222ff",
  ]);
  expect(moved.tracks.visual?.[0]?.clips.map((clip: { start: number }) => clip.start)).toEqual([0, 1.5, 3]);

  const deleted = deleteTimelineClip(moved, "/tracks/visual/0/clips/1");
  expect(deleted.tracks.visual?.[0]?.clips).toHaveLength(2);
});
