import type { JsonValue, MotionRole, Timeline, TimelineSchema } from "valle-engine";
import transitionParameters from "../../../../crates/valle-timeline/schema/transition-parameters.generated.json";

type TimelineVisualTrack = TimelineSchema.TimelineVisualTrackWire;
type TimelineAudioTrack = TimelineSchema.TimelineAudioTrackWire;
type TimelineCaptionTrack = TimelineSchema.TimelineCaptionTrackWire;
type TimelineAdjustmentTrack = TimelineSchema.TimelineAdjustmentTrackWire;
type TimelineVisualClip = TimelineSchema.TimelineVisualClipWire;
type TimelineAudioClip = TimelineSchema.TimelineAudioClipWire;
type TimelineCaptionClip = TimelineSchema.TimelineCaptionClipWire;
type TimelineAdjustmentClip = TimelineSchema.TimelineAdjustmentClipWire;

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
  motionRole?: MotionRole,
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
        && target.clip.kind !== "solid"
        && !(target.clip.kind === "motion" && motionRole?.type === "overlay")) {
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
    case "visual": {
      target.track.clips.splice(address.clipIndex, 1);
      target.track.transitions = target.track.transitions?.filter(t => t.from !== address.clipIndex && t.to !== address.clipIndex)
        .map(t => ({ ...t, from: t.from > address.clipIndex ? t.from - 1 : t.from, to: t.to > address.clipIndex ? t.to - 1 : t.to }));
      break;
    }
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
  const originalClips = [...track.clips];
  const transitions = (track.transitions ?? []).map(t => ({
    ...t, fromClip: originalClips[t.from]!, toClip: originalClips[t.to]!,
    duration: originalClips[t.from]!.start + originalClips[t.from]!.duration - originalClips[t.to]!.start,
  }));
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
  const retained = transitions.flatMap(t => {
    const from = track.clips.indexOf(t.fromClip), to = track.clips.indexOf(t.toClip);
    return to === from + 1 ? [{ from, to, kind: t.kind, params: t.params, duration: t.duration }] : [];
  });
  track.transitions = retained.map(({ from, to, kind, params }) => ({ from, to, kind, ...(params ? { params } : {}) }));
  let cursor = slots[0]?.start ?? 0;
  for (let index = 0; index < track.clips.length; index += 1) {
    const current = track.clips[index]!;
    current.start = cursor;
    const transition = retained.find(t => t.from === index);
    cursor += finiteNumber(current.duration, `clip ${index} duration`)
      + (transition ? -transition.duration : (slots[index]?.gapAfter ?? 0));
  }
  return next;
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

export function clearTimelineMotionProp(
  timeline: Timeline,
  timelinePath: string,
  name: string,
): Timeline {
  return editTimelineClip(timeline, timelinePath, (target) => {
    if (target.band !== "visual" || target.clip.kind !== "motion") {
      throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
    }
    if (!target.clip.props || !Object.hasOwn(target.clip.props, name)) return;
    delete target.clip.props[name];
    if (!Object.keys(target.clip.props).length) delete target.clip.props;
  });
}

export function setTimelineMotionData(
  timeline: Timeline,
  timelinePath: string,
  value: Record<string, JsonValue>,
): Timeline {
  return editTimelineClip(timeline, timelinePath, (target) => {
    if (target.band !== "visual" || target.clip.kind !== "motion") {
      throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
    }
    target.clip.data = structuredClone(value);
  });
}

export function setTimelineMotionResource(
  timeline: Timeline,
  timelinePath: string,
  slot: string,
  alias: string,
): Timeline {
  if (!Object.hasOwn(timeline.resources ?? {}, alias)) {
    throw new Error(`Timeline resource '${alias}' does not exist`);
  }
  return editTimelineClip(timeline, timelinePath, (target) => {
    if (target.band !== "visual" || target.clip.kind !== "motion") {
      throw new Error(`Timeline clip '${timelinePath}' is not Motion`);
    }
    target.clip.resources = { ...(target.clip.resources ?? {}), [slot]: alias };
  });
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


export type TimelineTransitionKind = TimelineSchema.TransitionKind;
export const transitionParameterSpecs = transitionParameters;
export const transitionLabels = {
  fade: "Fade", wipeLeft: "Wipe left", wipeRight: "Wipe right", circleOpen: "Circle open",
  simpleZoom: "Simple zoom", crossWarp: "Cross warp", linearBlur: "Linear blur",
  directionalWarp: "Directional warp", dreamyZoom: "Dreamy zoom", ripple: "Ripple",
  flyEye: "Fly eye", multiplyBlend: "Multiply blend", perlin: "Perlin noise",
} satisfies Record<TimelineTransitionKind, string>;

/** Ripple the following clips so overlap changes preserve all endpoint local clocks. */
export function setTimelineTransition(
  timeline: Timeline,
  timelinePath: string,
  kind: TimelineTransitionKind | null,
  durationSeconds?: number,
): Timeline {
  const next = structuredClone(timeline);
  const address = requireAddress(timelinePath);
  const target = requireClip(next, timelinePath);
  if (target.band !== "visual") throw new Error("Transitions require two visual clips");
  const track = target.track;
  const from = address.clipIndex, to = from + 1;
  const a = track.clips[from]!, b = track.clips[to];
  if (!b) throw new Error("Select a visual clip with a following clip");
  if (kind !== null && !Object.hasOwn(transitionLabels, kind)) throw new Error("Unknown transition kind");
  const existing = track.transitions?.find(t => t.from === from && t.to === to);
  if (!kind && !existing) return next;
  const overlap = existing ? a.start + a.duration - b.start : 0;
  const duration = kind ? durationSeconds ?? (existing ? overlap : Math.min(0.5, a.duration / 2, b.duration / 2)) : 0;
  if (!Number.isFinite(duration) || (kind && (duration <= 0 || duration >= a.duration || duration >= b.duration))) {
    throw new Error("Transition duration must be positive and shorter than both clips");
  }
  const delta = a.start + a.duration - duration - b.start;
  for (let index = to; index < track.clips.length; index++) track.clips[index]!.start += delta;
  track.transitions = (track.transitions ?? []).filter(t => t.from !== from);
  if (kind) track.transitions.push({ from, to, kind, ...(existing?.kind === kind && existing.params ? { params: existing.params } : {}) });
  return next;
}

/** Parameters use generated Rust metadata; compilation remains the final semantic admission. */
export function setTimelineTransitionParameter(timeline: Timeline, timelinePath: string, name: string, value: number): Timeline {
  const next = structuredClone(timeline);
  const address = requireAddress(timelinePath);
  const target = requireClip(next, timelinePath);
  if (target.band !== "visual") throw new Error("Transitions require visual clips");
  const transition = target.track.transitions?.find(t => t.from === address.clipIndex);
  if (!transition) throw new Error("Transition does not exist");
  const spec = transitionParameterSpecs[transition.kind].find(spec => spec.name === name);
  if (!spec) throw new Error(`Unknown transition parameter '${name}'`);
  if (!Number.isFinite(value) || Math.fround(value) < Math.fround(spec.min) || Math.fround(value) > Math.fround(spec.max)) throw new Error(`${spec.label} must be in [${spec.min}, ${spec.max}]`);
  const params = { ...transition.params, [name]: value };
  if (transition.kind === "directionalWarp" && Math.abs(params.directionX ?? -1) + Math.abs(params.directionY ?? 1) < 0.000001) {
    throw new Error("Transition direction must have nonzero length");
  }
  if (Math.fround(value) === Math.fround(spec.default)) delete params[name];
  transition.params = params;
  return next;
}
