import type { JsonValue, Timeline, TimelineSchema } from "@valle/engine";

type TimelineVisualTrack = TimelineSchema.TimelineVisualTrackWire;
type TimelineAudioTrack = TimelineSchema.TimelineAudioTrackWire;
type TimelineCaptionTrack = TimelineSchema.TimelineCaptionTrackWire;
type TimelineAdjustmentTrack = TimelineSchema.TimelineAdjustmentTrackWire;
type TimelineVisualClip = TimelineSchema.TimelineVisualClipWire;
type TimelineAudioClip = TimelineSchema.TimelineAudioClipWire;
type TimelineCaptionClip = TimelineSchema.TimelineCaptionClipWire;
type TimelineAdjustmentClip = TimelineSchema.TimelineAdjustmentClipWire;
type TimelineMotionCue = TimelineSchema.TimelineMotionCueBindingWire;

export type TimelineTrackBand = "visual" | "audio" | "caption" | "adjustment";

export interface TimelineClipAddress {
  band: TimelineTrackBand;
  trackIndex: number;
  clipIndex: number;
}

type TimelineMotionClip = Extract<TimelineVisualClip, { kind: "motion" }>;

export type TimelineClipTarget =
  | { band: "visual"; track: TimelineVisualTrack; clip: TimelineVisualClip }
  | { band: "audio"; track: TimelineAudioTrack; clip: TimelineAudioClip }
  | { band: "caption"; track: TimelineCaptionTrack; clip: TimelineCaptionClip }
  | { band: "adjustment"; track: TimelineAdjustmentTrack; clip: TimelineAdjustmentClip };

export type TimelineTimeFromFrames = (
  frames: number,
  fps: Timeline["canvas"]["fps"],
) => number;

export type TimelineSourceTimeDeltaFromFrames = (
  frames: number,
  fps: Timeline["canvas"]["fps"],
  rate?: number | null,
) => number;

/** Decode the compiler-provided source JSON Pointer for one public Timeline clip. */
export function timelineClipAddress(timelinePath: string): TimelineClipAddress | null {
  const match = /^\/tracks\/(visual|audio|caption|adjustment)\/(0|[1-9]\d*)\/clips\/(0|[1-9]\d*)$/.exec(timelinePath);
  if (!match) return null;
  return {
    band: match[1] as TimelineTrackBand,
    trackIndex: Number(match[2]),
    clipIndex: Number(match[3]),
  };
}

export function editTimelineClip(
  timeline: Timeline,
  timelinePath: string,
  edit: (target: TimelineClipTarget) => void,
): Timeline {
  const next = structuredClone(timeline);
  edit(requireClip(next, timelinePath));
  return next;
}

export function setTimelineClipDurationFrames(
  timeline: Timeline,
  timelinePath: string,
  frames: number,
  timeFromFrames: TimelineTimeFromFrames,
): Timeline {
  if (!Number.isSafeInteger(frames) || frames <= 0) {
    throw new Error("Timeline duration frames must be a positive safe integer");
  }
  return editTimelineClip(timeline, timelinePath, ({ clip }) => {
    clip.duration = timeFromFrames(frames, timeline.canvas.fps);
  });
}

export function setTimelineSourceStartFrames(
  timeline: Timeline,
  timelinePath: string,
  frames: number,
  timeFromFrames: TimelineTimeFromFrames,
): Timeline {
  if (!Number.isSafeInteger(frames) || frames < 0) {
    throw new Error("Timeline source start frames must be a non-negative safe integer");
  }
  const address = requireAddress(timelinePath);
  if (address.band !== "visual" && address.band !== "audio") {
    throw new Error(`Timeline clip '${timelinePath}' has no source trim`);
  }
  return editTimelineClip(timeline, timelinePath, (target) => {
    const seconds = timeFromFrames(frames, timeline.canvas.fps);
    if (target.band === "audio") {
      target.clip.trimStart = seconds;
      return;
    }
    if (target.band !== "visual"
      || target.clip.kind === "image"
      || target.clip.kind === "solid") {
      throw new Error(`Timeline clip '${timelinePath}' has no source trim`);
    }
    target.clip.trimStart = seconds;
  });
}

