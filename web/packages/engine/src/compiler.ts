import type { Timeline } from "./timeline.ts";
import type { TimelineDocument, TimelineDocumentSchema } from "./internal-timeline.ts";

import type { CanonicalTimelineDocument } from "./runtime/product-controller.ts";
import { resolveEngineRuntimeAssets } from "./runtime-assets.ts";
import { loadEngineWasm } from "./engine-wasm.ts";

interface TimelineCompilerWasmModule {
  default(input: { module_or_path: string }): Promise<unknown>;
  canonicalize_timeline_document(timelineJson: string): string;
  compile_timeline(timelineJson: string, motionSourcesJson: string): string;
  motion_preparation_inputs(timelineJson: string): string;
  normalize_timeline(timelineJson: string): string;
  timeline_source_time_delta_from_frames(frames: number, fpsJson: string, rateJson: string): number;
  timeline_time_from_frames(frames: number, fpsJson: string): number;
  timeline_document_view(timelineJson: string): string;
  sample_motion_properties(artifactJson: string, requestJson: string): string;
  compile_motion_jsx(
    source: string,
    optionsJson: string,
    fonts: Uint8Array[],
    aliases: Array<[string, Uint8Array]>,
    shaders: MotionShaderPackage[],
    audioSamples: Float32Array[],
  ): string;
  compile_motion_modules(
    entry: string,
    modulesJson: string,
    optionsJson: string,
    fonts: Uint8Array[],
    aliases: Array<[string, Uint8Array]>,
    shaders: MotionShaderPackage[],
    audioSamples: Float32Array[],
  ): string;
  motion_module_paths(entry: string, modulesJson: string): string;
  font_covers_text(bytes: Uint8Array, text: string): boolean;
  prepare_preview_package(inputJson: string, resourceBytes: Uint8Array): [string, Uint8Array];
  rewrite_motion_source(inputJson: string): string;
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
  /** Timeline placement for role validation before host data is expanded. */
  placement?: {
    component: string;
    path: string;
    track: "visual" | "caption" | "transition" | "effect";
  };
  resources?: readonly { control: string; contentHash: string }[];
  data?: { source: string; value: unknown };
  /** Frozen decoded mono PCM matching a resource digest, resampled by the caller. */
  audioSources?: readonly {
    control: string;
    contentHash: string;
    sampleRate: number;
    samples: Float32Array;
  }[];
  /** TTF/OTF bytes in the same order used by the renderer. */
  fonts?: readonly Uint8Array[];
  /** Asset font family aliases such as `asset://brandFont`. */
  fontAliases?: Readonly<Record<string, Uint8Array>>;
  shaders?: readonly MotionShaderPackage[];
}

export interface CompiledMotion {
  artifact: Record<string, unknown>;
  artifactDigest: string;
  warnings: MotionCompilerDiagnostic[];
  sourceMap: Record<string, unknown>;
  normalizedSource: string;
  normalizedAstDigest: string;
  preparedDataDigest: string;
}

export interface PreviewMotionInstance {
  clipPath: string;
  artifact: Record<string, unknown>;
  artifactDigest: string;
  fonts: readonly { bytes: Uint8Array; role: "font" | "formula-font" }[];
}

export interface PreviewResourceInput {
  id: string;
  entry: Record<string, unknown>;
  facts: Record<string, unknown>;
  dependencies?: readonly { role: string; resourceId: string }[];
}

export interface PreviewPackageInput {
  authorTimeline: Timeline;
  motionInstances: readonly PreviewMotionInstance[];
  resourceInputs?: readonly PreviewResourceInput[];
  resourceBytes?: Uint8Array;
}

export type MotionRole = TimelineDocumentSchema.MotionRole;
export interface MotionSourceMetadata {
  duration: string;
  role: MotionRole;
}

export interface PreparedPreviewPackage {
  fixedPackageManifestJson: string;
  timelineJson: string;
  resourceManifestJson: string;
  verifiedBindingBundleJson: string;
  resourceBytes: Uint8Array;
  executionProfileJson: string;
  externalResources: readonly { id: string; digest: string }[];
  motionSourceMetadata: Record<string, MotionSourceMetadata>;
}

export interface MotionSourceEdit {
  source: string;
  target: { kind: "composition"; field: string } | { kind: "prop-default"; name: string };
  value: unknown;
}

export interface MotionSourceEditResult {
  source: string;
  /** Byte offsets in the previous UTF-8 source. */
  replacedSpan: [number, number];
}

