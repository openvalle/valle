import { expect, test } from "bun:test";
import type { Timeline } from "valle-engine";

import {
  deleteTimelineClip,
  editTimelineClip,
  moveTimelineVisualClipBefore,
  setTimelineClipDurationFrames,
  setTimelineMotionData,
  setTimelineMotionResource,
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

test("Motion data edit copies the prepared object into one author clip", () => {
  const original = timeline();
  original.tracks.visual![0]!.clips[1] = {
    kind: "motion", component: "chart", start: 1.5, duration: 2,
    data: { rows: [{ id: "old", value: 3 }] },
  } as never;
  const rows = [{ id: "new", value: 8 }];
  const updated = setTimelineMotionData(original, "/tracks/visual/0/clips/1", { rows });
  rows[0]!.value = 99;
  expect(updated.tracks.visual![0]!.clips[1]).toMatchObject({ data: { rows: [{ id: "new", value: 8 }] } });
  expect(original.tracks.visual![0]!.clips[1]).toMatchObject({ data: { rows: [{ id: "old", value: 3 }] } });
  expect(() => setTimelineMotionData(original, "/tracks/visual/0/clips/0", { rows })).toThrow("not Motion");
});

test("Motion resource binding edits one clip and requires a declared alias", () => {
  const original = timeline();
  original.resources = { first: "one.png", second: "two.png" };
  original.tracks.visual![0]!.clips[1] = {
    kind: "motion", component: "chart", start: 1.5, duration: 2,
    resources: { poster: "first" },
  } as never;
  const updated = setTimelineMotionResource(original, "/tracks/visual/0/clips/1", "poster", "second");
  expect(updated.tracks.visual![0]!.clips[1]).toMatchObject({ resources: { poster: "second" } });
  expect(original.tracks.visual![0]!.clips[1]).toMatchObject({ resources: { poster: "first" } });
  expect(() => setTimelineMotionResource(original, "/tracks/visual/0/clips/1", "poster", "missing"))
    .toThrow("does not exist");
});

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

test("transition edits ripple following starts while preserving source clocks and durations", async () => {
  const { setTimelineTransition, transitionLabels } = await import("./timeline-edit.ts");
  const source = timeline();
  for (const kind of Object.keys(transitionLabels) as Array<keyof typeof transitionLabels>) {
    const next = setTimelineTransition(source, "/tracks/visual/0/clips/0", kind, 0.25);
    const track = next.tracks.visual![0]!;
    expect(track.transitions).toEqual([{ from: 0, to: 1, kind }]);
    expect(track.clips.map(c => [c.start, c.duration])).toEqual([[0,1],[0.75,2],[3.25,1]]);
    const removed = setTimelineTransition(next, "/tracks/visual/0/clips/0", null);
    expect(removed.tracks.visual![0]!.clips.map(c => c.start)).toEqual([0,1,3.5]);
    expect(removed.tracks.visual![0]!.transitions).toEqual([]);
  }
  expect(source.tracks.visual![0]!.clips[1]!.start).toBe(1.5);
  for (const duration of [0,1,NaN,-1]) {
    expect(() => setTimelineTransition(source, "/tracks/visual/0/clips/0", "fade", duration)).toThrow("duration");
  }
  expect(() => setTimelineTransition(source, "/tracks/visual/0/clips/2", "fade")).toThrow("following clip");
});

test("deleting and reordering visual clips keep only adjacent transition endpoints", async () => {
  const { setTimelineTransition } = await import("./timeline-edit.ts");
  const source = setTimelineTransition(timeline(), "/tracks/visual/0/clips/1", "circleOpen", 0.25);
  const deletedHead = deleteTimelineClip(source, "/tracks/visual/0/clips/0");
  expect(deletedHead.tracks.visual![0]!.transitions).toEqual([{from:0,to:1,kind:"circleOpen"}]);
  const deletedEndpoint = deleteTimelineClip(source, "/tracks/visual/0/clips/1");
  expect(deletedEndpoint.tracks.visual![0]!.transitions).toEqual([]);
  const reordered = moveTimelineVisualClipBefore(source, "/tracks/visual/0/clips/2", "/tracks/visual/0/clips/1");
  expect(reordered.tracks.visual![0]!.transitions).toEqual([]);
  const preserved = moveTimelineVisualClipBefore(source, "/tracks/visual/0/clips/0", "/tracks/visual/0/clips/1");
  expect(preserved.tracks.visual![0]!.transitions).toEqual([{from:1,to:2,kind:"circleOpen"}]);
  const clips = preserved.tracks.visual![0]!.clips;
  expect(clips[1]!.start+clips[1]!.duration-clips[2]!.start).toBe(0.25);
});

test("transition parameters survive duration changes, reorder and deletion; kind changes reset them", async () => {
  const { setTimelineTransition, setTimelineTransitionParameter, transitionParameterSpecs } = await import("./timeline-edit.ts");
  const four = timeline();
  four.tracks.visual![0]!.clips.push({kind:"solid",color:"#444444",start:6,duration:1});
  const source = setTimelineTransition(four, "/tracks/visual/0/clips/1", "circleOpen", 0.25);
  const edited = setTimelineTransitionParameter(source, "/tracks/visual/0/clips/1", "centerX", 0.2);
  expect(source.tracks.visual![0]!.transitions![0]!.params).toBeUndefined();
  const resized = setTimelineTransition(edited, "/tracks/visual/0/clips/1", "circleOpen", 0.4);
  expect(resized.tracks.visual![0]!.transitions![0]!.params).toEqual({centerX:0.2});
  const deletedHead = deleteTimelineClip(resized, "/tracks/visual/0/clips/0");
  expect(deletedHead.tracks.visual![0]!.transitions![0]).toEqual({from:0,to:1,kind:"circleOpen",params:{centerX:0.2}});
  const preserved = moveTimelineVisualClipBefore(resized, "/tracks/visual/0/clips/3", "/tracks/visual/0/clips/0");
  expect(preserved.tracks.visual![0]!.transitions![0]!.params).toEqual({centerX:0.2});
  expect(setTimelineTransition(resized, "/tracks/visual/0/clips/1", "ripple").tracks.visual![0]!.transitions![0]!.params).toBeUndefined();
  expect(() => setTimelineTransitionParameter(edited, "/tracks/visual/0/clips/1", "centerX", 2)).toThrow();
  expect(() => setTimelineTransitionParameter(edited, "/tracks/visual/0/clips/1", "frequency", 4)).toThrow();
  const zoom = setTimelineTransition(timeline(), "/tracks/visual/0/clips/0", "simpleZoom", 0.25);
  expect(setTimelineTransitionParameter(zoom, "/tracks/visual/0/clips/0", "quickness", 0.2).tracks.visual![0]!.transitions![0]!.params).toEqual({quickness:0.2});
  const ripple = setTimelineTransition(timeline(), "/tracks/visual/0/clips/0", "ripple", 0.25);
  expect(setTimelineTransitionParameter(ripple, "/tracks/visual/0/clips/0", "amplitude", 1/30).tracks.visual![0]!.transitions![0]!.params).toEqual({});
  for (const [kind, specs] of Object.entries(transitionParameterSpecs)) {
    const base = setTimelineTransition(timeline(), "/tracks/visual/0/clips/0", kind as keyof typeof transitionParameterSpecs, 0.25);
    for (const spec of specs) {
      const changed = setTimelineTransitionParameter(base, "/tracks/visual/0/clips/0", spec.name, spec.max);
      expect(changed.tracks.visual![0]!.transitions![0]!.params?.[spec.name] ?? spec.default).toBe(spec.max);
      expect(() => setTimelineTransitionParameter(base, "/tracks/visual/0/clips/0", spec.name, NaN)).toThrow();
    }
  }
});
