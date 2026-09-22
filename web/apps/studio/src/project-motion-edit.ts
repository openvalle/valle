import type { JsonValue } from "valle-engine";
import type { TimelineDocument } from "valle-engine/internal";
import type { GoodMotionContext } from "./motion-preview.ts";

export type ProjectMotionEdit =
  | { type: "prop"; name: string; value: { kind: string; value: JsonValue } }
  | { type: "data"; value: Record<string, JsonValue> };

type VisualItem = TimelineDocument["document"]["visual"]["tracks"][number]["items"][number];
type VisualClip = Extract<VisualItem, { type: "clip" }>;
type MotionSource = Extract<VisualClip["source"], { type: "motion" }>;

/** Resolve editor values from generated Timeline overrides plus admitted Artifact defaults. */
export function preparedMotionProps(
  timeline: TimelineDocument,
  motion: unknown,
  clipId: string,
  artifact: GoodMotionContext["artifact"],
): Record<string, { kind: string; value: unknown }> {
  const clip = findTimelineClip(timeline, clipId);
  const structure = findMotionStructure(motion, clipId);
  if (!clip || clip.source.type !== "motion" || !structure) return {};
  const controls = record(record(artifact.controls).props);
  const prepared = record(record(structure.authoring).props);
  const result: Record<string, { kind: string; value: unknown }> = {};
  for (const [name, schemaValue] of Object.entries(controls)) {
    const schema = record(schemaValue);
    const control = record(schema.control);
    const authored = clip.source.props[name];
    if (authored) {
      result[name] = {
        kind: motionValueKind(String(control.kind ?? "")),
        value: structuredClone(sampleAuthoredValue(authored)),
      };
      continue;
    }
    const resolved = prepared[name] ?? schema.default;
    if (isRecord(resolved) && typeof resolved.kind === "string" && Object.hasOwn(resolved, "value")) {
      result[name] = structuredClone(resolved) as { kind: string; value: unknown };
    }
  }
  return result;
}

export function findMotionStructure(
  motion: unknown,
  clipId: string,
): Record<string, unknown> | null {
  return arrayOfRecords(record(motion).structures).find((candidate) => candidate.clipId === clipId) ?? null;
}

function findTimelineClip(timeline: TimelineDocument, clipId: string): VisualClip | null {
  for (const track of timeline.document.visual.tracks) {
    const clip = track.items.find((candidate: VisualItem): candidate is VisualClip => (
      candidate.type === "clip" && candidate.id === clipId
    ));
    if (clip) return clip;
  }
  return null;
}

function motionValueKind(controlKind: string): string {
  switch (controlKind) {
    case "string":
      return "str";
    case "select":
      return "enum";
    default:
      return controlKind;
  }
}

function sampleAuthoredValue(value: MotionSource["props"][string]): unknown {
  if (value.type === "constant") return value.value;
  return value.keyframes[0]?.value ?? null;
}

function arrayOfRecords(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function record(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {};
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
