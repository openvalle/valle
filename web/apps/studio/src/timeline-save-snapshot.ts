import type {
  EditTimelineResponse,
  Timeline,
} from "@valle/engine";
import type { StudioTimelineRevision } from "./host.ts";

type AcceptedTimelineSave = Extract<
  EditTimelineResponse,
  { outcome: "committed" | "unchanged" }
>;

export interface ReloadedTimelineSnapshot {
  timelineRevision: StudioTimelineRevision;
  timelineJson: string;
  timeline: Timeline;
}

export interface CapturedTimelineSave {
  timeline: Timeline;
  timelineJson: string;
}

/** Capture the complete sparse Timeline before an asynchronous save begins. */
export function captureTimelineSaveSnapshot(workingCopy: Timeline): CapturedTimelineSave {
  const timeline = structuredClone(workingCopy);
  return { timeline, timelineJson: JSON.stringify(timeline) };
}

/**
 * Verify only the public full-document save contract: the accepted revision is
 * the one reloaded, and its sparse timeline.json is the submitted document.
 * Renderer hashes and manifests are derived internals and intentionally do not
 * participate in this check.
 */
export function assertReloadedTimelineSave(
  response: AcceptedTimelineSave,
  reloaded: ReloadedTimelineSnapshot,
  submitted: CapturedTimelineSave,
): Timeline {
  assertSame("Timeline revision", response.revision, reloaded.timelineRevision.revision);

  let serializedTimeline: Timeline;
  try {
    serializedTimeline = JSON.parse(reloaded.timelineJson) as Timeline;
  } catch (error) {
    throw new Error(`reloaded timelineJson is not JSON: ${errorText(error)}`);
  }
  assertJsonSame("reloaded timelineJson", reloaded.timeline, serializedTimeline);
  assertJsonSame("reloaded complete Timeline", submitted.timeline, reloaded.timeline);
  return structuredClone(reloaded.timeline);
}

function assertSame(label: string, expected: number, actual: number): void {
  if (actual !== expected) {
    throw new Error(`saved snapshot ${label} drift: expected ${expected}, got ${actual}`);
  }
}

function assertJsonSame(label: string, expected: unknown, actual: unknown): void {
  if (!jsonEqual(expected, actual)) throw new Error(`saved snapshot ${label} drift`);
}

function jsonEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left)
      && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => jsonEqual(value, right[index]));
  }
  if (isRecord(left) || isRecord(right)) {
    if (!isRecord(left) || !isRecord(right)) return false;
    const leftKeys = Object.keys(left).sort();
    const rightKeys = Object.keys(right).sort();
    return jsonEqual(leftKeys, rightKeys)
      && leftKeys.every((key) => jsonEqual(left[key], right[key]));
  }
  return false;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
