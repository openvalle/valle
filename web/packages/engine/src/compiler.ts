import type { Timeline } from "./timeline.ts";
import type { TimelineDocument } from "./internal-timeline.ts";

import type { CanonicalTimelineDocument } from "./runtime/product-controller.ts";
import { resolveEngineRuntimeAssets } from "./runtime-assets.ts";

interface TimelineCompilerWasmModule {
  default(wasmUrl: string): Promise<unknown>;
  canonicalize_timeline_document(timelineJson: string): string;
  compile_timeline(timelineJson: string, motionSourcesJson: string): string;
  normalize_timeline(timelineJson: string): string;
  timeline_source_time_delta_from_frames(
    frames: number,
    fpsJson: string,
    rateJson: string,
  ): number;
  timeline_time_from_frames(frames: number, fpsJson: string): number;
  timeline_document_view(timelineJson: string): string;
  sample_motion_properties(artifactJson: string, requestJson: string): string;
  compile_motion_jsx(
    source: string, optionsJson: string, fonts: Uint8Array[],
    aliases: Array<[string, Uint8Array]>, shaders: MotionShaderPackage[],
  ): string;
  compile_motion_modules(
    entry: string, modulesJson: string, optionsJson: string, fonts: Uint8Array[],
    aliases: Array<[string, Uint8Array]>, shaders: MotionShaderPackage[],
  ): string;
}

export interface MotionCompilerDiagnostic {
  class: string;
  code: string;
  span: { start: number; end: number; line: number; column: number };
  sourcePath?: string;
  nodePath?: string;
  utility?: string;
  style?: unknown;
  message: string;
}

export class MotionCompileError extends Error {
  readonly diagnostics: MotionCompilerDiagnostic[];

  constructor(diagnostics: MotionCompilerDiagnostic[]) {
    super(diagnostics.map((diagnostic) => diagnostic.message).join("\n"));
    this.name = "MotionCompileError";
    this.diagnostics = diagnostics;
  }
}

/** Frozen packages must have the same bytes when the artifact is rendered. */
export interface MotionShaderPackage {
  frozenBytes: Uint8Array;
  assetControl?: string;
}

export interface MotionCompileOptions {
  resources?: readonly { control: string; contentHash: string }[];
  data?: { source: string; value: unknown };
  /** TTF/OTF bytes in the same order used by the renderer. */
  fonts?: readonly Uint8Array[];
  /** Asset font family aliases such as `asset://brandFont`. */
  fontAliases?: Readonly<Record<string, Uint8Array>>;
  shaders?: readonly MotionShaderPackage[];
}

export interface CompiledMotion {
  artifact: Record<string, unknown>;
  artifactDigest: string;
  sourceMap: Record<string, unknown>;
  normalizedSource: string;
  normalizedAstDigest: string;
  preparedDataDigest: string;
}

export interface MotionPropertyRequest {
  node: string;
  startFrame: number; endFrame: number; maxPoints: number; durationFrames: number;
  fps: string;
  props: Record<string, unknown>;
  viewport: [number, number];
}
export interface MotionPropertySamples {
  node: string;
  frames: number[];
  channels: Array<{
    property: string; unit: string; unavailable: string | null;
    samples: Array<{ frame: number; value: number[]; velocity: number[] | null; boundary: string | null }>;
  }>;
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
  compileTimeline(timeline: Timeline, motionSources?: Record<string, number>): CanonicalTimelineDocument;
  canonicalizeTimelineDocument(timeline: TimelineDocument): CanonicalTimelineDocument;
}

export interface WebCompilerRuntime extends TimelineCompilerRuntime {
  /** Compile a standalone Motion TSX/JSX source to a Scene artifact. */
  compileMotionJsx(source: string, options?: MotionCompileOptions): CompiledMotion;
  /** Compile an explicit project-relative module closure to a Scene artifact. */
  compileMotionModules(entry: string, modules: Record<string, string>, options?: MotionCompileOptions): CompiledMotion;
}

export interface TimelineCompilerRuntimeOptions {
  runtimeAssets: unknown;
  runtimeBaseUrl?: string | URL;
}

const moduleLoads = new Map<string, Promise<TimelineCompilerWasmModule>>();