export function trimTimelineClipFrames(
  timeline: Timeline,
  timelinePath: string,
  edge: "start" | "end",
  deltaFrames: number,
  timeFromFrames: TimelineTimeFromFrames,
  sourceTimeDeltaFromFrames: TimelineSourceTimeDeltaFromFrames,
): Timeline {
  if (!Number.isSafeInteger(deltaFrames)) {
    throw new Error("Timeline trim delta must be a safe integer");
  }
  const delta = timeFromFrames(deltaFrames, timeline.canvas.fps);
  return editTimelineClip(timeline, timelinePath, (target) => {
    const clip = target.clip;
    const start = finiteNumber(clip.start, "clip start");
    const duration = finiteNumber(clip.duration, "clip duration");
    let trimStart: number | undefined;
    if (edge === "end") {
      clip.duration = duration + delta;
    } else {
      clip.start = start + delta;
      clip.duration = duration - delta;
      if (target.band === "audio") {
        const sourceDelta = finiteNumber(
          sourceTimeDeltaFromFrames(deltaFrames, timeline.canvas.fps, target.clip.rate),
          "source trim delta",
        );
        target.clip.trimStart = finiteOptionalNumber(target.clip.trimStart, 0) + sourceDelta;
        trimStart = target.clip.trimStart;
      } else if (target.band === "visual"
        && target.clip.kind !== "image"
        && target.clip.kind !== "solid") {
        const sourceDelta = finiteNumber(
          sourceTimeDeltaFromFrames(deltaFrames, timeline.canvas.fps, target.clip.rate),
          "source trim delta",
        );
        target.clip.trimStart = finiteOptionalNumber(target.clip.trimStart, 0) + sourceDelta;
        trimStart = target.clip.trimStart;
      }
    }
    if (finiteNumber(clip.start, "clip start") < 0
      || finiteNumber(clip.duration, "clip duration") <= 0
      || (trimStart !== undefined && trimStart < 0)) {
      throw new Error("Timeline trim would create a negative start or non-positive duration");
    }
  });
}

export function deleteTimelineClip(timeline: Timeline, timelinePath: string): Timeline {
  const next = structuredClone(timeline);
  const address = requireAddress(timelinePath);
  const target = requireClip(next, timelinePath);
  switch (target.band) {
    case "visual": target.track.clips.splice(address.clipIndex, 1); break;
    case "audio": target.track.clips.splice(address.clipIndex, 1); break;
    case "caption": target.track.clips.splice(address.clipIndex, 1); break;
    case "adjustment": target.track.clips.splice(address.clipIndex, 1); break;
  }
  return next;
}

export function moveTimelineVisualClipBefore(
  timeline: Timeline,
  timelinePath: string,
  beforeTimelinePath: string,
): Timeline {
  const source = requireAddress(timelinePath);
  const target = requireAddress(beforeTimelinePath);
  if (source.band !== "visual" || target.band !== "visual"
    || source.trackIndex !== target.trackIndex) {
    throw new Error("Studio can only reorder clips within one visual track");
  }
  const next = structuredClone(timeline);
  const track = next.tracks.visual?.[source.trackIndex];
  if (!track || !track.clips[target.clipIndex]) {
    throw new Error("Timeline visual reorder target does not exist");
  }
  const slots = track.clips.map((clip, index) => ({
    start: finiteNumber(clip.start, `clip ${index} start`),
    gapAfter: index + 1 < track.clips.length
      ? Math.max(0,
        finiteNumber(track.clips[index + 1]!.start, `clip ${index + 1} start`)
          - finiteNumber(clip.start, `clip ${index + 1} start`)
          - finiteNumber(clip.duration, `clip ${index} duration`))
      : 0,
  }));
  const [clip] = track.clips.splice(source.clipIndex, 1);
  if (!clip) throw new Error(`Timeline clip '${timelinePath}' does not exist`);
  const insertion = source.clipIndex < target.clipIndex ? target.clipIndex - 1 : target.clipIndex;
  track.clips.splice(insertion, 0, clip);
  let cursor = slots[0]?.start ?? 0;
  for (let index = 0; index < track.clips.length; index += 1) {
    const current = track.clips[index]!;
    current.start = cursor;
    cursor += finiteNumber(current.duration, `clip ${index} duration`) + (slots[index]?.gapAfter ?? 0);
  }
  return next;
}