export interface MotionPropertyRequest {
  node: string;
  startFrame: number;
  endFrame: number;
  maxPoints: number;
  durationFrames: number;
  fps: string;
  props: Record<string, unknown>;
  viewport: [number, number];
}
export interface MotionPropertySamples {
  node: string;
  frames: number[];
  channels: Array<{
    property: string;
    unit: string;
    unavailable: string | null;
    samples: Array<{
      frame: number;
      value: number[];
      velocity: number[] | null;
      boundary: string | null;
    }>;
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
  compileTimeline(
    timeline: Timeline,
    motionSources?: Record<string, MotionSourceMetadata>,
  ): CanonicalTimelineDocument;
  canonicalizeTimelineDocument(timeline: TimelineDocument): CanonicalTimelineDocument;
}

export interface WebCompilerRuntime extends TimelineCompilerRuntime {
  /** Whether an already bound face covers every non-control character. */
  fontCoversText(bytes: Uint8Array, text: string): boolean;
  /** Resolve the exact import closure with the shared Motion linker. */
  motionModulePaths(entry: string, modules: Record<string, string>): string[];
  /** Immutable Motion inputs, including captions prepared by the shared Rust compiler. */
  motionPreparationInputs(timeline: Timeline): MotionPreparationInput[];
  /** Compile a standalone Motion TSX/JSX source to a Scene artifact. */
  compileMotionJsx(source: string, options?: MotionCompileOptions): CompiledMotion;
  /** Compile an explicit project-relative module closure to a Scene artifact. */
  compileMotionModules(
    entry: string,
    modules: Record<string, string>,
    options?: MotionCompileOptions,
  ): CompiledMotion;
  preparePreviewPackage(input: PreviewPackageInput): PreparedPreviewPackage;
  rewriteMotionSource(input: MotionSourceEdit): MotionSourceEditResult;
}

export interface TimelineCompilerRuntimeOptions {
  runtimeAssets: unknown;
  runtimeBaseUrl?: string | URL;
}

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
    timelineSourceTimeDeltaFromFrames: (frames, fps, rate) =>
      timelineSourceTimeDeltaFromFramesWithWasm(wasm, frames, fps, rate),
    motionPreparationInputs: (timeline) =>
      JSON.parse(
        wasm.motion_preparation_inputs(JSON.stringify(timeline)),
      ) as MotionPreparationInput[],
    motionModulePaths: (entry, modules) => {
      const result = JSON.parse(wasm.motion_module_paths(entry, JSON.stringify(modules))) as
        | { status: "ok"; paths: string[] }
        | { status: "error"; diagnostics: MotionCompilerDiagnostic[] };
      if (result.status === "error") throw new MotionCompileError(result.diagnostics);
      return result.paths;
    },
    fontCoversText: (bytes, text) => wasm.font_covers_text(bytes, text),
    compileTimeline: (timeline, motionSources) =>
      compileTimelineWithWasm(wasm, timeline, motionSources),
    canonicalizeTimelineDocument: (timeline) =>
      canonicalizeTimelineDocumentWithWasm(wasm, timeline),
    compileMotionJsx: (source, compileOptions) =>
      compileMotionJsxWithWasm(wasm, source, compileOptions),
    compileMotionModules: (entry, modules, compileOptions) =>
      compileMotionModulesWithWasm(wasm, entry, modules, compileOptions),
    preparePreviewPackage: (input) => preparePreviewPackageWithWasm(wasm, input),
    rewriteMotionSource: (input) => rewriteMotionSourceWithWasm(wasm, input),
  };
}

export function rewriteMotionSourceWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "rewrite_motion_source">,
  input: MotionSourceEdit,
): MotionSourceEditResult {
  return JSON.parse(wasm.rewrite_motion_source(JSON.stringify(input))) as MotionSourceEditResult;
}

export function preparePreviewPackageWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "prepare_preview_package">,
  input: PreviewPackageInput,
): PreparedPreviewPackage {
  const original = input.resourceBytes ?? new Uint8Array();
  const chunks = [original];
  let length = original.byteLength;
  const ranges = new Map<Uint8Array, { offset: number; length: number }>();
  const motionInstances = input.motionInstances.map((instance) => ({
    ...instance,
    fonts: instance.fonts.map((font) => {
      let range = ranges.get(font.bytes);
      if (!range) {
        range = { offset: length, length: font.bytes.byteLength };
        ranges.set(font.bytes, range);
        chunks.push(font.bytes);
        length += font.bytes.byteLength;
      }
      return { bytes: range, role: font.role };
    }),
  }));
  const bytes = chunks.length === 1 ? original : new Uint8Array(length);
  if (chunks.length > 1) {
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
  }
  const [json, resourceBytes] = wasm.prepare_preview_package(
    JSON.stringify({
      authorTimeline: input.authorTimeline,
      resourceInputs: input.resourceInputs,
      motionInstances,
    }),
    bytes,
  );
  return { ...JSON.parse(json), resourceBytes } as PreparedPreviewPackage;
}

