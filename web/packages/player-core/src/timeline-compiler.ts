import type { Timeline } from "@valle/engine";
import type { TimelineDocument } from "@valle/engine/internal";

import type { CanonicalTimelineDocument } from "./player/product-controller.ts";
import { resolvePlayerRuntimeAssets } from "./runtime-assets.ts";

interface TimelineCompilerWasmModule {
  default(wasmUrl: string): Promise<unknown>;
  canonicalize_timeline_document(timelineJson: string): string;
  compile_timeline(timelineJson: string): string;
  normalize_timeline(timelineJson: string): string;
  timeline_source_time_delta_from_frames(
    frames: number,
    fpsJson: string,
    rateJson: string,
  ): number;
  timeline_time_from_frames(frames: number, fpsJson: string): number;
  timeline_document_view(timelineJson: string): string;
}

export interface TimelineCompilerRuntime {
  /** Normalize the sparse working copy through Rust admission/q6 rules. */
  normalizeTimeline(timeline: Timeline): Timeline;
  /** Convert frame counts with exact Rust rational/q6 semantics. */
  timelineTimeFromFrames(frames: number, fps: Timeline["canvas"]["fps"]): number;
  /** Convert composition-frame deltas to source seconds through the exact clip rate. */
  timelineSourceTimeDeltaFromFrames(
    frames: number,
    fps: Timeline["canvas"]["fps"],
    rate?: number | null,
  ): number;
  /** Read-only projection used by Studio preview and inspection. */
  compileTimeline(timeline: Timeline): CanonicalTimelineDocument;
  canonicalizeTimelineDocument(timeline: TimelineDocument): CanonicalTimelineDocument;
}

export interface TimelineCompilerRuntimeOptions {
  runtimeAssets: unknown;
  runtimeBaseUrl?: string | URL;
}

const moduleLoads = new Map<string, Promise<TimelineCompilerWasmModule>>();

/**
 * Load only the Rust/WASM Timeline compiler surface. This intentionally does not initialize the
 * product renderer, CanvasKit, media decoders, or a fixed package.
 */
export async function createTimelineCompilerRuntime(
  options: TimelineCompilerRuntimeOptions,
): Promise<TimelineCompilerRuntime> {
  const runtimeAssets = resolvePlayerRuntimeAssets(options.runtimeAssets, options.runtimeBaseUrl);
  const wasm = await loadTimelineCompilerWasm(runtimeAssets.engine.glue, runtimeAssets.engine.wasm);
  return {
    normalizeTimeline: (timeline) => normalizeTimelineWithWasm(wasm, timeline),
    timelineTimeFromFrames: (frames, fps) => timelineTimeFromFramesWithWasm(wasm, frames, fps),
    timelineSourceTimeDeltaFromFrames: (frames, fps, rate) => (
      timelineSourceTimeDeltaFromFramesWithWasm(wasm, frames, fps, rate)
    ),
    compileTimeline: (timeline) => compileTimelineWithWasm(wasm, timeline),
    canonicalizeTimelineDocument: (timeline) => canonicalizeTimelineDocumentWithWasm(wasm, timeline),
  };
}

/** Normalize one sparse Timeline without introducing a JavaScript q6 implementation. */
export function normalizeTimelineWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "normalize_timeline">,
  timeline: Timeline,
): Timeline {
  return JSON.parse(wasm.normalize_timeline(JSON.stringify(timeline))) as Timeline;
}

/** Convert a frame count to q6 seconds in Rust using the exact authored frame rate. */
export function timelineTimeFromFramesWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "timeline_time_from_frames">,
  frames: number,
  fps: Timeline["canvas"]["fps"],
): number {
  return wasm.timeline_time_from_frames(frames, JSON.stringify(fps));
}

/** Apply the source playback rate before Rust performs the one canonical q6 rounding. */
export function timelineSourceTimeDeltaFromFramesWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "timeline_source_time_delta_from_frames">,
  frames: number,
  fps: Timeline["canvas"]["fps"],
  rate?: number | null,
): number {
  return wasm.timeline_source_time_delta_from_frames(
    frames,
    JSON.stringify(fps),
    JSON.stringify(rate ?? 1),
  );
}

/** Compile a sparse public Timeline into a renderer-only canonical projection. */
export function compileTimelineWithWasm(
  wasm: Pick<
    TimelineCompilerWasmModule,
    "compile_timeline" | "timeline_document_view"
  >,
  timeline: Timeline,
): CanonicalTimelineDocument {
  const timelineJson = wasm.compile_timeline(JSON.stringify(timeline));
  return canonicalTimelineDocumentFromJson(wasm, timelineJson);
}

/** Keep player and compiler-only callers on the exact same Rust canonicalization/view seam. */
export function canonicalizeTimelineDocumentWithWasm(
  wasm: Pick<
    TimelineCompilerWasmModule,
    "canonicalize_timeline_document" | "timeline_document_view"
  >,
  timeline: TimelineDocument,
): CanonicalTimelineDocument {
  const timelineJson = wasm.canonicalize_timeline_document(JSON.stringify(timeline));
  return canonicalTimelineDocumentFromJson(wasm, timelineJson);
}

function canonicalTimelineDocumentFromJson(
  wasm: Pick<TimelineCompilerWasmModule, "timeline_document_view">,
  timelineJson: string,
): CanonicalTimelineDocument {
  return {
    timeline: JSON.parse(timelineJson) as TimelineDocument,
    timelineJson,
    view: JSON.parse(wasm.timeline_document_view(timelineJson)) as CanonicalTimelineDocument["view"],
  };
}

function loadTimelineCompilerWasm(
  moduleUrl: string,
  wasmUrl: string,
): Promise<TimelineCompilerWasmModule> {
  const key = `${moduleUrl}\n${wasmUrl}`;
  const existing = moduleLoads.get(key);
  if (existing) return existing;
  const loading = (async () => {
    const module = await import(moduleUrl) as unknown as TimelineCompilerWasmModule;
    await module.default(wasmUrl);
    return module;
  })();
  moduleLoads.set(key, loading);
  void loading.catch(() => moduleLoads.delete(key));
  return loading;
}