export function editTimelineMotionFrames(
  timeline: Timeline,
  timelinePath: string,
  edit:
    | { type: "motionPhaseFrames"; phase: "enter" | "exit"; frames: number }
    | { type: "motionCueFrames"; cue: string; field: "start" | "end" | "enter" | "exit"; frames: number },
  timeFromFrames: TimelineTimeFromFrames,
): Timeline {
  if (!Number.isSafeInteger(edit.frames) || edit.frames < 0) {
    throw new Error("Motion frame value must be a non-negative safe integer");
  }
  const address = requireAddress(timelinePath);
  if (address.band !== "visual") {
    throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
  }
  const seconds = timeFromFrames(edit.frames, timeline.canvas.fps);
  return editTimelineClip(timeline, timelinePath, (target) => {
    if (target.band !== "visual" || target.clip.kind !== "motion") {
      throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
    }
    const clip = target.clip;
    if (edit.type === "motionPhaseFrames") {
      clip.phases = edit.phase === "enter"
        ? { ...clip.phases, enterDuration: seconds }
        : { ...clip.phases, exitDuration: seconds };
      return;
    }
    const cues = { ...clip.cues };
    const cue: TimelineMotionCue | undefined = cues[edit.cue];
    if (!cue) throw new Error(`Motion cue '${edit.cue}' does not exist`);
    if ((edit.field === "start" || edit.field === "end") && cue.type !== "source-range") {
      throw new Error(`Motion cue '${edit.cue}' has no source range`);
    }
    if (edit.field === "enter") {
      cues[edit.cue] = { ...cue, enterDuration: seconds };
    } else if (edit.field === "exit") {
      cues[edit.cue] = { ...cue, exitDuration: seconds };
    } else {
      // The guard above establishes that source-range cues are the only cues
      // that can reach this branch. Keep the mutation inside the narrowed
      // branch so TypeScript cannot manufacture start/end on word cues.
      if (cue.type !== "source-range") {
        throw new Error(`Motion cue '${edit.cue}' has no source range`);
      }
      cues[edit.cue] = edit.field === "start"
        ? { ...cue, start: seconds }
        : { ...cue, end: seconds };
    }
    clip.cues = cues;
  });
}

export function setTimelineMotionProp(
  timeline: Timeline,
  timelinePath: string,
  name: string,
  value: JsonValue,
): Timeline {
  const address = requireAddress(timelinePath);
  if (address.band !== "visual") {
    throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
  }
  const next = structuredClone(timeline);
  const clip: TimelineVisualClip | undefined = next.tracks.visual?.[address.trackIndex]
    ?.clips[address.clipIndex];
  if (!clip || clip.kind !== "motion") {
    throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
  }
  const motion: TimelineMotionClip = clip;
  motion.props = { ...motion.props, [name]: structuredClone(value) };
  return next;
}

function requireClip(timeline: Timeline, timelinePath: string): TimelineClipTarget {
  const address = requireAddress(timelinePath);
  switch (address.band) {
    case "visual": {
      const track = timeline.tracks.visual?.[address.trackIndex];
      const clip = track?.clips[address.clipIndex];
      if (track && clip) return { band: "visual", track, clip };
      break;
    }
    case "audio": {
      const track = timeline.tracks.audio?.[address.trackIndex];
      const clip = track?.clips[address.clipIndex];
      if (track && clip) return { band: "audio", track, clip };
      break;
    }
    case "caption": {
      const track = timeline.tracks.caption?.[address.trackIndex];
      const clip = track?.clips[address.clipIndex];
      if (track && clip) return { band: "caption", track, clip };
      break;
    }
    case "adjustment": {
      const track = timeline.tracks.adjustment?.[address.trackIndex];
      const clip = track?.clips[address.clipIndex];
      if (track && clip) return { band: "adjustment", track, clip };
      break;
    }
  }
  throw new Error(`Timeline clip '${timelinePath}' does not exist`);
}

function requireAddress(timelinePath: string): TimelineClipAddress {
  const address = timelineClipAddress(timelinePath);
  if (!address) throw new Error(`'${timelinePath}' is not an editable Timeline path`);
  return address;
}

function finiteNumber(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`${label} is invalid`);
  return value;
}

function finiteOptionalNumber(value: unknown, fallback: number): number {
  return value === undefined ? fallback : finiteNumber(value, "Timeline number");
}