/** A checked view into the binary member; JSON contains ranges, never file contents. */
export function resourceByteRange(value: unknown, bytes: Uint8Array): Uint8Array {
  const range = value as { offset?: unknown; length?: unknown } | null;
  if (!range || !Number.isSafeInteger(range.offset) || !Number.isSafeInteger(range.length))
    throw new Error("invalid resource byte range");
  const offset = range.offset as number,
    length = range.length as number;
  if (offset < 0 || length < 0 || offset > bytes.byteLength - length)
    throw new Error("resource byte range exceeds binary member");
  return bytes.subarray(offset, offset + length);
}

/** Bounded local-property inspection; no layout or rendering. Run in a diagnostic worker. */
export async function sampleMotionProperties(
  options: TimelineCompilerRuntimeOptions,
  artifact: Record<string, unknown>,
  request: MotionPropertyRequest,
): Promise<MotionPropertySamples> {
  const engine = resolveEngineRuntimeAssets(options.runtimeAssets, options.runtimeBaseUrl);
  const wasm = await loadTimelineCompilerWasm(engine.glue, engine.wasm);
  return JSON.parse(
    wasm.sample_motion_properties(JSON.stringify(artifact), JSON.stringify(request)),
  ) as MotionPropertySamples;
}

export function compileMotionJsxWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "compile_motion_jsx">,
  source: string,
  options: MotionCompileOptions = {},
): CompiledMotion {
  const inputs = motionCompileInputs(options);
  return readMotionCompileResult(
    wasm.compile_motion_jsx(
      source,
      inputs.optionsJson,
      inputs.fonts,
      inputs.aliases,
      inputs.shaders,
      inputs.audioSamples,
    ),
  );
}

export function compileMotionModulesWithWasm(
  wasm: Pick<TimelineCompilerWasmModule, "compile_motion_modules">,
  entry: string,
  modules: Record<string, string>,
  options: MotionCompileOptions = {},
): CompiledMotion {
  const inputs = motionCompileInputs(options);
  return readMotionCompileResult(
    wasm.compile_motion_modules(
      entry,
      JSON.stringify(modules),
      inputs.optionsJson,
      inputs.fonts,
      inputs.aliases,
      inputs.shaders,
      inputs.audioSamples,
    ),
  );
}

function motionCompileInputs(options: MotionCompileOptions): {
  optionsJson: string;
  fonts: Uint8Array[];
  aliases: Array<[string, Uint8Array]>;
  shaders: MotionShaderPackage[];
  audioSamples: Float32Array[];
} {
  return {
    optionsJson: JSON.stringify({
      resources: options.resources ?? [],
      data: options.data ?? null,
      placement: options.placement ?? null,
      audioSources: (options.audioSources ?? []).map((source) => ({
        control: source.control,
        contentHash: source.contentHash,
        sampleRate: source.sampleRate,
      })),
    }),
    fonts: [...(options.fonts ?? [])],
    aliases: Object.entries(options.fontAliases ?? {}),
    shaders: [...(options.shaders ?? [])],
    audioSamples: (options.audioSources ?? []).map((source) => source.samples),
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
    warnings: result.warnings,
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
  wasm: Pick<TimelineCompilerWasmModule, "compile_timeline" | "timeline_document_view">,
  timeline: Timeline,
  motionSources: Record<string, MotionSourceMetadata> = {},
): CanonicalTimelineDocument {
  const timelineJson = wasm.compile_timeline(
    JSON.stringify(timeline),
    JSON.stringify(motionSources),
  );
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
    view: JSON.parse(
      wasm.timeline_document_view(timelineJson),
    ) as CanonicalTimelineDocument["view"],
  };
}

function loadTimelineCompilerWasm(
  moduleUrl: string,
  wasmUrl: string,
): Promise<TimelineCompilerWasmModule> {
  return loadEngineWasm(moduleUrl, wasmUrl) as Promise<TimelineCompilerWasmModule>;
}

export interface MotionPreparationInput {
  clipPath: string;
  component: string;
  locator: string;
  resources: Record<string, string>;
  data: unknown;
}