/**
 * Load the Rust/WASM Timeline and Motion compiler surface without initializing the product
 * renderer, CanvasKit, media decoders, or a fixed package.
 */
export async function createTimelineCompilerRuntime(
  options: TimelineCompilerRuntimeOptions,
): Promise<WebCompilerRuntime> {
  const engine = resolveEngineRuntimeAssets(options.runtimeAssets, options.runtimeBaseUrl);
  const wasm = await loadTimelineCompilerWasm(engine.glue, engine.wasm);
  return {
    normalizeTimeline: (timeline) => normalizeTimelineWithWasm(wasm, timeline),
    timelineTimeFromFrames: (frames, fps) => timelineTimeFromFramesWithWasm(wasm, frames, fps),
    timelineSourceTimeDeltaFromFrames: (frames, fps, rate) => (
      timelineSourceTimeDeltaFromFramesWithWasm(wasm, frames, fps, rate)
    ),
    compileTimeline: (timeline, motionSources) => compileTimelineWithWasm(wasm, timeline, motionSources),
    canonicalizeTimelineDocument: (timeline) => canonicalizeTimelineDocumentWithWasm(wasm, timeline),
    compileMotionJsx: (source, compileOptions) => compileMotionJsxWithWasm(wasm, source, compileOptions),
    compileMotionModules: (entry, modules, compileOptions) => (
      compileMotionModulesWithWasm(wasm, entry, modules, compileOptions)
    ),
  };
}

/** Bounded local-property inspection; no layout or rendering. Run in a diagnostic worker. */
export async function sampleMotionProperties(options: TimelineCompilerRuntimeOptions, artifact: Record<string, unknown>, request: MotionPropertyRequest): Promise<MotionPropertySamples> {
  const engine = resolveEngineRuntimeAssets(options.runtimeAssets, options.runtimeBaseUrl);
  const wasm = await loadTimelineCompilerWasm(engine.glue, engine.wasm);
  return JSON.parse(wasm.sample_motion_properties(JSON.stringify(artifact), JSON.stringify(request))) as MotionPropertySamples;
}

export function compileMotionJsxWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "compile_motion_jsx">,
  source: string,
  options: MotionCompileOptions = {},
): CompiledMotion {
  const inputs = motionCompileInputs(options);
  return readMotionCompileResult(wasm.compile_motion_jsx(
    source, inputs.optionsJson, inputs.fonts, inputs.aliases, inputs.shaders,
  ));
}

export function compileMotionModulesWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "compile_motion_modules">,
  entry: string,
  modules: Record<string, string>,
  options: MotionCompileOptions = {},
): CompiledMotion {
  const inputs = motionCompileInputs(options);
  return readMotionCompileResult(wasm.compile_motion_modules(
    entry, JSON.stringify(modules), inputs.optionsJson, inputs.fonts, inputs.aliases, inputs.shaders,
  ));
}

function motionCompileInputs(options: MotionCompileOptions): {
  optionsJson: string;
  fonts: Uint8Array[];
  aliases: Array<[string, Uint8Array]>;
  shaders: MotionShaderPackage[];
} {
  return {
    optionsJson: JSON.stringify({ resources: options.resources ?? [], data: options.data ?? null }),
    fonts: [...options.fonts ?? []],
    aliases: Object.entries(options.fontAliases ?? {}),
    shaders: [...options.shaders ?? []],
  };
}

function readMotionCompileResult(json: string): CompiledMotion {
  const result = JSON.parse(json) as
    | ({ status: "ok" } & CompiledMotion)
    | { status: "error"; diagnostics: MotionCompilerDiagnostic[] };
  if (result.status === "error") throw new MotionCompileError(result.diagnostics);
  if (result.status !== "ok") throw new TypeError("invalid Motion compiler response");
  return {
    artifact: result.artifact,
    artifactDigest: result.artifactDigest,
    sourceMap: result.sourceMap,
    normalizedSource: result.normalizedSource,
    normalizedAstDigest: result.normalizedAstDigest,
    preparedDataDigest: result.preparedDataDigest,
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
  motionSources: Record<string, number> = {},
): CanonicalTimelineDocument {
  const timelineJson = wasm.compile_timeline(JSON.stringify(timeline), JSON.stringify(motionSources));
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
