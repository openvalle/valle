import transitionParameterSpecs from "../../../../../../crates/valle-timeline/schema/transition-parameters.generated.json";
import { drawSrgbPreviewCpu, srgbPreviewScratchBytes } from "./output-transform.ts";
import canvasKitPackage from "canvaskit-wasm/package.json";
import type {
  Canvas,
  CanvasKit,
  BlendMode,
  GrDirectContext,
  Font,
  Image,
  ImageFilter,
  Paint,
  Path,
  RuntimeEffect,
  Shader,
  Surface,
  Typeface,
  MallocObj,
} from "canvaskit-wasm";

import {
  BOUND_PROGRAM_SCHEDULES_ABI,
  RENDER_BINDINGS_ABI,
  RENDER_PLAN_ABI,
  decodePackedAbi,
  type PackedValue,
} from "../../abi/packed.ts";
import {
  applyDrawProgramPatch,
  decodeDrawProgram,
  type DrawProgramWire,
  type LinearColorWire,
  type RectWire,
} from "./draw-program.ts";
import { instanceOpacityBounds, programClipBounds, programNodeBounds } from "./program-bounds.ts";
import { clipPaintRectangleCoversOutline, paintRectangleBounds } from "./path-clip-proof.ts";
import { PathArenaCache } from "./path-arena-cache.ts";
import { RasterPathCache } from "./raster-path-cache.ts";
import { RasterPaintCache, type FillPaintLease } from "./raster-paint-cache.ts";
import { RasterTreeCache, type RasterTreeSignature } from "./raster-tree-cache.ts";
import { canvasKitGpuCacheBudget } from "./gpu-budget.ts";
import { drawPyramidFilter, drawPyramidFilterCooperative, pyramidWorkingBytes } from "./pyramid.ts";
import { drawLensDistortion } from "./lens.ts";
import { velocityBlurUniforms } from "./velocity.ts";
import type { GpuInstanceBatch, GpuInstances } from "../webgl/instances.ts";
import { cropProgramInputs } from "./filter-roi.ts";
import {
  CanvasKitBuiltinRuntime,
  TRANSITION_KERNELS,
  type BuiltinKernel,
} from "./builtin-runtime.ts";

type Wire = Record<string, any>;
const IDENTITY_MATRIX = [1, 0, 0, 0, 1, 0, 0, 0, 1];
const RASTER_LAYER_CACHE_BYTES = 64 * 1024 * 1024;

export interface CanvasKitExternalObject {
  /** Exact ResourceKey from the request fulfilled for this handle. */
  readonly key: PackedValue;
  readonly kind: "visual" | "dataTexture" | "font" | "runtimeShader" | "scene3d" | "frame";
  readonly image?: Image;
  readonly bytes?: Uint8Array;
}

export interface CanvasKitObjectTable {
  readonly generation: bigint | number;
  readonly objects: ReadonlyMap<number, CanvasKitExternalObject>;
}

export interface CanvasKitExecutionTarget {
  /** Intermediate snapshots share the Ganesh context; submit at the frame boundary. */
  readonly deferGpuFlush?: boolean;
  /** Restrict path coverage layers to their conservative contour hull. */
  readonly boundedPathClipLayers?: boolean;
  readonly fuseClipPaints?: boolean;
  readonly retainRasterTrees?: boolean;
  readonly geometryRenderer?: GpuInstances | null;
  readonly surface: Surface;
  /** GPU context used for working F16 surfaces. Null selects CanvasKit's CPU surface factory. */
  readonly directContext?: GrDirectContext | null;
  /** Same immutable limits admitted by Engine lowering for this frame. */
  readonly maxSurfaceBytes?: bigint;
  readonly maxFrameBytes?: bigint;
  /** Optional diagnostics for materialized passes; GPU times measure submission, not completion. */
  readonly onPassTiming?: (timing: CanvasKitPassTiming) => void;
  readonly onLocalPassTiming?: (timing: CanvasKitPassTiming) => void;
  readonly onRasterLeafTiming?: (kind: string, milliseconds: number) => void;
  readonly onProgramAdmissionStageTiming?: (kind: string, milliseconds: number) => void;
  /** Preview may yield between complete local passes; resources remain frame-owned. */
  readonly yieldToPresentation?: () => Promise<void>;
  /**
   * Lends bytes beyond the frame's planned envelope: to keep surfaces the frame uses again, which
   * are evicted instead when it refuses, and for live surfaces beyond the plan, which then fail the
   * frame. Without a lender the pool evicts, and exceeds the envelope up to `maxFrameBytes` only
   * for live surfaces.
   */
  readonly borrowSurfaceBytes?: (bytes: bigint) => boolean;
}

/** Planning needs the working-surface backend, not the target or external objects. */
export interface CanvasKitPlanOptions {
  /** Working F16 surfaces live on a GPU context; otherwise on CanvasKit's CPU surface factory. */
  readonly gpu: boolean;
  readonly geometryRenderer?: GpuInstances | null;
  /** Same immutable limits admitted by Engine lowering for this frame. */
  readonly maxSurfaceBytes?: bigint;
  readonly maxFrameBytes?: bigint;
  readonly onProgramAdmissionStageTiming?: (kind: string, milliseconds: number) => void;
}

/**
 * A frame admitted up to its external objects, with the working memory its execution needs.
 * A runtime admits both byte counts before it fulfills the frame's resources, then runs it once.
 */
export interface CanvasKitFramePlan {
  /**
   * Peak bytes of working F16 surfaces: the slots the plan holds live, the transients this
   * executor adds to a pass, and the output. The frame's live surfaces stay within it; the pool
   * keeps others beside them only in bytes the budget lends.
   */
  readonly surfaceBytes: bigint;
  /**
   * Peak bytes of images held beside the surface pool: a GPU surface copied on write while its
   * snapshot is read, CPU snapshots and kernel buffers, and raster layers the frame may retain.
   */
  readonly imageBytes: bigint;
}

export interface CanvasKitPassTiming {
  readonly kind: string;
  readonly width: number;
  readonly height: number;
  readonly drawMs: number;
  readonly snapshotMs: number;
}

/** Canonical Rust/Wasm kernels owned by ProductEngine. */
export interface CanvasKitEngineKernels {
  transform_srgb_preview_pixels(
    pixels: Float32Array,
    opaque: boolean,
  ): void | Float32Array | Promise<Float32Array>;
  transform_srgb_preview_f16?(
    pixels: Uint8Array,
    opaque: boolean,
  ): void | Uint8Array | Promise<Uint8Array>;
  plan_pyramid_filter?(
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    filterJson: string,
  ): Float64Array;
  apply_bloom_f16?(
    input: Uint8Array,
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    threshold: number,
    knee: number,
    intensity: number,
    radius: number,
  ): Uint8Array | Promise<Uint8Array>;
  apply_glow_f16?(
    input: Uint8Array,
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    colorJson: string,
    intensity: number,
    radius: number,
  ): Uint8Array | Promise<Uint8Array>;
  plan_radial_blur?(
    amount: number,
    inputX: number,
    inputY: number,
    inputWidth: number,
    inputHeight: number,
    outputX: number,
    outputY: number,
    outputWidth: number,
    outputHeight: number,
  ): Float64Array;
  apply_radial_blur_f16?(
    input: Uint8Array,
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    originX: number,
    originY: number,
    centerX: number,
    centerY: number,
    amount: number,
  ): Uint8Array | Promise<Uint8Array>;
  apply_film_grain_f16?(
    input: Uint8Array,
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    originX: number,
    originY: number,
    seed: number,
    amount: number,
    size: number,
  ): Uint8Array | Promise<Uint8Array>;
  apply_lens_distortion_f16?(
    input: Uint8Array,
    inputWidth: number,
    inputHeight: number,
    outputWidth: number,
    outputHeight: number,
    offsetX: number,
    offsetY: number,
    originX: number,
    originY: number,
    k1: number,
    k2: number,
    frameX: number,
    frameY: number,
    frameWidth: number,
    frameHeight: number,
  ): Uint8Array | Promise<Uint8Array>;
  perlin_noise_tables?(seed: number): Uint8Array;
}

export interface CanvasKitGlyphCoverageProfile {
  readonly rasterizer: "freetype";
  readonly library: "renderer-embedded";
  readonly hinting: "none";
  readonly edging: "antialias";
  readonly subpixelPositioning: false;
}

export const CANVASKIT_GLYPH_COVERAGE_PROFILE: CanvasKitGlyphCoverageProfile = Object.freeze({
  rasterizer: "freetype",
  library: "renderer-embedded",
  hinting: "none",
  edging: "antialias",
  subpixelPositioning: false,
});

export interface CanvasKitExecutionProfile {
  readonly surfaceBackend: "canvaskit-cpu" | "canvaskit-gpu";
  readonly glyphCoverage: CanvasKitGlyphCoverageProfile;
  readonly canvasKitVersion: string;
}

/** Surface pool roles: the presented output, outer pass slots, program slots and transients. */
export type CanvasKitSurfaceKind = "output" | "outer" | "program" | "transient";

export interface CanvasKitExecutionReport {
  readonly profile: CanvasKitExecutionProfile;
  readonly passes: number;
  readonly programs: number;
  readonly physicalSurfaces: number;
  readonly maximumLiveImages: number;
  readonly surfaceAllocations: number;
  readonly surfaceReuses: number;
  readonly surfaceAllocatedBytes: bigint;
  readonly surfaceResidentBytes: bigint;
  readonly surfacePeakBytes: bigint;
  /** The schedule's working-surface peak for this frame, against which `surfacePeakBytes` is measured. */
  readonly plannedSurfacePeakBytes: bigint;
  /** Resident bytes by role at the moment of `surfacePeakBytes`. */
  readonly surfacePeakBytesByKind: Readonly<Record<CanvasKitSurfaceKind, bigint>>;
  /** The admitted working-surface envelope, `CanvasKitFramePlan.surfaceBytes`. */
  readonly surfaceEnvelopeBytes: bigint;
  /** Live working-surface bytes beyond the envelope at their highest point. */
  readonly surfaceOverflowBytes: bigint;
  /** The admitted `CanvasKitFramePlan.imageBytes`. */
  readonly heldImageBytes: bigint;
  readonly surfacePoolEvictions: number;
  /** Ganesh resource cache after the frame, including pinned surfaces; zero without a GPU context. */
  readonly gpuCacheUsageBytes: number;
  readonly gpuCacheLimitBytes: number;
  /** Circle batch targets held outside the surface pool. */
  readonly geometryTargetBytes: number;
  readonly programCacheHits: number;
  readonly programCacheMisses: number;
  readonly fontCacheHits: number;
  readonly fontCacheMisses: number;
  readonly shaderCacheHits: number;
  readonly shaderCacheMisses: number;
  readonly rasterLayerCacheHits: number;
  readonly rasterLayerCacheMisses: number;
  readonly rasterLayerCacheEntries: number;
  readonly rasterLayerCacheBytes: number;
  readonly packetAdmissionMs: number;
  readonly programAdmissionMs: number;
  readonly scheduleAdmissionMs: number;
  readonly renderMs: number;
  readonly programLocalPasses: number;
  readonly directRasterPrograms: number;
  readonly programGroups: number;
  readonly programTransformGroups: number;
  readonly programClipGroups: number;
  readonly pathClipElisions: number;
  readonly programOpacityGroups: number;
  readonly programFilterGroups: number;
  readonly programMaskGroups: number;
  readonly programShaderGroups: number;
  readonly programBackdropGroups: number;
  readonly programBlendGroups: number;
  readonly committed: true;
}

export class CanvasKitCompositorError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(`[${code}] ${message}`);
    this.name = "CanvasKitCompositorError";
  }
}

/** A program admitted up to its external objects. */
interface ProgramStructure {
  readonly plan: Wire;
  readonly draw: DrawProgramWire;
  readonly directRaster: DirectRasterPlan | null;
}

interface AdmittedProgram extends ProgramStructure {
  readonly pathCache?: RasterPathCache;
  readonly paintCache?: RasterPaintCache;
  readonly textures: Map<string, CanvasKitExternalObject>;
  readonly fonts: Map<string, Typeface>;
  readonly shaders: Map<string, RuntimeEffect>;
  readonly scenes: Map<string, CanvasKitExternalObject>;
}

interface DirectRasterPlan {
  readonly roots: readonly number[];
  readonly layerBounds: ReadonlyMap<number, RectWire>;
  readonly backdrops: ReadonlyMap<number, DirectRasterBackdrop>;
  /** A layer blurs or drops a shadow, so the device matrix must be scale/translate. */
  readonly spatialFilters: boolean;
}

interface DirectRasterBackdrop {
  readonly destination: number | null;
  readonly hasLocalInputs: boolean;
}

export interface ImageValue {
  readonly image: Image | null;
  readonly owned: boolean;
  readonly roi: DeviceRoi;
}

interface BoundProgramRuntimeSchedule {
  readonly resourceRois: Map<number, DeviceRoi>;
  readonly slotExtents: Map<number, { width: number; height: number } | null>;
  readonly passes: readonly BoundProgramExecutionPass[];
  readonly estimatedSurfaceBytes: bigint;
}

/** How a program runs in this frame, decided while the frame is planned. */
interface ProgramRun {
  /** The bound schedule with its exclusively consumed device-space producers cropped. */
  readonly bound: BoundProgramRuntimeSchedule;
  readonly direct: boolean;
  readonly device: number[];
}

interface BoundProgramExecutionPass {
  readonly output: number;
  readonly deviceRoi: DeviceRoi;
  readonly storage: Wire;
  readonly retireAfter: readonly number[];
}

export interface DeviceRoi {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

interface ExecutorCacheCounters {
  programHits: number;
  programMisses: number;
  fontHits: number;
  fontMisses: number;
  shaderHits: number;
  shaderMisses: number;
  rasterHits: number;
  rasterMisses: number;
  rasterEntries: number;
  rasterBytes: number;
}

function emptyExecutorCacheCounters(): ExecutorCacheCounters {
  return {
    programHits: 0,
    programMisses: 0,
    fontHits: 0,
    fontMisses: 0,
    shaderHits: 0,
    shaderMisses: 0,
    rasterHits: 0,
    rasterMisses: 0,
    rasterEntries: 0,
    rasterBytes: 0,
  };
}

function cacheCounterReport(
  before: ExecutorCacheCounters,
  after: ExecutorCacheCounters,
): Pick<
  CanvasKitExecutionReport,
  | "programCacheHits"
  | "programCacheMisses"
  | "fontCacheHits"
  | "fontCacheMisses"
  | "shaderCacheHits"
  | "shaderCacheMisses"
  | "rasterLayerCacheHits"
  | "rasterLayerCacheMisses"
  | "rasterLayerCacheEntries"
  | "rasterLayerCacheBytes"
> {
  return {
    programCacheHits: after.programHits - before.programHits,
    programCacheMisses: after.programMisses - before.programMisses,
    fontCacheHits: after.fontHits - before.fontHits,
    fontCacheMisses: after.fontMisses - before.fontMisses,
    shaderCacheHits: after.shaderHits - before.shaderHits,
    shaderCacheMisses: after.shaderMisses - before.shaderMisses,
    rasterLayerCacheHits: after.rasterHits - before.rasterHits,
    rasterLayerCacheMisses: after.rasterMisses - before.rasterMisses,
    rasterLayerCacheEntries: after.rasterEntries,
    rasterLayerCacheBytes: after.rasterBytes,
  };
}

function touch<K, V>(values: Map<K, V>, key: K, value: V): void {
  values.delete(key);
  values.set(key, value);
}

function trimPlainCache<K, V>(values: Map<K, V>, maximum: number): void {
  while (values.size > maximum) {
    const oldest = values.keys().next();
    if (oldest.done) return;
    values.delete(oldest.value);
  }
}

function trimDeleteableCache<K, V extends { delete(): void }>(
  values: Map<K, V>,
  maximum: number,
): void {
  while (values.size > maximum) {
    const oldest = values.entries().next();
    if (oldest.done) return;
    oldest.value[1].delete();
    values.delete(oldest.value[0]);
  }
}

/** Long-lived Web executor. Built-in kernels and F16 working surfaces are reused across frames. */
export class CanvasKitExecutor {
  readonly pathArenas = new PathArenaCache();
  readonly paths = new RasterPathCache();
  readonly paints = new RasterPaintCache();
  readonly trees = new RasterTreeCache();
  readonly builtins: CanvasKitBuiltinRuntime;
  private workingArena: SurfaceArena | null = null;
  private readonly drawPrograms = new Map<string, Promise<DrawProgramWire>>();
  private readonly baselinePrograms = new Map<string, Promise<Uint8Array>>();
  private readonly planTemplates = new Map<
    string,
    { packet: Uint8Array; admitted: Promise<Wire> }
  >();
  private readonly typefaces = new Map<string, Typeface>();
  private readonly runtimeShaders = new Map<string, RuntimeEffect>();
  private readonly rasterLayers = new Map<
    string,
    { image: Image; roi: DeviceRoi; bytes: number }
  >();
  private cacheCounters = emptyExecutorCacheCounters();

  constructor(
    readonly CanvasKit: CanvasKit,
    readonly engineKernels: CanvasKitEngineKernels | null = null,
  ) {
    this.builtins = new CanvasKitBuiltinRuntime(CanvasKit);
  }

  async execute(
    planPacket: ArrayBuffer | ArrayBufferView,
    bindingPacket: ArrayBuffer | ArrayBufferView,
    schedulePacket: ArrayBuffer | ArrayBufferView,
    objectTable: CanvasKitObjectTable,
    target: CanvasKitExecutionTarget,
  ): Promise<CanvasKitExecutionReport> {
    const plan = await this.plan(planPacket, bindingPacket, schedulePacket, {
      gpu: target.directContext != null,
      geometryRenderer: target.geometryRenderer,
      maxSurfaceBytes: target.maxSurfaceBytes,
      maxFrameBytes: target.maxFrameBytes,
      onProgramAdmissionStageTiming: target.onProgramAdmissionStageTiming,
    });
    return this.run(plan, objectTable, target);
  }

  /** Admit a frame's packets, programs and schedule, and plan the working memory it needs. */
  plan(
    planPacket: ArrayBuffer | ArrayBufferView,
    bindingPacket: ArrayBuffer | ArrayBufferView,
    schedulePacket: ArrayBuffer | ArrayBufferView,
    options: CanvasKitPlanOptions,
  ): Promise<CanvasKitFramePlan> {
    return planCanvasKitFrame(this, planPacket, bindingPacket, schedulePacket, options);
  }

  /** Execute a planned frame once its external objects are fulfilled. A plan runs once. */
  run(
    plan: CanvasKitFramePlan,
    objectTable: CanvasKitObjectTable,
    target: CanvasKitExecutionTarget,
  ): Promise<CanvasKitExecutionReport> {
    return runCanvasKitFrame(this, plan, objectTable, target);
  }

  /** Working surfaces the pool keeps between frames; the next frame reuses them first. */
  get workingSurfaceBytes(): bigint {
    return this.workingArena?.residentBytes ?? 0n;
  }

  arena(
    context: GrDirectContext | null,
    width: number,
    height: number,
    maxSurfaceBytes: bigint,
    maxFrameBytes: bigint,
    frame: SurfaceFrame = { envelopeBytes: maxFrameBytes, borrow: null, liveness: null },
  ): SurfaceArena {
    if (!this.workingArena?.matches(context, width, height, maxSurfaceBytes, maxFrameBytes)) {
      this.clearRasterLayers();
      this.workingArena?.delete();
      this.workingArena = new SurfaceArena(
        this.CanvasKit,
        context,
        width,
        height,
        maxSurfaceBytes,
        maxFrameBytes,
      );
    }
    this.workingArena.beginFrame(frame);
    return this.workingArena;
  }

  /** Retire optional frame storage while keeping the context, programs and shader history. */
  trimWorkingMemory(): void {
    this.clearRasterLayers();
    this.workingArena?.delete();
    this.workingArena = null;
  }

  dispose(): void {
    this.pathArenas.clear();
    this.paths.dispose();
    this.paints.dispose();
    this.trees.dispose();
    this.clearRasterLayers();
    this.workingArena?.delete();
    this.workingArena = null;
    this.builtins.dispose();
    for (const face of this.typefaces.values()) face.delete();
    for (const effect of this.runtimeShaders.values()) effect.delete();
    this.drawPrograms.clear();
    this.baselinePrograms.clear();
    this.planTemplates.clear();
    this.typefaces.clear();
    this.runtimeShaders.clear();
  }

  async drawProgram(key: string, packed: Uint8Array): Promise<DrawProgramWire> {
    const cached = this.drawPrograms.get(key);
    if (cached) {
      this.cacheCounters.programHits += 1;
      touch(this.drawPrograms, key, cached);
      return cached;
    }
    this.cacheCounters.programMisses += 1;
    const pending = decodeDrawProgram(packed, this.pathArenas);
    this.drawPrograms.set(key, pending);
    try {
      return await pending;
    } catch (error) {
      this.drawPrograms.delete(key);
      throw error;
    }
  }

  async baselineProgram(key: string, packed: Uint8Array): Promise<Uint8Array> {
    const cached = this.baselinePrograms.get(key);
    if (cached) {
      touch(this.baselinePrograms, key, cached);
      return cached;
    }
    // Decoding the template-owned baseline once also primes the exact DrawProgram cache when the
    // baseline is itself the requested frame.
    const pending = this.drawProgram(key, packed).then(() => packed);
    this.baselinePrograms.set(key, pending);
    try {
      return await pending;
    } catch (error) {
      this.baselinePrograms.delete(key);
      throw error;
    }
  }

  async planTemplate(key: string, packetInput: ArrayBuffer | ArrayBufferView): Promise<Wire> {
    const packet = exactPacketBytes(packetInput);
    const cached = this.planTemplates.get(key);
    if (
      cached &&
      cached.packet.buffer === packet.buffer &&
      cached.packet.byteOffset === packet.byteOffset &&
      cached.packet.byteLength === packet.byteLength
    ) {
      touch(this.planTemplates, key, cached);
      return cached.admitted;
    }
    const admitted = decodePackedAbi(packet, RENDER_PLAN_ABI).then((value) =>
      ownBaselines(record(value, "RenderPlanTemplate")),
    );
    const entry = { packet, admitted };
    this.planTemplates.set(key, entry);
    try {
      return await admitted;
    } catch (error) {
      this.planTemplates.delete(key);
      throw error;
    }
  }

  typeface(key: string, faceIndex: number, bytes: Uint8Array): Typeface {
    // CanvasKit 0.41 does not expose SkFontArguments::CollectionIndex: this API always decodes
    // face zero. Engine admission rejects non-zero collection faces for the Native/Web common
    // profile; keep this trust boundary fail-closed for foreign or stale render-plan packets too.
    if (faceIndex !== 0) {
      fail("font_admission", `font '${key}' requests unsupported collection face ${faceIndex}`);
    }
    const cached = this.typefaces.get(key);
    if (cached) {
      this.cacheCounters.fontHits += 1;
      touch(this.typefaces, key, cached);
      return cached;
    }
    this.cacheCounters.fontMisses += 1;
    const face = this.CanvasKit.Typeface.MakeFreeTypeFaceFromData(bytes.slice().buffer);
    if (!face) fail("font_admission", `font '${key}' cannot be decoded`);
    this.typefaces.set(key, face);
    return face;
  }

  runtimeShader(key: string, bytes: Uint8Array): RuntimeEffect {
    const cached = this.runtimeShaders.get(key);
    if (cached) {
      this.cacheCounters.shaderHits += 1;
      touch(this.runtimeShaders, key, cached);
      return cached;
    }
    this.cacheCounters.shaderMisses += 1;
    let detail = "";
    const effect = this.CanvasKit.RuntimeEffect.Make(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
      (message) => {
        detail = message;
      },
    );
    if (!effect) fail("shader_admission", `shader '${key}' failed: ${detail}`);
    this.runtimeShaders.set(key, effect);
    return effect;
  }

  cacheSnapshot(): ExecutorCacheCounters {
    return { ...this.cacheCounters };
  }

  rasterLayer(key: string): ImageValue | null {
    const cached = this.rasterLayers.get(key);
    if (!cached) {
      this.cacheCounters.rasterMisses += 1;
      return null;
    }
    this.cacheCounters.rasterHits += 1;
    touch(this.rasterLayers, key, cached);
    return { image: cached.image, owned: false, roi: cached.roi };
  }

  /** Bytes of the raster layers kept for later frames. */
  get rasterLayerBytes(): number {
    return this.cacheCounters.rasterBytes;
  }

  retainRasterLayer(key: string, value: ImageValue): boolean {
    if (!value.owned || !value.image) return false;
    const bytes = value.roi.width * value.roi.height * 8;
    if (bytes > RASTER_LAYER_CACHE_BYTES) return false;
    while (
      this.rasterLayers.size >= 64 ||
      this.cacheCounters.rasterBytes + bytes > RASTER_LAYER_CACHE_BYTES
    ) {
      const oldest = this.rasterLayers.entries().next().value;
      if (!oldest) break;
      oldest[1].image.delete();
      this.rasterLayers.delete(oldest[0]);
      this.cacheCounters.rasterBytes -= oldest[1].bytes;
    }
    this.rasterLayers.set(key, { image: value.image, roi: value.roi, bytes });
    this.cacheCounters.rasterEntries = this.rasterLayers.size;
    this.cacheCounters.rasterBytes += bytes;
    return true;
  }

  private clearRasterLayers(): void {
    for (const value of this.rasterLayers.values()) value.image.delete();
    this.rasterLayers.clear();
    this.cacheCounters.rasterEntries = 0;
    this.cacheCounters.rasterBytes = 0;
  }

  trimCaches(): void {
    trimDeleteableCache(this.typefaces, 256);
    trimDeleteableCache(this.runtimeShaders, 128);
    trimPlainCache(this.drawPrograms, 256);
    trimPlainCache(this.baselinePrograms, 64);
    trimPlainCache(this.planTemplates, 64);
  }
}

/**
 * Sealed Web Product Compositor executor.
 *
 * Both binary packets and every external object are admitted before a working surface is
 * allocated. Rendering happens in private slot-pooled surfaces and the caller's target receives
 * one final image only after the complete plan succeeds.
 */
export async function executeCanvasKitCompositor(
  CanvasKit: CanvasKit,
  planPacket: ArrayBuffer | ArrayBufferView,
  bindingPacket: ArrayBuffer | ArrayBufferView,
  schedulePacket: ArrayBuffer | ArrayBufferView,
  objectTable: CanvasKitObjectTable,
  target: CanvasKitExecutionTarget,
): Promise<CanvasKitExecutionReport> {
  const executor = new CanvasKitExecutor(CanvasKit);
  try {
    return await executor.execute(planPacket, bindingPacket, schedulePacket, objectTable, target);
  } finally {
    executor.dispose();
  }
}

/** What a frame plan carries to its single run. */
interface PlannedFrame {
  readonly executor: CanvasKitExecutor;
  readonly options: CanvasKitPlanOptions;
  readonly plan: Wire;
  readonly bindings: Wire;
  readonly externalSlots: unknown[];
  readonly structures: Map<number, ProgramStructure>;
  readonly runs: Map<number, ProgramRun>;
  readonly dynamic: Map<number, Wire>;
  readonly extent: { width: number; height: number };
  readonly boundSchedule: BoundOuterSchedule;
  readonly liveness: SurfaceLiveness;
  readonly maxSurfaceBytes: bigint;
  readonly maxFrameBytes: bigint;
  readonly dataTextureBytes: bigint;
  readonly plannedSurfacePeakBytes: bigint;
  readonly cacheBefore: ExecutorCacheCounters;
  readonly packetAdmissionMs: number;
  readonly programAdmissionMs: number;
  readonly scheduleAdmissionMs: number;
  readonly programDiagnostics: ReturnType<typeof inspectProgramDiagnostics>;
}

const plannedFrames = new WeakMap<CanvasKitFramePlan, PlannedFrame>();

async function planCanvasKitFrame(
  executor: CanvasKitExecutor,
  planPacket: ArrayBuffer | ArrayBufferView,
  bindingPacket: ArrayBuffer | ArrayBufferView,
  schedulePacket: ArrayBuffer | ArrayBufferView,
  options: CanvasKitPlanOptions,
): Promise<CanvasKitFramePlan> {
  const { builtins } = executor;
  const packetAdmissionStarted = performance.now();
  executor.trimCaches();
  const cacheBefore = executor.cacheSnapshot();
  const [bindingsValue, scheduleValue] = await Promise.all([
    decodePackedAbi(bindingPacket, RENDER_BINDINGS_ABI),
    decodePackedAbi(schedulePacket, BOUND_PROGRAM_SCHEDULES_ABI),
  ]);
  const bindings = record(bindingsValue, "RenderBindings");
  const schedule = record(scheduleValue, "BoundProgramSchedules");
  const templateHash = digestString(bindings.templateHash, "bindings.templateHash");
  const plan = await executor.planTemplate(templateHash, planPacket);
  validatePacketTriple(plan, schedule, templateHash);
  const externalSlots = array(
    record(plan.bindingLayout, "plan.bindingLayout").externalSlots,
    "externalSlots",
  );

  const structures = new Map<number, ProgramStructure>();
  const packetAdmissionMs = performance.now() - packetAdmissionStarted;
  const programAdmissionStarted = performance.now();
  const programLayouts = array(plan.programs, "plan.programs");
  const framePrograms = array(bindings.programs, "bindings.programs");
  if (framePrograms.length !== programLayouts.length) {
    fail("program_layout", "frame program count does not match the template layout");
  }
  const programContracts: Wire[] = [];
  for (let index = 0; index < programLayouts.length; index += 1) {
    const layout = record(programLayouts[index], `plan.programs[${index}]`);
    const frameProgram = record(framePrograms[index], `bindings.programs[${index}]`);
    const contract = admitFrameProgramLayout(layout, frameProgram, index);
    const structure = await admitProgramStructure(
      executor,
      contract,
      options.onProgramAdmissionStageTiming,
    );
    programContracts.push(structure.plan);
    const id = positiveId(structure.plan.id, `bindings.programs[${index}].id`);
    if (structures.has(id)) fail("duplicate_program", `program ${id} is duplicated`);
    structures.set(id, structure);
  }

  const programAdmissionMs = performance.now() - programAdmissionStarted;
  const programDiagnostics = inspectProgramDiagnostics(structures);
  const scheduleAdmissionStarted = performance.now();
  admitCanvasKitOutput(plan);
  const requiredKernels = requiredBuiltinKernels(plan, programContracts);
  requiredKernels.add("normalizeInput");
  builtins.admit(requiredKernels);

  const extent = renderExtent(plan);
  const dynamic = dynamicBindings(bindings);
  preflightEffectDomains(plan, dynamic, extent);
  const boundSchedule = admitBoundOuterSchedule(plan, programContracts, schedule, dynamic, extent);
  const rootBytes = BigInt(extent.width) * BigInt(extent.height) * 8n;
  const maxSurfaceBytes = options.maxSurfaceBytes ?? maxBigInt(rootBytes, 64n * 1024n * 1024n);
  const maxFrameBytes =
    options.maxFrameBytes ?? maxBigInt(maxSurfaceBytes * 16n, 512n * 1024n * 1024n);
  let dataTextureBytes = 0n;
  for (const raw of externalSlots) {
    const expected = record(record(raw, "external slot").expected, "external descriptor");
    if (expected.kind !== "dataTexture") continue;
    const size = record(expected.extent, "data texture extent");
    const bytes =
      BigInt(positiveId(size.width, "data texture width")) *
      BigInt(positiveId(size.height, "data texture height")) *
      8n;
    if (bytes > maxSurfaceBytes)
      fail("resource_budget", "data texture exceeds the surface byte budget");
    dataTextureBytes += bytes;
  }
  const plannedSurfacePeakBytes = BigInt(
    positiveIdOrZero(schedule.estimatedPeakSurfaceBytes, "frame surface bytes"),
  );
  if (dataTextureBytes + plannedSurfacePeakBytes > maxFrameBytes) {
    fail("resource_budget", "data textures and working surfaces exceed the frame byte budget");
  }
  const runs = planProgramRuns(
    executor,
    plan,
    structures,
    boundSchedule,
    dynamic,
    options.geometryRenderer ?? null,
  );
  const working = frameWorkingSet(
    executor,
    plan,
    structures,
    runs,
    boundSchedule,
    dynamic,
    extent,
    options.gpu,
  );
  if (working.surfaceBytes > maxFrameBytes - dataTextureBytes) {
    fail(
      "resource_budget",
      `the frame's working surfaces need ${working.surfaceBytes} bytes beside ${dataTextureBytes} bytes of data textures, budget ${maxFrameBytes}`,
    );
  }
  const framePlan: CanvasKitFramePlan = Object.freeze({
    surfaceBytes: working.surfaceBytes,
    imageBytes: working.imageBytes,
  });
  plannedFrames.set(framePlan, {
    executor,
    options,
    plan,
    bindings,
    externalSlots,
    structures,
    runs,
    dynamic,
    extent,
    boundSchedule,
    liveness: working.liveness,
    maxSurfaceBytes,
    maxFrameBytes,
    dataTextureBytes,
    plannedSurfacePeakBytes,
    cacheBefore,
    packetAdmissionMs,
    programAdmissionMs,
    scheduleAdmissionMs: performance.now() - scheduleAdmissionStarted,
    programDiagnostics,
  });
  return framePlan;
}

async function runCanvasKitFrame(
  executor: CanvasKitExecutor,
  framePlan: CanvasKitFramePlan,
  objectTable: CanvasKitObjectTable,
  target: CanvasKitExecutionTarget,
): Promise<CanvasKitExecutionReport> {
  const frame = plannedFrames.get(framePlan);
  if (!frame || frame.executor !== executor)
    fail("frame_plan", "frame plan already ran or belongs to another executor");
  plannedFrames.delete(framePlan);
  const {
    options,
    plan,
    bindings,
    extent,
    dynamic,
    boundSchedule,
    runs,
    maxSurfaceBytes,
    maxFrameBytes,
    dataTextureBytes,
  } = frame;
  if (
    (target.directContext != null) !== options.gpu ||
    (target.geometryRenderer ?? null) !== (options.geometryRenderer ?? null) ||
    target.maxSurfaceBytes !== options.maxSurfaceBytes ||
    target.maxFrameBytes !== options.maxFrameBytes
  ) {
    fail("frame_plan", "execution target differs from the one the frame was planned for");
  }
  const { CanvasKit, builtins } = executor;
  const bindingStarted = performance.now();
  const generation = exactPositiveInteger(
    bindings.externalGeneration,
    "bindings.externalGeneration",
  );
  if (generation !== BigInt(objectTable.generation)) {
    fail(
      "stale_generation",
      `binding generation ${generation} does not match object table ${objectTable.generation}`,
    );
  }

  const externalIds = array(bindings.externalIds, "bindings.externalIds").map((value, index) =>
    positiveId(value, `bindings.externalIds[${index}]`),
  );
  const externalSlots = frame.externalSlots;
  if (externalIds.length !== externalSlots.length)
    fail("binding_layout", "external slot count mismatch");
  const externalBySlot = new Map<number, CanvasKitExternalObject>();
  for (let index = 0; index < externalIds.length; index += 1) {
    const slot = record(externalSlots[index], `externalSlots[${index}]`);
    const slotId = positiveId(slot.id, `externalSlots[${index}].id`);
    if (slotId !== index + 1) fail("binding_layout", "external slots are not canonical");
    const object = objectTable.objects.get(externalIds[index]!);
    if (!object) fail("missing_external", `external handle ${externalIds[index]} is absent`);
    if (!deepEqual(object.key, slot.key))
      fail("external_contract", `external slot ${slotId} key mismatch`);
    const expected = record(slot.expected, "external descriptor");
    if (expected.kind === "dataTexture") {
      const size = record(expected.extent, "data texture extent");
      const width = positiveId(size.width, "data texture width"),
        height = positiveId(size.height, "data texture height");
      if (
        width > 4096 ||
        height > 4096 ||
        width * height * 8 > 64 * 1024 * 1024 ||
        object.kind !== "dataTexture" ||
        !object.image ||
        object.image.width() !== width * 2 ||
        object.image.height() !== height * 4 ||
        object.image.getImageInfo().colorType !== CanvasKit.ColorType.Alpha_8
      ) {
        fail("external_contract", "data texture storage does not match its admitted descriptor");
      }
    } else if (object.kind === "dataTexture")
      fail("external_contract", "data texture has a non-data descriptor");
    externalBySlot.set(slotId, object);
  }
  const programs = new Map<number, AdmittedProgram>();
  for (const [id, structure] of frame.structures) {
    programs.set(
      id,
      bindProgramResources(
        executor,
        structure,
        externalBySlot,
        options.onProgramAdmissionStageTiming,
      ),
    );
  }
  const programAdmissionMs = frame.programAdmissionMs + performance.now() - bindingStarted;

  preflightTarget(target.surface, extent);
  const arena = executor.arena(
    target.directContext ?? null,
    extent.width,
    extent.height,
    maxSurfaceBytes,
    maxFrameBytes - dataTextureBytes,
    {
      envelopeBytes: framePlan.surfaceBytes,
      borrow: target.borrowSurfaceBytes ?? null,
      liveness: frame.liveness,
    },
  );
  arena.geometryRenderer = target.geometryRenderer ?? null;
  arena.onLocalPassTiming = target.onLocalPassTiming ?? null;
  arena.onRasterLeafTiming = target.onRasterLeafTiming ?? null;
  arena.trees = target.retainRasterTrees && arena.gpu ? executor.trees : null;
  arena.yieldToPresentation = target.yieldToPresentation ?? null;
  arena.deferGpuFlush = target.deferGpuFlush === true;
  arena.boundedPathClipLayers = target.boundedPathClipLayers === true;
  arena.fuseClipPaints = target.fuseClipPaints === true;
  arena.geometryRenderer?.beginFrame(target.borrowSurfaceBytes);
  const resources = new Map<number, ImageValue>();
  const externalResourceById = new Map<number, CanvasKitExternalObject>();
  for (const raw of array(plan.resources, "plan.resources")) {
    const resource = record(raw, "plan resource");
    const kind = record(resource.kind, "plan resource kind");
    if (kind.kind === "external") {
      externalResourceById.set(
        positiveId(resource.id, "resource.id"),
        required(externalBySlot, positiveId(kind.slot, "resource.slot"), "external slot"),
      );
    }
  }
  const runProgram = async (
    id: number,
    destinations: ImageValue[],
    device: number[],
  ): Promise<ImageValue> => {
    arena.activeProgram = id;
    try {
      return await executeProgram(
        CanvasKit,
        builtins,
        arena,
        executor.engineKernels,
        required(programs, id, "program"),
        required(runs, id, "program run"),
        destinations,
        device,
      );
    } finally {
      arena.activeProgram = 0;
    }
  };

  let maximumLiveImages = 0;
  let output: Image | null = null;
  const renderStarted = performance.now();
  try {
    for (const [index, rawPass] of array(plan.passes, "plan.passes").entries()) {
      const pass = record(rawPass, `passes[${index}]`);
      if (positiveId(pass.id, `passes[${index}].id`) !== index + 1)
        fail("pass_order", "pass ids are not canonical");
      const kind = record(pass.kind, `passes[${index}].kind`);
      const step = boundSchedule.passes[index]!;
      const outputId = step.output;
      arena.outerPass = index + 1;
      if (kind.kind === "bindBackdropView" || kind.kind === "aliasResource") {
        resources.set(outputId, borrowedValue(resourceValue(resources, kind.input)));
        retireOuterResources(step.retireAfter, resources);
        maximumLiveImages = Math.max(maximumLiveImages, liveImages(resources));
        continue;
      }
      const outputRoi = step.deviceRoi;
      if (outputRoi.width === 0 || outputRoi.height === 0) {
        resources.set(outputId, transparentValue());
        retireOuterResources(step.retireAfter, resources);
        continue;
      }
      const passStarted = target.onPassTiming ? performance.now() : 0;
      const slot = step.surfaceSlot;
      const slotExtent =
        slot === null
          ? extent
          : required(boundSchedule.slotExtents, slot, "bound surface slot extent");
      if (slotExtent === null)
        fail("bound_schedule", `visible output ${outputId} has no physical slot extent`);
      const surface =
        slot === null ? arena.output() : arena.slot(slot, slotExtent.width, slotExtent.height);
      const canvas = surface.getCanvas();
      canvas.clear(CanvasKit.TRANSPARENT);
      canvas.save();
      canvas.translate(-outputRoi.x, -outputRoi.y);

      try {
        switch (kind.kind) {
          case "clearRegion":
            canvas.clear(color4(kind.workingLinearRec2020Premul));
            break;
          case "frameRegion":
            if (kind.layer != null) {
              const layer = record(kind.layer, "frame layer");
              drawImageValue(
                CanvasKit,
                canvas,
                resourceValue(resources, layer.layer),
                dynamicScalar(dynamic, layer.opacity),
                "srcOver",
              );
            }
            break;
          case "importRegion":
            renderImport(
              CanvasKit,
              builtins,
              canvas,
              externalObject(externalResourceById, kind.external),
              kind,
              dynamic,
            );
            break;
          case "rasterProgram": {
            const id = positiveId(kind.program, "RasterProgram.program");
            const program = required(programs, id, "program");
            const destinations = array(kind.destinationInputs, "destinationInputs").map((input) =>
              borrowedValue(resourceValue(resources, input)),
            );
            const device = dynamicTransform(dynamic, kind.transform);
            const cacheKey = rasterLayerKey(program, kind, device, outputRoi, plan);
            const cached = cacheKey === null ? null : executor.rasterLayer(cacheKey);
            let result: ImageValue;
            if (cached) result = cached;
            else {
              const releaseFrames = bindFrames(
                CanvasKit,
                arena,
                program,
                kind,
                resources,
                device,
                extent,
              );
              try {
                result = await runProgram(id, destinations, device);
              } finally {
                releaseFrames();
              }
            }
            let retained = false;
            try {
              drawImageValue(CanvasKit, canvas, result, 1, "src");
              if (!cached && cacheKey !== null)
                retained = executor.retainRasterLayer(cacheKey, result);
            } finally {
              if (result.owned && !retained) result.image?.delete();
            }
            break;
          }
          case "rasterCaption": {
            const result = await runProgram(
              positiveId(kind.program, "RasterCaption.program"),
              [],
              dynamicTransform(dynamic, kind.transform),
            );
            drawImageValue(CanvasKit, canvas, resourceValue(resources, kind.destination), 1, "src");
            drawImageValue(
              CanvasKit,
              canvas,
              result,
              dynamicScalar(dynamic, kind.opacity),
              "srcOver",
            );
            if (result.owned) result.image?.delete();
            break;
          }
          case "resolveRegion": {
            const bounds = dynamicBounds(dynamic, kind.sampleBounds);
            canvas.save();
            canvas.clipRect(deviceRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, false);
            drawImageValue(CanvasKit, canvas, resourceValue(resources, kind.input), 1, "src");
            canvas.restore();
            break;
          }
          case "dispatchKernel": {
            const invocation = record(kind.invocation, "kernel invocation");
            const inputs = new Map<number, MaterializedValue>();
            try {
              for (const id of kernelInputs(invocation)) {
                if (!inputs.has(id))
                  inputs.set(
                    id,
                    materializeRootValue(CanvasKit, arena, resourceValue(resources, id), extent),
                  );
              }
              executeKernel(
                CanvasKit,
                builtins,
                arena,
                canvas,
                invocation,
                new Map([...inputs].map(([id, input]) => [id, input.value])),
                dynamic,
                extent,
              );
            } finally {
              for (const input of inputs.values()) input.release();
            }
            break;
          }
          case "compositeRegion": {
            const mode = record(kind.mode, "composite mode");
            const source = resourceValue(resources, kind.layer);
            const backdrop = resourceValue(resources, kind.backdrop);
            if (mode.kind === "blend" && String(mode.mode) !== "normal") {
              const rootSource = materializeRootValue(CanvasKit, arena, source, extent);
              try {
                const rootBackdrop = materializeRootValue(CanvasKit, arena, backdrop, extent);
                try {
                  drawCreativeBlend(
                    CanvasKit,
                    builtins,
                    canvas,
                    rootSource.value.image,
                    rootBackdrop.value.image,
                    String(mode.mode),
                    "srgb",
                    dynamicScalar(dynamic, kind.opacity),
                    true,
                  );
                } finally {
                  rootBackdrop.release();
                }
              } finally {
                rootSource.release();
              }
            } else {
              drawImageValue(CanvasKit, canvas, backdrop, 1, "src");
              drawImageValue(
                CanvasKit,
                canvas,
                source,
                dynamicScalar(dynamic, kind.opacity),
                "srcOver",
              );
            }
            break;
          }
          case "copyConvert": {
            const input = materializeRootValue(
              CanvasKit,
              arena,
              resourceValue(resources, kind.input),
              extent,
            );
            const scratchBytes = srgbPreviewScratchBytes(
              extent.width,
              extent.height,
              typeof executor.engineKernels?.transform_srgb_preview_f16 === "function",
            );
            try {
              await drawOutputTransform(
                CanvasKit,
                builtins,
                canvas,
                input.value.image,
                record(kind.operation, "copy operation"),
                !target.directContext &&
                  outputRoi.x === 0 &&
                  outputRoi.y === 0 &&
                  outputRoi.width === extent.width &&
                  outputRoi.height === extent.height &&
                  scratchBytes <= maxSurfaceBytes &&
                  // CanvasKit storage and the canonical Engine WASM input copy coexist.
                  arena.residentBytes + scratchBytes * 2n + dataTextureBytes <= maxFrameBytes
                  ? executor.engineKernels
                  : null,
              );
            } finally {
              input.release();
            }
            break;
          }
          default:
            fail(
              "unsupported_pass",
              `execution pass '${String(kind.kind)}' is not closed in the CanvasKit executor`,
            );
        }
      } finally {
        canvas.restore();
      }

      if (!arena.geometryRenderer) surface.flush();
      const snapshotStarted = target.onPassTiming ? performance.now() : 0;
      const snapshot = surface.makeImageSnapshot();
      target.onPassTiming?.({
        kind: String(kind.kind),
        width: outputRoi.width,
        height: outputRoi.height,
        drawMs: snapshotStarted - passStarted,
        snapshotMs: performance.now() - snapshotStarted,
      });
      resources.set(outputId, { image: snapshot, owned: true, roi: outputRoi });
      retireOuterResources(step.retireAfter, resources);
      maximumLiveImages = Math.max(maximumLiveImages, liveImages(resources));
    }

    output = image(resources, positiveId(plan.output, "plan.output"));
    const presentStarted = target.onPassTiming ? performance.now() : 0;
    const targetCanvas = target.surface.getCanvas();
    targetCanvas.clear(CanvasKit.TRANSPARENT);
    drawImage(CanvasKit, targetCanvas, output, 1, "src");
    target.surface.flush();
    target.onPassTiming?.({
      kind: "presentOutput",
      width: extent.width,
      height: extent.height,
      drawMs: performance.now() - presentStarted,
      snapshotMs: 0,
    });
    arena.prepareReport();
    arena.releaseUnused();
    return {
      profile: {
        surfaceBackend: target.directContext ? "canvaskit-gpu" : "canvaskit-cpu",
        glyphCoverage: CANVASKIT_GLYPH_COVERAGE_PROFILE,
        canvasKitVersion: canvasKitPackage.version,
      },
      passes: array(plan.passes, "plan.passes").length,
      programs: programs.size,
      physicalSurfaces: arena.count,
      maximumLiveImages,
      surfaceAllocations: arena.frameAllocations,
      surfaceReuses: arena.frameReuses,
      surfaceAllocatedBytes: arena.frameAllocatedBytes,
      surfaceResidentBytes: arena.residentBytes,
      surfacePeakBytes: arena.framePeakBytes,
      plannedSurfacePeakBytes: frame.plannedSurfacePeakBytes,
      surfacePeakBytesByKind: arena.framePeakByKind,
      surfaceEnvelopeBytes: framePlan.surfaceBytes,
      surfaceOverflowBytes: arena.frameOverflowBytes,
      heldImageBytes: framePlan.imageBytes,
      surfacePoolEvictions: arena.frameEvictions,
      ...arena.gpuCacheReport(),
      geometryTargetBytes: arena.geometryRenderer?.stats.allocatedBytes ?? 0,
      ...cacheCounterReport(frame.cacheBefore, executor.cacheSnapshot()),
      packetAdmissionMs: frame.packetAdmissionMs,
      programAdmissionMs,
      scheduleAdmissionMs: frame.scheduleAdmissionMs,
      renderMs: performance.now() - renderStarted,
      ...frame.programDiagnostics,
      pathClipElisions: arena.pathClipElisions,
      committed: true,
    };
  } finally {
    disposeValues(resources, output);
    disposePrograms(programs);
    arena.endFrame();
    arena.geometryRenderer?.endFrame();
    executor.trimCaches();
  }
}

function rasterLayerKey(
  program: ProgramStructure,
  pass: Wire,
  device: readonly number[],
  roi: DeviceRoi,
  plan: Wire,
): string | null {
  const resources = record(program.plan.resources, "program.resources");
  if (
    array(pass.externalInputs, "RasterProgram.externalInputs").length !== 0 ||
    array(pass.destinationInputs, "RasterProgram.destinationInputs").length !== 0 ||
    frameInputs(pass).length !== 0 ||
    array(resources.textures, "program textures").length !== 0 ||
    array(resources.fonts, "program fonts").length !== 0 ||
    array(resources.runtimeShaders, "program shaders").length !== 0 ||
    array(resources.scenes, "program scenes").length !== 0 ||
    array(program.plan.destinationUses, "program destination uses").length !== 0 ||
    array(
      record(program.draw.requirements, "DrawProgram requirements").destinationUses,
      "DrawProgram destination uses",
    ).length !== 0 ||
    array(record(program.plan.localPlan, "program.localPlan").passes, "program passes").some(
      (raw) => {
        const kind = record(record(raw, "program pass").kind, "program pass kind");
        return kind.kind === "readDestination" || kind.kind === "backdrop";
      },
    )
  )
    return null;
  // Native keys use the exact f64 device-matrix bits. Preserve -0 and subpixel phase here too.
  const bits = new DataView(new ArrayBuffer(8));
  const matrix = device.map((value) => {
    bits.setFloat64(0, value, false);
    return bits.getBigUint64(0, false).toString(16).padStart(16, "0");
  });
  return [
    digestString(program.plan.contentHash, "program.contentHash"),
    digestString(program.plan.frameHash, "program.frameHash"),
    JSON.stringify(record(plan.renderSpec, "plan.renderSpec"), bigintJson),
    matrix.join(""),
    roi.x,
    roi.y,
    roi.width,
    roi.height,
  ].join("|");
}

/** The host frames a raster program pass hands its program. */
function frameInputs(pass: Wire): Wire[] {
  return pass.frameInputs === undefined
    ? []
    : array(pass.frameInputs, "RasterProgram.frameInputs").map((input) =>
        record(input, "frame input"),
      );
}

/**
 * Where a program drawn through `device` finds a host frame, as native
 * `DeviceTransform::frame_region` rounds it: the whole pixels under its layer box, each edge of
 * the box's bounds rounded to the nearest pixel, half up, inside the frame and never empty.
 */
export function frameRegion(
  device: readonly number[],
  extent: { width: number; height: number },
): DeviceRoi {
  const m = device;
  let left = Infinity,
    top = Infinity,
    right = -Infinity,
    bottom = -Infinity;
  for (const [x, y] of [
    [0, 0],
    [1, 0],
    [0, 1],
    [1, 1],
  ] as const) {
    const w = m[6]! * x + m[7]! * y + m[8]!;
    if (!Number.isFinite(w) || w === 0)
      fail("frame_region", "program transform crosses the perspective horizon");
    const px = (m[0]! * x + m[1]! * y + m[2]!) / w,
      py = (m[3]! * x + m[4]! * y + m[5]!) / w;
    if (!Number.isFinite(px) || !Number.isFinite(py))
      fail("frame_region", "program transform is not finite");
    left = Math.min(left, px);
    top = Math.min(top, py);
    right = Math.max(right, px);
    bottom = Math.max(bottom, py);
  }
  const span = (low: number, high: number, size: number): [number, number] => {
    const limit = Math.max(size, 1);
    const start = Math.min(Math.max(Math.floor(low + 0.5), 0), limit - 1);
    const end = Math.min(Math.max(Math.floor(high + 0.5), start + 1), limit);
    return [start, end - start];
  };
  const [x, width] = span(left, right, extent.width);
  const [y, height] = span(top, bottom, extent.height);
  return { x, y, width, height };
}

/**
 * Hands a program the host frames its pass reads, each the frame's pixels under the program's
 * viewport, as native `frame_images` cuts them. Returns what releases them after the run.
 */
function bindFrames(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  program: AdmittedProgram,
  pass: Wire,
  resources: Map<number, ImageValue>,
  device: readonly number[],
  extent: { width: number; height: number },
): () => void {
  const inputs = frameInputs(pass);
  const reads = record(program.plan.requirements, "program.requirements").frames;
  const readFrames = reads === undefined ? [] : array(reads, "program frames");
  if (readFrames.some((frame) => !inputs.some((input) => input.frame === frame))) {
    fail("frame_contract", "a program reads a host frame its pass does not supply");
  }
  if (inputs.length === 0) return () => {};
  const region = frameRegion(device, extent);
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  const held: { surface: Surface; image: Image }[] = [];
  const identities: string[] = [];
  const release = (): void => {
    for (const identity of identities.splice(0)) program.textures.delete(identity);
    for (const { surface, image } of held.splice(0)) {
      image.delete();
      arena.releaseTransient(surface);
    }
  };
  try {
    for (const input of inputs) {
      const frame = String(input.frame);
      if (frame !== "from" && frame !== "to" && frame !== "source")
        fail("frame_contract", `unknown host frame '${frame}'`);
      const key = `input://${frame}`;
      const picture = resourceValue(resources, input.input);
      let image: Image;
      // Under a viewport that covers the frame, as when the canvas fills the output, the program
      // reads the picture as it is.
      if (sameRoi(region, root) && sameRoi(picture.roi, root) && picture.image)
        image = picture.image;
      else {
        const surface = arena.transient(region.width, region.height);
        const canvas = surface.getCanvas();
        canvas.clear(CanvasKit.TRANSPARENT);
        canvas.save();
        canvas.translate(-region.x, -region.y);
        drawImageValue(CanvasKit, canvas, picture, 1, "src");
        canvas.restore();
        surface.flush();
        image = surface.makeImageSnapshot();
        held.push({ surface, image });
      }
      const identity = textureIdentity({
        key,
        kind: "frame",
        colorDomain: "linearRec2020",
        sampleTimeMicros: null,
      });
      program.textures.set(identity, { key, kind: "frame", image });
      identities.push(identity);
    }
  } catch (error) {
    release();
    throw error;
  }
  return release;
}

function inspectProgramDiagnostics(programs: ReadonlyMap<number, ProgramStructure>) {
  const report = {
    programLocalPasses: 0,
    directRasterPrograms: 0,
    programGroups: 0,
    programTransformGroups: 0,
    programClipGroups: 0,
    programOpacityGroups: 0,
    programFilterGroups: 0,
    programMaskGroups: 0,
    programShaderGroups: 0,
    programBackdropGroups: 0,
    programBlendGroups: 0,
  };
  for (const program of programs.values()) {
    report.programLocalPasses += array(
      record(program.plan.localPlan, "program.localPlan").passes,
      "program passes",
    ).length;
    report.directRasterPrograms += Number(program.directRaster !== null);
    for (const rawNode of program.draw.nodes) {
      const node = record(rawNode, "DrawProgram node");
      if (node.kind !== "group") continue;
      const group = record(node.value, "DrawProgram group");
      report.programGroups += 1;
      report.programTransformGroups += Number(!deepEqual(group.transform, IDENTITY_MATRIX));
      report.programClipGroups += Number(group.clip != null);
      report.programOpacityGroups += Number(finiteNumber(group.opacity, "group opacity") !== 1);
      report.programFilterGroups += Number(array(group.filters, "group filters").length !== 0);
      report.programMaskGroups += Number(group.mask != null);
      report.programShaderGroups += Number(group.shader != null || group.transition != null);
      report.programBackdropGroups += Number(group.backdrop != null);
      report.programBlendGroups += Number(String(group.internalBlend) !== "normal");
    }
  }
  return report;
}

/** A template keeps each program's baselines as bytes. Their base64 text is only transport;
 * keeping it as well would hold every baseline a second time for the template's lifetime. */
function ownBaselines(plan: Wire): Wire {
  for (const [index, value] of array(plan.programs, "plan.programs").entries()) {
    const layout = record(value, `plan.programs[${index}]`);
    layout.baselinePacked = decodeBase64Bytes(layout.baselinePacked, "program.baselinePacked");
    layout.baselineFramePacked = decodeBase64Bytes(
      layout.baselineFramePacked,
      "program.baselineFramePacked",
    );
  }
  return plan;
}

function templateBytes(value: unknown, label: string): Uint8Array {
  if (!(value instanceof Uint8Array))
    fail("program_contract", `${label} must be admitted template bytes`);
  return value;
}

/** Decode a program and check its packed contract; external objects are bound per run. */
async function admitProgramStructure(
  executor: CanvasKitExecutor,
  plan: Wire,
  onTiming?: (kind: string, milliseconds: number) => void,
): Promise<ProgramStructure> {
  let at = onTiming ? performance.now() : 0;
  const measured = (kind: string) => {
    if (!onTiming) return;
    const next = performance.now();
    onTiming(kind, next - at);
    at = next;
  };
  const baselineHash = digestString(plan.baselineContentHash, "program.baselineContentHash");
  const baselineFrame = templateBytes(plan.baselineFramePacked, "program.baselineFramePacked");
  const baseline = await executor.baselineProgram(
    baselineHash,
    templateBytes(plan.baselinePacked, "program.baselinePacked"),
  );
  measured("baseline");
  const framePatch = decodeBase64Bytes(plan.framePatch, "program.framePatch");
  const framePacked = applyDrawProgramPatch(baselineFrame, framePatch);
  measured("framePatch");
  const patch = decodeBase64Bytes(plan.patch, "program.patch");
  const packed = applyDrawProgramPatch(baseline, patch);
  measured("drawPatch");
  const contentHash = digestString(plan.contentHash, "program.contentHash");
  const draw = await executor.drawProgram(contentHash, packed);
  measured("decodeDraw");
  let frameValue: unknown;
  try {
    frameValue = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(framePacked));
  } catch (error) {
    fail(
      "program_contract",
      `program ${String(plan.id)} frame metadata is invalid: ${String(error)}`,
    );
  }
  const exactPlan = { ...plan, ...record(frameValue, "program frame metadata") };
  measured("frameJson");
  if (
    !deepEqual(draw.viewport, exactPlan.viewport) ||
    !deepEqual(draw.requirements, exactPlan.requirements)
  ) {
    fail("program_contract", `program ${String(exactPlan.id)} packed contract mismatch`);
  }
  measured("contract");
  const directRaster = directRasterPlan(draw, record(exactPlan.localPlan, "program.localPlan"));
  measured("rasterPlan");
  return { plan: exactPlan, draw, directRaster };
}

/** Bind a program's textures, fonts, shaders and scenes to the frame's external objects. */
function bindProgramResources(
  executor: CanvasKitExecutor,
  program: ProgramStructure,
  objects: Map<number, CanvasKitExternalObject>,
  onTiming?: (kind: string, milliseconds: number) => void,
): AdmittedProgram {
  const started = onTiming ? performance.now() : 0;
  const resources = record(program.plan.resources, "program.resources");
  const requirements = record(program.plan.requirements, "program.requirements");
  const textures = admitProgramTextures(requirements.externalTextures, resources.textures, objects);
  const fonts = new Map<string, Typeface>();
  for (const raw of array(resources.fonts, "program fonts")) {
    const binding = record(raw, "font binding");
    const object = required(objects, positiveId(binding.slot, "font slot"), "font slot");
    if (object.kind !== "font" || !object.bytes) fail("font_contract", "font slot has no bytes");
    const faceIndex = positiveIdOrZero(binding.faceIndex, "faceIndex");
    // PreparedFrame uses the public ContentDigest spelling while DrawProgram's sealed internal
    // font ABI stores the same digest as bare lowercase hex. Normalize only at this typed bridge.
    const faceHash = digestString(binding.faceHash, "font binding faceHash").slice(
      "sha256:".length,
    );
    const key = fontKey(faceHash, faceIndex);
    fonts.set(key, executor.typeface(key, faceIndex, object.bytes));
  }
  const shaders = new Map<string, RuntimeEffect>();
  for (const raw of array(resources.runtimeShaders, "program shaders")) {
    const binding = record(raw, "shader binding");
    const object = required(objects, positiveId(binding.slot, "shader slot"), "shader slot");
    if (object.kind !== "runtimeShader" || !object.bytes)
      fail("shader_contract", "shader slot has no source bytes");
    const key = `${String(binding.key)}:${JSON.stringify(object.key, bigintJson)}`;
    shaders.set(String(binding.key), executor.runtimeShader(key, object.bytes));
  }
  const scenes = new Map<string, CanvasKitExternalObject>();
  for (const raw of array(resources.scenes, "program scenes")) {
    const binding = record(raw, "scene binding");
    const object = required(objects, positiveId(binding.slot, "scene slot"), "scene slot");
    if (object.kind !== "scene3d" || !object.image)
      fail("scene_contract", "Scene3D slot has no raster image");
    scenes.set(String(binding.key), object);
  }
  onTiming?.("resources", performance.now() - started);
  return {
    ...program,
    pathCache: executor.paths,
    paintCache: executor.paints,
    textures,
    fonts,
    shaders,
    scenes,
  };
}

function admitFrameProgramLayout(layout: Wire, program: Wire, index: number): Wire {
  const label = `program ${index + 1}`;
  const layoutId = positiveId(layout.id, `plan.programs[${index}].id`);
  if (
    layoutId !== index + 1 ||
    positiveId(program.id, `bindings.programs[${index}].id`) !== layoutId
  ) {
    fail("program_layout", `${label} does not match its reusable template slot`);
  }
  digestString(layout.structureHash, `plan.programs[${index}].structureHash`);
  digestString(layout.baselineContentHash, `plan.programs[${index}].baselineContentHash`);
  digestString(layout.baselineFrameHash, `plan.programs[${index}].baselineFrameHash`);
  digestString(program.contentHash, `bindings.programs[${index}].contentHash`);
  digestString(program.frameHash, `bindings.programs[${index}].frameHash`);
  return { ...layout, ...program };
}

async function executeProgram(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  engineKernels: CanvasKitEngineKernels | null,
  admitted: AdmittedProgram,
  run: ProgramRun,
  destinations: ImageValue[],
  device: number[],
): Promise<ImageValue> {
  let yieldAt = performance.now();
  const localPlan = record(admitted.plan.localPlan, "program.localPlan");
  arena.treeSignatures = new Map();
  const { bound } = run;
  if (run.direct) {
    return executeDirectRasterProgram(
      CanvasKit,
      builtins,
      arena,
      admitted,
      bound,
      destinations,
      device,
      localPlan,
    );
  }
  const values = new Map<number, ImageValue>();
  const retiredValues: ImageValue[] = [];
  const localSlotImages = new Map<number, Image>();
  const retiredImages = new Set<Image>();
  let result = transparentValue();
  const retire = (resources: readonly number[]): void => {
    for (const resource of resources) {
      const value = required(values, resource, "retired program resource");
      if (value.owned) retiredValues.push(value);
      values.delete(resource);
    }
  };
  try {
    for (const [index, rawPass] of array(localPlan.passes, "localPlan.passes").entries()) {
      // A frame with many Shutter samples can record fast enough overall while blocking the
      // media clock for several frame intervals. Yield only at complete pass boundaries.
      if (arena.yieldToPresentation && performance.now() - yieldAt >= 6) {
        await arena.yieldToPresentation();
        yieldAt = performance.now();
      }
      const pass = record(rawPass, "program pass");
      const kind = record(pass.kind, "program pass kind");
      const step = bound.passes[index];
      if (!step) fail("program_schedule", `bound program pass ${index + 1} is absent`);
      const { output, storage } = step;
      if (storage.kind === "transparent") {
        values.set(output, transparentValue());
        retire(step.retireAfter);
        continue;
      }
      if (storage.kind === "destination") {
        const destination = destinations[positiveId(storage.destination, "destination") - 1];
        if (!destination)
          fail("program_schedule", `destination ${String(storage.destination)} is absent`);
        values.set(output, borrowedValue(destination));
        retire(step.retireAfter);
        continue;
      }
      if (storage.kind === "alias") {
        values.set(
          output,
          borrowedValue(
            required(
              values,
              positiveId(storage.source, "program alias source"),
              "program alias source",
            ),
          ),
        );
        retire(step.retireAfter);
        continue;
      }
      if (storage.kind !== "surface" && storage.kind !== "output") {
        fail("program_schedule", `unknown program storage kind '${String(storage.kind)}'`);
      }

      const outputRoi = step.deviceRoi;
      if (outputRoi.width === 0 || outputRoi.height === 0) {
        // A static surface allocation can be clipped away by frame binding. Match the native
        // executor: its value is transparent, and no surface or kernel invocation is needed.
        values.set(output, transparentValue());
        retire(step.retireAfter);
        continue;
      }
      const slot =
        storage.kind === "surface" ? positiveId(storage.slot, "bound program surface slot") : 0;
      const localStarted = arena.onLocalPassTiming ? performance.now() : 0;
      let surface: Surface;
      if (slot === 0) {
        surface = arena.transient(outputRoi.width, outputRoi.height);
      } else {
        const prior = localSlotImages.get(slot);
        if (prior) {
          retiredImages.add(prior);
          prior.delete();
        }
        const slotExtent = required(bound.slotExtents, slot, "bound program slot extent");
        if (!slotExtent) fail("program_schedule", `visible program slot ${slot} has no extent`);
        surface = arena.programSlot(
          positiveId(admitted.plan.id, "program id"),
          slot,
          slotExtent.width,
          slotExtent.height,
        );
      }
      let localSnapshotMs = 0;
      const snapshot = await (async () => {
        try {
          const canvas = surface.getCanvas();
          canvas.clear(CanvasKit.TRANSPARENT);
          canvas.save();
          canvas.translate(-outputRoi.x, -outputRoi.y);
          try {
            await executeProgramPass(
              CanvasKit,
              builtins,
              engineKernels,
              arena,
              canvas,
              admitted,
              kind,
              values,
              destinations,
              device,
              localPlan,
              outputRoi,
            );
          } finally {
            canvas.restore();
          }
          if (!arena.gpu || !arena.deferGpuFlush) surface.flush();
          const at = arena.onLocalPassTiming ? performance.now() : 0;
          const image = surface.makeImageSnapshot();
          if (arena.onLocalPassTiming) localSnapshotMs = performance.now() - at;
          return image;
        } finally {
          if (slot === 0) arena.releaseTransient(surface);
        }
      })();
      arena.onLocalPassTiming?.({
        kind: String(kind.kind),
        width: outputRoi.width,
        height: outputRoi.height,
        drawMs: performance.now() - localStarted - localSnapshotMs,
        snapshotMs: localSnapshotMs,
      });
      if (slot !== 0) localSlotImages.set(slot, snapshot);
      const value = { image: snapshot, owned: true, roi: outputRoi };
      values.set(output, value);
      if (storage.kind === "output") result = value;
      retire(step.retireAfter);
    }
    if (result.image === null)
      result = values.get(positiveId(localPlan.output, "localPlan.output")) ?? result;
    // `result` is deliberately excluded from the cleanup below and ownership crosses this
    // function boundary. The outer pass must delete it after copying it into its own surface;
    // reporting it as borrowed leaks one snapshot per DrawProgram execution.
    return { image: result.image, owned: true, roi: result.roi };
  } finally {
    const liveSlotImages = new Set(localSlotImages.values());
    for (const value of [...values.values(), ...retiredValues]) {
      if (
        value.owned &&
        value.image &&
        value.image !== result.image &&
        !retiredImages.has(value.image) &&
        !liveSlotImages.has(value.image)
      ) {
        value.image.delete();
      }
    }
    values.clear();
    for (const image of liveSlotImages) {
      if (image !== result.image && !retiredImages.has(image)) image.delete();
    }
  }
}

function directRasterPlan(draw: DrawProgramWire, localPlan: Wire): DirectRasterPlan | null {
  const passes = array(localPlan.passes, "direct raster passes");
  if (
    passes.length === 1 &&
    record(record(passes[0], "pass").kind, "pass kind").kind === "rasterTree"
  ) {
    return null;
  }
  const destinationUses = array(draw.requirements.destinationUses, "DrawProgram destination uses");
  const resources = array(localPlan.resources, "direct raster resources").map((raw) =>
    record(raw, "direct raster resource"),
  );
  const layerBounds = new Map<number, RectWire>();
  const backdrops = new Map<number, DirectRasterBackdrop>();
  for (const rawPass of array(localPlan.passes, "direct raster passes")) {
    const kind = record(record(rawPass, "direct raster pass").kind, "direct raster pass kind");
    if (kind.kind === "readDestination") {
      if (String(kind.operation) !== "backdrop") return null;
      const node = positiveIdOrZero(kind.node, "direct raster backdrop node");
      if (backdrops.has(node)) return null;
      const localInputs = array(kind.localInputs, "direct raster destination local inputs");
      backdrops.set(node, {
        destination:
          kind.external == null
            ? null
            : positiveId(kind.external, "direct raster backdrop destination") - 1,
        hasLocalInputs: localInputs.length !== 0,
      });
      continue;
    }
    if (kind.kind !== "applyOpacity" && kind.kind !== "applyFilter") continue;
    const node = positiveIdOrZero(kind.node, "direct raster layer node");
    const resourceId = positiveId(
      kind.kind === "applyOpacity" ? kind.input : kind.output,
      "direct raster layer resource",
    );
    const resource = record(
      requiredIndex(resources, resourceId - 1, "direct raster layer resource"),
      "direct raster layer resource",
    );
    const bounds = record(resource.bounds, "direct raster layer bounds");
    if (bounds.kind !== "finite") return null;
    layerBounds.set(node, rect(bounds.rect, "direct raster layer rect"));
  }
  const memo = new Map<string, boolean>();
  const visiting = new Set<number>();
  let spatialFilters = false;
  const eligible = (id: number, depth: number, insideLayer: boolean, rotated: boolean): boolean => {
    if (depth > 128 || visiting.has(id)) return false;
    const memoKey = `${id}:${Number(insideLayer)}:${Number(rotated)}`;
    const cached = memo.get(memoKey);
    if (cached !== undefined) return cached;
    const node = record(requiredIndex(draw.nodes, id, "direct raster node"), "direct raster node");
    if (node.kind !== "group") {
      memo.set(memoKey, true);
      return true;
    }
    const group = record(node.value, "direct raster group");
    const opacity = finiteNumber(group.opacity, "direct raster group opacity");
    const filters = array(group.filters, "direct raster group filters");
    const opensLayer = opacity !== 1 || filters.length !== 0;
    const backdrop = group.backdrop == null ? null : backdrops.get(id);
    const kinds = filters.map((filter) =>
      String(record(filter, "direct raster group filter").kind),
    );
    const backdropKinds =
      group.backdrop == null
        ? []
        : array(
            record(group.backdrop, "direct raster backdrop").filters,
            "direct raster backdrop filters",
          ).map((filter) => String(record(filter, "direct raster backdrop filter").kind));
    const spatial = kinds.some((kind) => kind === "blur" || kind === "dropShadow");
    const transformed =
      rotated ||
      !isScaleTranslateMatrix(numberArray(group.transform, 9, "direct raster group transform"));
    const supported =
      group.mask == null &&
      // saveLayer's image-filter crop truncates the wide glow tail; use scheduled ROI passes.
      // Noise displacement runs the kernel Native shares, which only a scheduled pass draws.
      !kinds.some((kind) =>
        [
          "glow",
          "bloom",
          "radialBlur",
          "filmGrain",
          "lensDistortion",
          "chromaticAberration",
          "velocityBlur",
          "noiseDisplacement",
        ].includes(kind),
      ) &&
      // A backdrop list that names an advanced filter runs a scheduled pass per function.
      !backdropKinds.some((kind) =>
        [
          "glow",
          "bloom",
          "radialBlur",
          "filmGrain",
          "lensDistortion",
          "chromaticAberration",
          "velocityBlur",
          "noiseDisplacement",
        ].includes(kind),
      ) &&
      // Skia filters a blur or drop-shadow layer under a rotated or skewed CTM in scale/translate
      // space and resamples the result, softening its content; scheduled passes filter in device space.
      !(spatial && transformed) &&
      // Path clips use the scheduled filled-path coverage contract, not clipPath's AA mask.
      (group.clip == null || record(group.clip, "direct raster clip").kind !== "path") &&
      group.shader == null &&
      group.transition == null &&
      String(group.internalBlend) === "normal" &&
      opacity >= 0 &&
      opacity <= 1 &&
      (group.backdrop == null || backdrop !== undefined) &&
      !(backdrop?.hasLocalInputs && (insideLayer || opensLayer)) &&
      (opacity === 0 || (opacity === 1 && filters.length === 0) || layerBounds.has(id));
    if (!supported) {
      memo.set(memoKey, false);
      return false;
    }
    spatialFilters ||= spatial;
    visiting.add(id);
    const children = array(group.children, "direct raster group children");
    const result = children.every((child) =>
      eligible(
        positiveIdOrZero(child, "direct raster child"),
        depth + 1,
        insideLayer || opensLayer,
        transformed,
      ),
    );
    visiting.delete(id);
    memo.set(memoKey, result);
    return result;
  };
  const roots = draw.roots.map((root) => positiveIdOrZero(root, "direct raster root"));
  const externalBackdropCount = [...backdrops.values()].filter(
    (backdrop) => backdrop.destination !== null,
  ).length;
  return roots.every((root) => eligible(root, 0, false, false)) &&
    externalBackdropCount === destinationUses.length
    ? { roots, layerBounds, backdrops, spatialFilters }
    : null;
}

function executeDirectRasterProgram(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  admitted: AdmittedProgram,
  bound: BoundProgramRuntimeSchedule,
  destinations: ImageValue[],
  device: number[],
  localPlan: Wire,
): ImageValue {
  const output = positiveId(localPlan.output, "direct raster output");
  const outputRoi = required(bound.resourceRois, output, "direct raster output ROI");
  if (outputRoi.width === 0 || outputRoi.height === 0) return transparentValue();
  const surface = arena.transient(outputRoi.width, outputRoi.height);
  try {
    const canvas = surface.getCanvas();
    canvas.clear(CanvasKit.TRANSPARENT);
    canvas.save();
    canvas.translate(-outputRoi.x, -outputRoi.y);
    canvas.concat(programMatrix(admitted, localPlan, output, device));
    try {
      const fallback = (root: unknown) => {
        drawProgramTree(
          CanvasKit,
          builtins,
          arena,
          surface,
          canvas,
          admitted,
          destinations,
          outputRoi,
          positiveIdOrZero(root, "direct raster root"),
          0,
        );
      };
      if (arena.geometryRenderer)
        arena.geometryRenderer.drawChildren(
          CanvasKit,
          canvas,
          admitted.draw,
          admitted.directRaster!.roots,
          IDENTITY_MATRIX,
          fallback,
        );
      else admitted.directRaster!.roots.forEach(fallback);
    } finally {
      canvas.restore();
    }
    surface.flush();
    return { image: surface.makeImageSnapshot(), owned: true, roi: outputRoi };
  } finally {
    arena.releaseTransient(surface);
  }
}

function drawProgramTree(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  surface: Surface,
  canvas: Canvas,
  admitted: AdmittedProgram,
  destinations: ImageValue[],
  outputRoi: DeviceRoi,
  nodeId: number,
  depth: number,
): void {
  if (depth > 128) fail("program_schedule", "direct raster tree exceeds the group depth budget");
  const node = record(
    requiredIndex(admitted.draw.nodes, nodeId, "direct raster node"),
    "direct raster node",
  );
  if (node.kind !== "group") {
    drawProgramNode(
      CanvasKit,
      builtins,
      canvas,
      admitted,
      nodeId,
      IDENTITY_MATRIX,
      0,
      undefined,
      arena,
    );
    return;
  }
  const group = record(node.value, "direct raster group");
  const opacity = finiteNumber(group.opacity, "direct raster group opacity");
  const filters = array(group.filters, "direct raster group filters");
  canvas.save();
  let layer = false;
  let layerPaint: Paint | null = null;
  let layerFilter: ImageFilter | null = null;
  try {
    canvas.concat(numberArray(group.transform, 9, "direct raster group transform"));
    if (opacity === 0) return;
    if (opacity !== 1 || filters.length !== 0) {
      layerPaint = new CanvasKit.Paint();
      layerPaint.setAlphaf(opacity);
      const bounds = required(
        admitted.directRaster!.layerBounds,
        nodeId,
        "direct raster layer bounds",
      );
      for (const rawFilter of filters) {
        const next = drawProgramImageFilter(
          CanvasKit,
          record(rawFilter, "direct raster group filter"),
          layerFilter,
        );
        if (next !== layerFilter) layerFilter?.delete();
        layerFilter = next;
      }
      if (layerFilter) layerPaint.setImageFilter(layerFilter);
      canvas.saveLayer(layerPaint, skRect(CanvasKit, bounds));
      layer = true;
    }
    // The layer is opened first so this group's clip limits the filter input without clipping a
    // blur/drop-shadow expansion when the layer is restored. Ancestor clips remain outside and
    // therefore continue to constrain the complete descendant result.
    if (group.clip != null) {
      applyClip(
        CanvasKit,
        canvas,
        admitted.draw,
        record(group.clip, "direct raster group clip"),
        IDENTITY_MATRIX,
      );
    }
    if (group.backdrop != null) {
      const contract = required(
        admitted.directRaster!.backdrops,
        nodeId,
        "direct raster backdrop contract",
      );
      const destination = contract.destination === null ? null : destinations[contract.destination];
      if (contract.destination !== null && !destination) {
        fail(
          "program_schedule",
          `direct raster backdrop destination ${contract.destination + 1} is absent`,
        );
      }
      drawDirectRasterBackdrop(
        CanvasKit,
        arena,
        surface,
        canvas,
        destination ?? null,
        contract.hasLocalInputs,
        record(group.backdrop, "direct raster backdrop"),
        outputRoi,
      );
    }
    const children = array(group.children, "direct raster group children");
    const fallback = (child: unknown) => {
      drawProgramTree(
        CanvasKit,
        builtins,
        arena,
        surface,
        canvas,
        admitted,
        destinations,
        outputRoi,
        positiveIdOrZero(child, "direct raster child"),
        depth + 1,
      );
    };
    if (arena.geometryRenderer)
      arena.geometryRenderer.drawChildren(
        CanvasKit,
        canvas,
        admitted.draw,
        children as number[],
        IDENTITY_MATRIX,
        fallback,
      );
    else children.forEach(fallback);
  } finally {
    if (layer) canvas.restore();
    layerPaint?.delete();
    layerFilter?.delete();
    canvas.restore();
  }
}

function drawDirectRasterBackdrop(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  surface: Surface,
  canvas: Canvas,
  destination: ImageValue | null,
  hasLocalInputs: boolean,
  backdrop: Wire,
  outputRoi: DeviceRoi,
): void {
  const bounds = rect(backdrop.bounds, "direct raster backdrop bounds");
  const filters = array(backdrop.filters, "direct raster backdrop filters");
  let localSnapshot: Image | null = null;
  let combinedSnapshot: Image | null = null;
  let combinedSurface: Surface | null = null;
  let input = destination;
  if (hasLocalInputs) {
    surface.flush();
    localSnapshot = surface.makeImageSnapshot();
    const local: ImageValue = { image: localSnapshot, owned: false, roi: outputRoi };
    if (destination) {
      combinedSurface = arena.transient(outputRoi.width, outputRoi.height);
      const combinedCanvas = combinedSurface.getCanvas();
      combinedCanvas.clear(CanvasKit.TRANSPARENT);
      combinedCanvas.save();
      combinedCanvas.translate(-outputRoi.x, -outputRoi.y);
      try {
        drawImageValue(CanvasKit, combinedCanvas, destination, 1, "src");
        drawImageValue(CanvasKit, combinedCanvas, local, 1, "srcOver");
      } finally {
        combinedCanvas.restore();
      }
      combinedSurface.flush();
      combinedSnapshot = combinedSurface.makeImageSnapshot();
      input = { image: combinedSnapshot, owned: false, roi: outputRoi };
    } else {
      input = local;
    }
  }
  if (!input) fail("program_schedule", "direct raster backdrop has no destination input");
  canvas.save();
  try {
    // Establish the local-space output clip first, then cancel the complete local-to-surface
    // matrix so the resolved destination remains stationary in root device coordinates. Skia's
    // clip is already stored in device space, therefore filtering may still sample the admitted
    // destination outside the output bounds while only the requested backdrop region is written.
    canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
    const matrix = numberArray(canvas.getTotalMatrix(), 9, "direct raster canvas matrix");
    const inverse = inverse3(matrix);
    if (!inverse) fail("program_schedule", "direct raster backdrop has a non-invertible transform");
    canvas.concat(inverse);
    canvas.translate(-outputRoi.x, -outputRoi.y);
    if (filters.length === 0) {
      drawImageValue(CanvasKit, canvas, input, 1, "src");
    } else {
      applyFiltersAsLayerValue(
        CanvasKit,
        canvas,
        input,
        backdropFiltersInDeviceSpace(filters, matrix, bounds),
        null,
      );
    }
  } finally {
    canvas.restore();
    combinedSnapshot?.delete();
    if (combinedSurface) arena.releaseTransient(combinedSurface);
    localSnapshot?.delete();
  }
}

async function executeProgramPass(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  engineKernels: CanvasKitEngineKernels | null,
  arena: SurfaceArena,
  canvas: Canvas,
  admitted: AdmittedProgram,
  kind: Wire,
  values: Map<number, ImageValue>,
  destinations: ImageValue[],
  device: number[],
  localPlan: Wire,
  outputRoi: DeviceRoi,
): Promise<void> {
  switch (kind.kind) {
    case "clear":
      return;
    case "rasterNode":
      drawProgramNode(
        CanvasKit,
        builtins,
        canvas,
        admitted,
        positiveIdOrZero(kind.node, "RasterNode.node"),
        programMatrix(admitted, localPlan, kind.output, device),
        0,
        undefined,
        arena,
      );
      return;
    case "rasterTree": {
      const matrix = programMatrix(admitted, localPlan, kind.output, device);
      const objects = new RasterDrawObjects(CanvasKit, admitted.draw, admitted.paintCache);
      const bounds = new Map<number, ReturnType<typeof programNodeBounds>>();
      try {
        const fallback = (root: unknown) => {
          drawProgramNode(
            CanvasKit,
            builtins,
            canvas,
            admitted,
            positiveIdOrZero(root, "RasterTree.root"),
            matrix,
            0,
            bounds,
            arena,
            objects,
            outputRoi,
          );
        };
        const roots = array(kind.roots, "RasterTree.roots");
        if (arena.geometryRenderer)
          arena.geometryRenderer.drawChildren(
            CanvasKit,
            canvas,
            admitted.draw,
            roots as number[],
            matrix,
            fallback,
          );
        else roots.forEach(fallback);
      } finally {
        objects.delete();
      }
      return;
    }
    case "readDestination": {
      const external =
        kind.external == null
          ? transparentValue()
          : (destinations[positiveId(kind.external, "program destination") - 1] ??
            transparentValue());
      drawImageValue(CanvasKit, canvas, external, 1, "src");
      for (const input of array(kind.localInputs, "local destination inputs")) {
        drawImageValue(CanvasKit, canvas, programValue(values, input), 1, "srcOver");
      }
      return;
    }
    case "backdrop": {
      // The bounds and the filter lengths are in the owner's local units, while the backdrop and the
      // canvas are in device pixels: the filters, mapped into device pixels, read the backdrop around
      // the bounds, which then clip what they produce. One draw covers the cleared target: a second
      // one over the unfiltered backdrop would leave some of it along anti-aliased edges.
      const clip = { kind: "rect", value: rect(kind.bounds, "program backdrop bounds") };
      const matrix = programMatrix(admitted, localPlan, kind.output, device);
      const input = programValue(values, kind.input);
      const filters = backdropFiltersInDeviceSpace(
        array(kind.filters, "backdrop filters"),
        matrix,
        clip.value,
      );
      const last =
        filters.length === 0 ? null : record(filters[filters.length - 1], "backdrop filter");
      canvas.save();
      try {
        applyClip(CanvasKit, canvas, admitted.draw, clip, matrix);
        if (last?.kind === "noiseDisplacement") {
          drawDisplacedBackdrop(
            CanvasKit,
            builtins,
            engineKernels,
            arena,
            canvas,
            input,
            filters.slice(0, -1),
            last,
            outputRoi,
          );
        } else {
          applyFiltersAsLayerValue(CanvasKit, canvas, input, filters, null);
        }
      } finally {
        canvas.restore();
      }
      return;
    }
    case "sourceOver":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.destination), 1, "src");
      drawImageValue(CanvasKit, canvas, programValue(values, kind.source), 1, "srcOver");
      return;
    case "applyClip": {
      const clip = record(kind.clip, "program clip");
      if (clip.kind === "path") {
        canvas.save();
        canvas.concat(programMatrix(admitted, localPlan, kind.output, device));
        drawPathClipCoverage(CanvasKit, canvas, admitted.draw, clip, admitted.pathCache);
        canvas.restore();
        drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "srcIn");
        return;
      }
      canvas.save();
      applyClip(
        CanvasKit,
        canvas,
        admitted.draw,
        record(kind.clip, "program clip"),
        programMatrix(admitted, localPlan, kind.output, device),
      );
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      canvas.restore();
      return;
    }
    case "applyFilter": {
      const exact = programMatrix(admitted, localPlan, kind.output, device);
      const matrix = programFilterMatrix(kind, exact);
      const filter = programFilterInDeviceSpace(record(kind.filter, "program filter"), matrix);
      if (filter.kind === "noiseDisplacement") {
        const tables = engineKernels?.perlin_noise_tables;
        if (!tables) fail("noise_displacement_tables", "noise displacement tables are unavailable");
        drawNoiseDisplacement(
          CanvasKit,
          builtins,
          canvas,
          programValue(values, kind.input),
          filter,
          matrix,
          tables.call(engineKernels, positiveIdOrZero(filter.seed, "noise seed")),
        );
        return;
      }
      if (filter.kind === "chromaticAberration") {
        const input = programValue(values, kind.input);
        if (!input.image) return;
        const source = input.image.makeShaderOptions(
          CanvasKit.TileMode.Decal,
          CanvasKit.TileMode.Decal,
          CanvasKit.FilterMode.Linear,
          CanvasKit.MipmapMode.None,
          [1, 0, input.roi.x, 0, 1, input.roi.y, 0, 0, 1],
        );
        if (!source) fail("chromatic_shader", "chromatic input shader allocation failed");
        let shader: Shader | null = null;
        try {
          shader = builtins.shader("chromaticAberration", pair(filter.offset, "chromatic offset"), [
            source,
          ]);
          drawShader(CanvasKit, canvas, shader, "src");
        } finally {
          shader?.delete();
          source.delete();
        }
        return;
      }
      if (filter.kind === "velocityBlur" && arena.gpu) {
        const input = programValue(values, kind.input);
        if (!input.image) return;
        const uniforms = velocityBlurUniforms(
          input.roi,
          outputRoi,
          pair(filter.from, "velocity blur trail start"),
          pair(filter.to, "velocity blur trail end"),
        );
        if (uniforms) {
          const source = input.image.makeShaderOptions(
            CanvasKit.TileMode.Clamp,
            CanvasKit.TileMode.Clamp,
            CanvasKit.FilterMode.Nearest,
            CanvasKit.MipmapMode.None,
          );
          let shader: Shader | null = null;
          try {
            shader = builtins.shader("velocityBlur", uniforms, [source]);
            drawShader(CanvasKit, canvas, shader, "src");
          } finally {
            shader?.delete();
            source.delete();
          }
          return;
        }
      }
      if (filter.kind === "radialBlur") {
        const planner = engineKernels?.plan_radial_blur;
        if (!planner) fail("radial_blur_plan", "radial blur pass planner is unavailable");
        const input = programValue(values, kind.input);
        if (!input.image) return;
        const amount = finiteNumber(filter.amount, "radial blur amount");
        const plan = planner.call(
          engineKernels,
          amount,
          input.roi.x,
          input.roi.y,
          input.roi.width,
          input.roi.height,
          outputRoi.x,
          outputRoi.y,
          outputRoi.width,
          outputRoi.height,
        );
        await drawRadialBlur(
          CanvasKit,
          builtins,
          arena,
          canvas,
          input,
          outputRoi,
          pair(filter.center, "radial blur center"),
          amount,
          plan,
        );
        return;
      }
      if (filter.kind === "filmGrain") {
        drawFilmGrain(
          CanvasKit,
          builtins,
          canvas,
          programValue(values, kind.input),
          outputRoi,
          finiteNumber(filter.seed, "film grain seed"),
          finiteNumber(filter.amount, "film grain amount"),
          finiteNumber(filter.size, "film grain size"),
        );
        return;
      }
      if (filter.kind === "bloom" || filter.kind === "glow" || filter.kind === "lensDistortion") {
        const source = programValue(values, kind.input);
        if (!source.image) return;
        const inputRoi = source.roi;
        if (arena.gpu && filter.kind === "lensDistortion") {
          drawLensDistortion(
            CanvasKit,
            builtins,
            canvas,
            source.image,
            inputRoi,
            outputRoi,
            programLensFrame(localPlan, kind, matrix, exact),
            finiteNumber(filter.k1, "lens distortion k1"),
            finiteNumber(filter.k2, "lens distortion k2"),
          );
          return;
        }
        const planner = engineKernels?.plan_pyramid_filter;
        if (arena.gpu && planner && (filter.kind === "bloom" || filter.kind === "glow")) {
          const recipe = planner.call(
            engineKernels,
            inputRoi.width,
            inputRoi.height,
            outputRoi.width,
            outputRoi.height,
            inputRoi.x - outputRoi.x,
            inputRoi.y - outputRoi.y,
            JSON.stringify(filter),
          );
          if (arena.yieldToPresentation)
            await drawPyramidFilterCooperative(
              arena.yieldToPresentation,
              CanvasKit,
              builtins,
              arena,
              canvas,
              source.image,
              inputRoi,
              outputRoi,
              filter.kind,
              recipe,
            );
          else
            drawPyramidFilter(
              CanvasKit,
              builtins,
              arena,
              canvas,
              source.image,
              inputRoi,
              outputRoi,
              filter.kind,
              recipe,
            );
          return;
        }
        let readRoi = inputRoi;
        if (planner && (filter.kind === "bloom" || filter.kind === "glow")) {
          const recipe = planner.call(
            engineKernels,
            inputRoi.width,
            inputRoi.height,
            outputRoi.width,
            outputRoi.height,
            inputRoi.x - outputRoi.x,
            inputRoi.y - outputRoi.y,
            JSON.stringify(filter),
          );
          // Keep raster coverage intact. Only copy the region used by the F16 kernel across
          // the CanvasKit/WASM boundary, using the planner's original sampling grid.
          const region =
            recipe.length >= 15
              ? {
                  x: outputRoi.x + recipe[0]!,
                  y: outputRoi.y + recipe[1]!,
                  width: recipe[2]!,
                  height: recipe[3]!,
                }
              : outputRoi;
          const x = Math.max(inputRoi.x, region.x),
            y = Math.max(inputRoi.y, region.y);
          const width = Math.max(
            0,
            Math.min(inputRoi.x + inputRoi.width, region.x + region.width) - x,
          );
          const height = Math.max(
            0,
            Math.min(inputRoi.y + inputRoi.height, region.y + region.height) - y,
          );
          if (!width || !height) return;
          readRoi = { x, y, width, height };
        }
        const info = {
          width: readRoi.width,
          height: readRoi.height,
          colorType: CanvasKit.ColorType.RGBA_F16,
          alphaType: CanvasKit.AlphaType.Premul,
          colorSpace: CanvasKit.ColorSpace.SRGB,
        };
        const input = source.image.readPixels(
          readRoi.x - inputRoi.x,
          readRoi.y - inputRoi.y,
          info,
          undefined,
          readRoi.width * 8,
        );
        if (!input || !(input instanceof Uint8Array))
          fail("pyramid_read", "working F16 input read failed");
        const dimensions = [
          readRoi.width,
          readRoi.height,
          outputRoi.width,
          outputRoi.height,
          readRoi.x - outputRoi.x,
          readRoi.y - outputRoi.y,
        ] as const;
        const output = await (filter.kind === "lensDistortion"
          ? (() => {
              const kernel = engineKernels?.apply_lens_distortion_f16;
              if (!kernel)
                fail(
                  "lens_distortion_kernel",
                  "working-linear F16 lens distortion kernel is unavailable",
                );
              const frame = programLensFrame(localPlan, kind, matrix, exact);
              return kernel.call(
                engineKernels,
                input,
                ...dimensions,
                outputRoi.x,
                outputRoi.y,
                finiteNumber(filter.k1, "lens distortion k1"),
                finiteNumber(filter.k2, "lens distortion k2"),
                ...frame,
              );
            })()
          : filter.kind === "bloom"
            ? (() => {
                const kernel = engineKernels?.apply_bloom_f16;
                if (!kernel) fail("bloom_kernel", "working-linear F16 bloom kernel is unavailable");
                return kernel.call(
                  engineKernels,
                  input,
                  ...dimensions,
                  finiteNumber(filter.threshold, "bloom threshold"),
                  finiteNumber(filter.knee, "bloom knee"),
                  finiteNumber(filter.intensity, "bloom intensity"),
                  finiteNumber(filter.radius, "bloom radius"),
                );
              })()
            : (() => {
                const kernel = engineKernels?.apply_glow_f16;
                if (!kernel) fail("glow_kernel", "working-linear F16 glow kernel is unavailable");
                return kernel.call(
                  engineKernels,
                  input,
                  ...dimensions,
                  JSON.stringify(record(filter.color, "glow color")),
                  finiteNumber(filter.intensity, "glow intensity"),
                  finiteNumber(filter.radius, "glow radius"),
                );
              })());
        const image = CanvasKit.MakeImage(
          { ...info, width: outputRoi.width, height: outputRoi.height },
          output,
          outputRoi.width * 8,
        );
        if (!image) fail("pyramid_image", "working F16 pyramid output allocation failed");
        try {
          drawImageValue(CanvasKit, canvas, { image, owned: false, roi: outputRoi }, 1, "src");
        } finally {
          image.delete();
        }
        return;
      }
      applyFiltersAsLayerValue(CanvasKit, canvas, programValue(values, kind.input), [filter], null);
      return;
    }
    case "applyMask": {
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      const mask = programValue(values, kind.mask);
      const inverted = kind.mode === "alphaInverted" || kind.mode === "luminanceInverted";
      if (!mask.image) {
        if (!inverted) canvas.clear(CanvasKit.TRANSPARENT);
        return;
      }
      const source = mask.image.makeShaderOptions(
        CanvasKit.TileMode.Decal,
        CanvasKit.TileMode.Decal,
        CanvasKit.FilterMode.Nearest,
        CanvasKit.MipmapMode.None,
        [1, 0, mask.roi.x, 0, 1, mask.roi.y, 0, 0, 1],
      );
      if (!source) fail("mask_shader", "mask input cannot become a shader");
      let coverage: Shader | null = null;
      try {
        const luminance = kind.mode === "luminance" || kind.mode === "luminanceInverted";
        coverage = luminance ? builtins.shader("maskCoverage", [], [source]) : source;
        drawShader(CanvasKit, canvas, coverage, inverted ? "dstOut" : "dstIn");
      } finally {
        if (coverage !== source) coverage?.delete();
        source.delete();
      }
      return;
    }
    case "applyOpacity":
      drawImageValue(
        CanvasKit,
        canvas,
        programValue(values, kind.input),
        finiteNumber(kind.opacity, "program opacity"),
        "src",
      );
      return;
    case "applyTransition": {
      const transition = record(kind.transition, "local transition");
      const bounds = rect(transition.bounds, "transition bounds");
      const matrix = programMatrix(admitted, localPlan, kind.output, device);
      const inverse = inverse3(matrix);
      if (!inverse) fail("shader_transform", "transition owner transform is not invertible");
      const children: Shader[] = [];
      const paint = new CanvasKit.Paint();
      let shader: Shader | null = null;
      canvas.save();
      try {
        for (const id of [kind.from, kind.to]) {
          const input = programValue(values, id);
          const content = input.image
            ? input.image.makeShaderOptions(
                CanvasKit.TileMode.Decal,
                CanvasKit.TileMode.Decal,
                CanvasKit.FilterMode.Linear,
                CanvasKit.MipmapMode.None,
                mul3(inverse, [1, 0, input.roi.x, 0, 1, input.roi.y, 0, 0, 1]),
              )
            : CanvasKit.Shader.MakeColor(CanvasKit.TRANSPARENT, CanvasKit.ColorSpace.SRGB);
          if (!content) fail("shader_transform", "transition input shader allocation failed");
          try {
            children.push(
              builtins.shader("transitionInput", [bounds.width, bounds.height], [content]),
            );
          } finally {
            content.delete();
          }
        }
        const progress = finiteNumber(transition.progress, "transition progress");
        const kernel = record(transition.kernel, "transition kernel");
        if (kernel.type === "shader") {
          const effect = required(
            admitted.shaders,
            String(record(kernel.shader, "transition shader key").uri),
            "transition shader",
          );
          // The canvas is origin-anchored, so owner pixels are the shader's own.
          shader = makeRuntimeShader(
            CanvasKit,
            builtins,
            effect,
            bounds,
            array(kernel.uniforms, "transition uniforms"),
            array(kernel.textures, "transition textures"),
            admitted,
            children.splice(0),
            [["progress", [progress]]],
          );
        } else {
          shader = builtins.shader(
            transitionKernel(String(kernel.kind)),
            [
              bounds.width,
              bounds.height,
              progress,
              ...transitionValues(kernel.params, String(kernel.kind)),
            ],
            children,
          );
        }
        paint.setAntiAlias(true);
        paint.setShader(shader);
        canvas.concat(matrix);
        canvas.drawRect(skRect(CanvasKit, bounds), paint);
      } finally {
        canvas.restore();
        paint.delete();
        shader?.delete();
        for (const child of children) child.delete();
      }
      return;
    }
    case "applyShader":
      applyProgramShader(
        CanvasKit,
        builtins,
        canvas,
        admitted,
        programValue(values, kind.input),
        (kind.backdrops === undefined ? [] : array(kind.backdrops, "shader backdrops")).map((id) =>
          programValue(values, id),
        ),
        record(kind.shader, "program shader"),
        programMatrix(admitted, localPlan, kind.output, device),
      );
      return;
    case "applyTransform":
      drawImageValue(CanvasKit, canvas, programValue(values, kind.input), 1, "src");
      return;
    case "blend":
      drawCreativeBlendValues(
        CanvasKit,
        builtins,
        canvas,
        programValue(values, kind.source),
        programValue(values, kind.destination),
        String(kind.mode),
        String(kind.space),
        1,
        false,
      );
      return;
    default:
      fail("unsupported_program_pass", `program pass '${String(kind.kind)}' is not closed`);
  }
}

/** Pass-local immutable fill paints and a saveLayer paint. Skia copies restore paint on save. */
class RasterDrawObjects {
  private readonly fills = new Map<number, Paint>();
  private readonly leases: FillPaintLease[] = [];
  private layerPaint: Paint | null = null;
  constructor(
    private readonly CK: CanvasKit,
    private readonly draw: DrawProgramWire,
    private readonly cache?: RasterPaintCache,
  ) {}
  fill(id: number): Paint {
    let paint = this.fills.get(id);
    if (!paint) {
      const wire = requiredIndex(this.draw.paints, id, "paint");
      const lease = this.cache?.acquire(wire, () => makePaint(this.CK, wire));
      paint = lease?.paint ?? makePaint(this.CK, wire);
      if (lease) this.leases.push(lease);
      this.fills.set(id, paint);
    }
    return paint;
  }
  opacity(alpha: number): Paint {
    const paint = (this.layerPaint ??= new this.CK.Paint());
    paint.setAlphaf(alpha);
    return paint;
  }
  delete(): void {
    if (this.cache) this.leases.forEach((lease) => lease.release());
    else for (const paint of this.fills.values()) paint.delete();
    this.leases.length = 0;
    this.fills.clear();
    this.layerPaint?.delete();
    this.layerPaint = null;
  }
}

function isIdentityMatrix(matrix: readonly number[]): boolean {
  return matrix.length === 9 && matrix.every((value, i) => value === IDENTITY_MATRIX[i]);
}

function isScaleTranslateMatrix(matrix: readonly number[]): boolean {
  return (
    matrix[1] === 0 && matrix[3] === 0 && matrix[6] === 0 && matrix[7] === 0 && matrix[8] === 1
  );
}

export function drawProgramNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  nodeId: number,
  matrix: number[],
  depth = 0,
  boundsCache = new Map<number, ReturnType<typeof programNodeBounds>>(),
  arena?: SurfaceArena,
  objects?: RasterDrawObjects,
  targetRoi?: DeviceRoi,
): void {
  if (!objects) {
    const owned = new RasterDrawObjects(CanvasKit, admitted.draw, admitted.paintCache);
    try {
      drawProgramNode(
        CanvasKit,
        builtins,
        canvas,
        admitted,
        nodeId,
        matrix,
        depth,
        boundsCache,
        arena,
        owned,
        targetRoi,
      );
    } finally {
      owned.delete();
    }
    return;
  }
  if (depth > 128) fail("program_schedule", "RasterTree exceeds the group depth limit");
  const node = record(admitted.draw.nodes[nodeId], `DrawProgram node ${nodeId}`);
  const leafStarted = arena?.onRasterLeafTiming && node.kind !== "group" ? performance.now() : null;
  const transformed = !isIdentityMatrix(matrix);
  // These leaves manage internal canvas state. Keep an outer recovery save for exceptional exits.
  const saved =
    transformed || node.kind === "image" || node.kind === "instanceBatch" || node.kind === "shadow";
  if (saved) canvas.save();
  if (transformed) canvas.concat(matrix);
  try {
    switch (node.kind) {
      case "path":
        drawPathNode(
          CanvasKit,
          canvas,
          admitted.draw,
          record(node.value, "path node"),
          objects,
          admitted.pathCache,
        );
        break;
      case "instanceBatch":
        drawBatchNode(
          CanvasKit,
          builtins,
          canvas,
          admitted,
          record(node.value, "batch node"),
          arena,
        );
        break;
      case "image":
        drawImageNode(CanvasKit, builtins, canvas, admitted, record(node.value, "image node"));
        break;
      case "glyphRun":
        drawGlyphNode(CanvasKit, canvas, admitted, record(node.value, "glyph node"), objects);
        break;
      case "shadow":
        drawShadowNode(CanvasKit, canvas, record(node.value, "shadow node"));
        break;
      case "runtimeShader":
        drawRuntimeShaderNode(
          CanvasKit,
          builtins,
          canvas,
          admitted,
          record(node.value, "runtime shader node"),
        );
        break;
      case "scene3d":
        drawSceneNode(CanvasKit, builtins, canvas, admitted, record(node.value, "Scene3D node"));
        break;
      case "group": {
        const group = record(node.value, "RasterTree group");
        if (
          group.mask != null ||
          group.backdrop != null ||
          group.shader != null ||
          group.transition != null ||
          array(group.filters, "RasterTree group filters").length !== 0 ||
          group.internalBlend !== "normal"
        ) {
          fail("program_schedule", `group node ${nodeId} requires a separate pixel operation`);
        }
        const transform = numberArray(group.transform, 9, "RasterTree group transform");
        const opacity = finiteNumber(group.opacity, "RasterTree group opacity");
        if (opacity === 0) break;
        const paint = opacity === 1 ? null : objects.opacity(opacity);
        const groupTransformed = !isIdentityMatrix(transform);
        const groupSaved = !paint && (groupTransformed || group.clip != null);
        if (paint) {
          const bounds = programNodeBounds(admitted.draw, nodeId, boundsCache);
          canvas.saveLayer(paint, bounds == null ? undefined : skRect(CanvasKit, bounds));
        } else if (groupSaved) canvas.save();
        if (groupTransformed) canvas.concat(transform);
        let clipped = false;
        const pathClip =
          group.clip != null && record(group.clip, "RasterTree clip").kind === "path";
        const clipValue = pathClip
          ? record(record(group.clip, "RasterTree clip").value, "path clip")
          : null;
        const clipBounds =
          pathClip && arena?.boundedPathClipLayers
            ? programClipBounds(admitted.draw, record(group.clip, "RasterTree clip"))
            : undefined;
        const clipLayer = clipBounds == null ? undefined : skRect(CanvasKit, clipBounds);
        try {
          if (pathClip && arena?.fuseClipPaints && (group.children as number[]).length === 1) {
            const child = admitted.draw.nodes[(group.children as number[])[0]!]!;
            if (child.kind === "path") {
              const fill = record(child.value, "clip paint fill");
              if (fill.fill != null && fill.stroke == null) {
                const rectangle = requiredIndex(
                  admitted.draw.paths,
                  fill.path,
                  "clip paint rectangle",
                );
                const bounds = paintRectangleBounds(rectangle);
                if (bounds) {
                  const covered =
                    !arena.recordingClipPaints &&
                    clipPaintRectangleCoversOutline(
                      rectangle,
                      requiredIndex(admitted.draw.paths, clipValue!.path, "clip paint outline"),
                      canvas.getTotalMatrix(),
                    );
                  canvas.save();
                  try {
                    if (!covered)
                      canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
                    drawPathNode(
                      CanvasKit,
                      canvas,
                      admitted.draw,
                      {
                        path: clipValue!.path,
                        fillRule: clipValue!.fillRule,
                        fill: fill.fill,
                        stroke: null,
                      },
                      objects,
                      admitted.pathCache,
                    );
                  } finally {
                    canvas.restore();
                  }
                  arena.pathClipElisions++;
                  return;
                }
              }
            }
          }
          if (group.clip != null) {
            if (!pathClip)
              applyClip(
                CanvasKit,
                canvas,
                admitted.draw,
                record(group.clip, "RasterTree group clip"),
                IDENTITY_MATRIX,
              );
            canvas.saveLayer(undefined, clipLayer);
            clipped = true;
          }
          const children = array(group.children, "RasterTree group children");
          const fallback = (child: unknown) =>
            drawProgramNode(
              CanvasKit,
              builtins,
              canvas,
              admitted,
              positiveIdOrZero(child, "RasterTree child"),
              IDENTITY_MATRIX,
              depth + 1,
              boundsCache,
              arena,
              objects,
              targetRoi,
            );
          if (arena?.trees)
            drawCachedChildren(
              CanvasKit,
              builtins,
              canvas,
              admitted,
              children,
              depth,
              boundsCache,
              arena,
              objects!,
              targetRoi,
              fallback,
            );
          else if (arena?.geometryRenderer)
            arena.geometryRenderer.drawChildren(
              CanvasKit,
              canvas,
              admitted.draw,
              children as number[],
              IDENTITY_MATRIX,
              fallback,
            );
          else children.forEach(fallback);
        } finally {
          if (clipped && pathClip) {
            const restore = new CanvasKit.Paint();
            try {
              restore.setBlendMode(CanvasKit.BlendMode.DstIn);
              canvas.saveLayer(restore, clipLayer);
              try {
                drawPathClipCoverage(
                  CanvasKit,
                  canvas,
                  admitted.draw,
                  record(group.clip, "RasterTree path clip"),
                  admitted.pathCache,
                );
              } finally {
                canvas.restore();
              }
            } finally {
              restore.delete();
            }
          }
          if (clipped) canvas.restore();
          if (paint || groupSaved) canvas.restore();
        }
        break;
      }
      default:
        fail("unsupported_node", `DrawProgram node '${String(node.kind)}' is not closed`);
    }
  } finally {
    if (saved) canvas.restore();
    if (leafStarted !== null)
      arena!.onRasterLeafTiming?.(String(node.kind), performance.now() - leafStarted);
  }
}

function drawCachedChildren(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  children: unknown[],
  depth: number,
  boundsCache: Map<number, ReturnType<typeof programNodeBounds>>,
  arena: SurfaceArena,
  objects: RasterDrawObjects,
  targetRoi: DeviceRoi | undefined,
  fallback: (child: unknown) => void,
): void {
  const cache = arena.trees!;
  const original = (part: unknown[]) => {
    if (arena.geometryRenderer)
      arena.geometryRenderer.drawChildren(
        CanvasKit,
        canvas,
        admitted.draw,
        part as number[],
        IDENTITY_MATRIX,
        fallback,
      );
    else part.forEach(fallback);
  };
  if (children.length < 3) {
    original(children);
    return;
  }
  let at = 0;
  while (at < children.length) {
    const start = at,
      signatures: RasterTreeSignature[] = [];
    while (at < children.length) {
      const signature = cache.signature(
        admitted.draw,
        children[at] as number,
        arena.treeSignatures,
      );
      if (!signature) break;
      signatures.push(signature);
      at++;
    }
    if (!signatures.length) {
      original([children[at++]]);
      continue;
    }
    const part = children.slice(start, at);
    const signature = {
      key: signatures.map((s) => `${s.key.length}:${s.key}`).join(""),
      leaves: signatures.reduce((n, s) => n + s.leaves, 0),
      bytes: signatures.reduce((n, s) => n + s.bytes, 0),
    };
    const bounds = part.map((id) => programNodeBounds(admitted.draw, id as number, boundsCache));
    if (bounds.some((b) => b === undefined)) {
      original(part);
      continue;
    }
    const nonempty = bounds.filter((b): b is RectWire => b != null);
    if (!nonempty.length) {
      original(part);
      continue;
    }
    const x = Math.min(...nonempty.map((b) => b.x)),
      y = Math.min(...nonempty.map((b) => b.y));
    const right = Math.max(...nonempty.map((b) => b.x + b.width)),
      bottom = Math.max(...nonempty.map((b) => b.y + b.height));
    const picture = cache.acquire(signature, () => {
      const recorder = new CanvasKit.PictureRecorder();
      try {
        const recording = recorder.beginRecording(
          CanvasKit.XYWHRect(x - 8, y - 8, right - x + 16, bottom - y + 16),
        );
        const isolated = Object.create(arena) as SurfaceArena;
        isolated.trees = null;
        isolated.geometryRenderer = null;
        isolated.recordingClipPaints = true;
        isolated.onRasterLeafTiming = null;
        for (const id of part)
          drawProgramNode(
            CanvasKit,
            builtins,
            recording,
            admitted,
            id as number,
            IDENTITY_MATRIX,
            depth + 1,
            boundsCache,
            isolated,
            objects,
          );
        return recorder.finishRecordingAsPicture();
      } finally {
        recorder.delete();
      }
    });
    if (picture) canvas.drawPicture(picture);
    else original(part);
  }
}

function drawPathNode(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  draw: DrawProgramWire,
  node: Wire,
  objects?: RasterDrawObjects,
  cache?: RasterPathCache,
): void {
  const wire = requiredIndex(draw.paths, node.path, "path"),
    fillRule = String(node.fillRule);
  const lease = cache?.acquire(wire, fillRule, () => buildPath(CanvasKit, wire, fillRule));
  const path = lease?.path ?? buildPath(CanvasKit, wire, fillRule);
  try {
    if (node.fill != null) {
      const paint =
        objects?.fill(node.fill as number) ??
        makePaint(CanvasKit, requiredIndex(draw.paints, node.fill, "paint"));
      try {
        canvas.drawPath(path, paint);
      } finally {
        if (!objects) paint.delete();
      }
    }
    if (node.stroke != null) {
      const stroke = record(node.stroke, "path stroke");
      const paint = makePaint(CanvasKit, requiredIndex(draw.paints, stroke.paint, "stroke paint"));
      try {
        const effect = configureStrokePaint(CanvasKit, paint, stroke);
        try {
          canvas.drawPath(path, paint);
        } finally {
          effect?.delete();
        }
      } finally {
        paint.delete();
      }
    }
  } finally {
    if (!lease || lease.owned) path.delete();
  }
}

function drawBatchNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
  arena?: SurfaceArena,
): void {
  const program = admitted.draw;
  if (arena?.geometryRenderer?.batch(CanvasKit, canvas, program, node as GpuInstanceBatch)) return;
  const shape = record(node.shape, "instance shape");
  const imageRegion = shape.kind === "image" ? record(shape.value, "instance image region") : null;
  const imageNode =
    imageRegion === null
      ? null
      : {
          texture: imageRegion.texture,
          src: imageRegion.src,
          dst: { x: 0, y: 0, width: 1, height: 1 },
          sampling: imageRegion.sampling,
          opacity: 1,
        };
  const atlas =
    imageRegion === null
      ? null
      : normalizeAtlasImage(CanvasKit, builtins, admitted, imageRegion, arena);
  const fillPaint = new CanvasKit.Paint();
  const strokePaint = new CanvasKit.Paint();
  const layerPaint = new CanvasKit.Paint();
  const batchPath =
    shape.kind === "path"
      ? buildPath(CanvasKit, requiredIndex(program.paths, shape.value, "batch path"), "nonZero")
      : null;
  const batchRoundRect =
    shape.kind === "roundRect"
      ? roundRect(CanvasKit, record(shape.value, "instance round rect"))
      : null;
  let fixedDash: ReturnType<typeof CanvasKit.PathEffect.MakeDash> | null = null;
  try {
    fillPaint.setAntiAlias(true);
    strokePaint.setAntiAlias(true);
    strokePaint.setStyle(CanvasKit.PaintStyle.Stroke);
    const pathStyle = node.pathStyle === null ? null : record(node.pathStyle, "batch Path style");
    const dash =
      pathStyle === null
        ? []
        : array(pathStyle.dash, "batch dash intervals").map((value) =>
            finiteNumber(value, "batch dash interval"),
          );
    if (pathStyle !== null) {
      strokePaint.setStrokeCap(
        pathStyle.cap === "round"
          ? CanvasKit.StrokeCap.Round
          : pathStyle.cap === "square"
            ? CanvasKit.StrokeCap.Square
            : CanvasKit.StrokeCap.Butt,
      );
      strokePaint.setStrokeJoin(
        pathStyle.join === "round"
          ? CanvasKit.StrokeJoin.Round
          : pathStyle.join === "bevel"
            ? CanvasKit.StrokeJoin.Bevel
            : CanvasKit.StrokeJoin.Miter,
      );
      strokePaint.setStrokeMiter(finiteNumber(pathStyle.miterLimit, "batch stroke miter"));
    }
    const range = record(node.instances, "batch instance range");
    const start = positiveIdOrZero(range.start, "batch instance start");
    const count = positiveIdOrZero(range.count, "batch instance count");
    const end = start + count;
    if (!Number.isSafeInteger(end) || end > program.batchInstances.count) {
      fail("wire_shape", "batch instance range escapes the DrawProgram table");
    }
    const view = program.batchInstances.view;
    const tableCount = program.batchInstances.count;
    const strokeRange =
      node.strokeColors === null ? null : record(node.strokeColors, "batch stroke color range");
    const strokeStart =
      strokeRange === null ? 0 : positiveIdOrZero(strokeRange.start, "batch stroke color start");
    const strokeView = program.strokeColors.view;
    const dashRange =
      node.dashOffsets === null ? null : record(node.dashOffsets, "batch dash offset range");
    const dashStart =
      dashRange === null ? 0 : positiveIdOrZero(dashRange.start, "batch dash offset start");
    const dashView = program.dashOffsets.view;
    if (
      imageRegion !== null &&
      atlas !== null &&
      drawOpaqueImageAtlas(
        CanvasKit,
        canvas,
        atlas.image,
        imageRegion,
        atlas.source,
        view,
        tableCount,
        start,
        end,
        fillPaint,
      )
    )
      return;
    if (dash.length && dashRange === null) {
      fixedDash = CanvasKit.PathEffect.MakeDash(
        dash,
        finiteNumber(pathStyle?.dashOffset, "batch dash offset"),
      );
      if (fixedDash) strokePaint.setPathEffect(fixedDash);
    }
    const drawShape = (paint: Paint) => {
      if (shape.kind === "circle") canvas.drawCircle(0, 0, 1, paint);
      else if (shape.kind === "rect") canvas.drawRect(CanvasKit.XYWHRect(0, 0, 1, 1), paint);
      else if (shape.kind === "roundRect") canvas.drawRRect(batchRoundRect!, paint);
      else if (shape.kind === "path") canvas.drawPath(batchPath!, paint);
      else fail("wire_shape", "image instances use texture sampling");
    };
    // Neighbouring instances often share a color; keep each paint's shader until it changes.
    const fillColor = [NaN, NaN, NaN, NaN, NaN],
      strokeColor = [NaN, NaN, NaN, NaN, NaN];
    const setColor = (
      paint: Paint,
      last: number[],
      red: number,
      green: number,
      blue: number,
      alpha: number,
      opacity: number,
    ) => {
      if (
        last[0] === red &&
        last[1] === green &&
        last[2] === blue &&
        last[3] === alpha &&
        last[4] === opacity
      )
        return;
      last[0] = red;
      last[1] = green;
      last[2] = blue;
      last[3] = alpha;
      last[4] = opacity;
      setWorkingColor(CanvasKit, paint, red, green, blue, alpha, opacity);
    };
    for (let index = start; index < end; index += 1) {
      const matrixOffset = index * 48;
      const a = view.getFloat64(matrixOffset, true);
      const b = view.getFloat64(matrixOffset + 8, true);
      const c = view.getFloat64(matrixOffset + 16, true);
      const d = view.getFloat64(matrixOffset + 24, true);
      const e = view.getFloat64(matrixOffset + 32, true);
      const f = view.getFloat64(matrixOffset + 40, true);
      const colorOffset = tableCount * 48 + index * 16;
      const alpha = view.getFloat32(colorOffset + 12, true);
      const opacity = view.getFloat32(tableCount * 64 + index * 4, true);
      const strokeWidth = view.getFloat32(tableCount * 68 + index * 4, true);
      if (imageNode !== null && strokeWidth !== 0)
        fail("wire_shape", "image instances cannot have a stroke");
      const strokeOffset = (strokeStart + index - start) * 16;
      const strokeAlpha =
        strokeRange === null ? alpha : strokeView.getFloat32(strokeOffset + 12, true);
      if (opacity <= 0 || (alpha <= 0 && (strokeWidth <= 0 || strokeAlpha <= 0))) continue;
      // Single-coverage opacity is folded by the shared DrawProgram compiler. Remaining
      // opacity isolates overlapping paints or image tinting.
      if (opacity < 1) {
        layerPaint.setAlphaf(opacity);
        const bounds = instanceOpacityBounds(program, shape, pathStyle, strokeWidth, [
          a,
          b,
          c,
          d,
          e,
          f,
        ]);
        canvas.saveLayer(layerPaint, bounds === null ? undefined : skRect(CanvasKit, bounds));
      }
      canvas.save();
      try {
        canvas.concat([a, c, e, b, d, f, 0, 0, 1]);
        if (alpha > 0 && imageNode !== null) {
          drawImageNode(
            CanvasKit,
            builtins,
            canvas,
            admitted,
            { ...imageNode, src: atlas!.source },
            {
              red: view.getFloat32(colorOffset, true),
              green: view.getFloat32(colorOffset + 4, true),
              blue: view.getFloat32(colorOffset + 8, true),
              alpha,
            },
            atlas!.image,
          );
        } else if (alpha > 0 && (pathStyle === null || pathStyle.fill === true)) {
          setColor(
            fillPaint,
            fillColor,
            view.getFloat32(colorOffset, true),
            view.getFloat32(colorOffset + 4, true),
            view.getFloat32(colorOffset + 8, true),
            alpha,
            1,
          );
          drawShape(fillPaint);
        }
        if (strokeWidth > 0 && strokeAlpha > 0) {
          if (strokeRange !== null) {
            setColor(
              strokePaint,
              strokeColor,
              strokeView.getFloat32(strokeOffset, true),
              strokeView.getFloat32(strokeOffset + 4, true),
              strokeView.getFloat32(strokeOffset + 8, true),
              strokeAlpha,
              1,
            );
          } else if (alpha > 0) {
            setColor(
              strokePaint,
              strokeColor,
              view.getFloat32(colorOffset, true),
              view.getFloat32(colorOffset + 4, true),
              view.getFloat32(colorOffset + 8, true),
              alpha,
              1,
            );
          } else {
            fail("wire_shape", "visible stroke requires a visible color");
          }
          strokePaint.setStrokeWidth(strokeWidth);
          let rowDash: ReturnType<typeof CanvasKit.PathEffect.MakeDash> | null = null;
          if (dash.length && dashRange !== null) {
            rowDash = CanvasKit.PathEffect.MakeDash(
              dash,
              dashView.getFloat32((dashStart + index - start) * 4, true),
            );
            strokePaint.setPathEffect(rowDash);
          }
          try {
            drawShape(strokePaint);
          } finally {
            rowDash?.delete();
          }
        }
      } finally {
        canvas.restore();
      }
      if (opacity < 1) canvas.restore();
    }
  } finally {
    atlas?.image.delete();
    fixedDash?.delete();
    batchPath?.delete();
    layerPaint.delete();
    strokePaint.delete();
    fillPaint.delete();
  }
}

function drawOpaqueImageAtlas(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  image: Image,
  region: Wire,
  source: RectWire,
  rows: DataView,
  count: number,
  start: number,
  end: number,
  paint: Paint,
): boolean {
  if (start === end) return false;
  const src = source;
  const x = src.x * image.width(),
    y = src.y * image.height();
  const width = src.width * image.width(),
    height = src.height * image.height();
  const sourceRects: number[] = [];
  const xforms: number[] = [];
  for (let index = start; index < end; index += 1) {
    const matrix = index * 48;
    const a = rows.getFloat64(matrix, true),
      b = rows.getFloat64(matrix + 8, true);
    const c = rows.getFloat64(matrix + 16, true),
      d = rows.getFloat64(matrix + 24, true);
    const e = rows.getFloat64(matrix + 32, true),
      f = rows.getFloat64(matrix + 40, true);
    const color = count * 48 + index * 16;
    if (
      rows.getFloat32(color, true) !== 1 ||
      rows.getFloat32(color + 4, true) !== 1 ||
      rows.getFloat32(color + 8, true) !== 1 ||
      rows.getFloat32(color + 12, true) !== 1 ||
      rows.getFloat32(count * 64 + index * 4, true) !== 1 ||
      rows.getFloat32(count * 68 + index * 4, true) !== 0
    )
      return false;
    const scos = a / width,
      ssin = b / width;
    if (Math.abs(c / height + ssin) > 1e-6 || Math.abs(d / height - scos) > 1e-6) return false;
    sourceRects.push(x, y, x + width, y + height);
    xforms.push(scos, ssin, e, f);
  }
  canvas.drawAtlas(image, sourceRects, xforms, paint, null, null, {
    filter:
      String(region.sampling) === "nearestClamp"
        ? CanvasKit.FilterMode.Nearest
        : CanvasKit.FilterMode.Linear,
  });
  return true;
}

function normalizeAtlasImage(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  admitted: AdmittedProgram,
  region: Wire,
  arena?: SurfaceArena,
): { image: Image; source: RectWire } {
  if (!arena) fail("surface_lifetime", "atlas image needs a working surface arena");
  // Convert source texels before filtering, as the native ProgramTexture does. The cropped
  // snapshot also keeps bilinear sampling from reading a neighboring atlas tile.
  const object = resolveProgramTexture(
    admitted.textures,
    record(region.texture, "atlas texture"),
    "atlas texture",
  );
  const image = object.image!;
  const { left, top, width, height, source } = atlasCrop(
    rect(region.src, "atlas source"),
    image.width(),
    image.height(),
  );
  const surface = arena.transient(width, height);
  try {
    const canvas = surface.getCanvas();
    canvas.clear(CanvasKit.TRANSPARENT);
    const shader = normalizedExternalShader(CanvasKit, builtins, object, "nearestClamp");
    const paint = new CanvasKit.Paint();
    try {
      paint.setBlendMode(CanvasKit.BlendMode.Src);
      paint.setShader(shader);
      canvas.save();
      try {
        canvas.translate(-left, -top);
        canvas.drawRect(CanvasKit.XYWHRect(left, top, width, height), paint);
      } finally {
        canvas.restore();
      }
    } finally {
      paint.delete();
      shader.delete();
    }
    surface.flush();
    return { image: surface.makeImageSnapshot([0, 0, width, height]), source };
  } finally {
    arena.releaseTransient(surface);
  }
}

function drawImageNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
  tint?: LinearColorWire,
  normalizedImage?: Image,
): void {
  const texture = record(node.texture, "image texture");
  const object = resolveProgramTexture(admitted.textures, texture, "program texture");
  const image = normalizedImage ?? object.image!;
  const src = rect(node.src, "image src");
  const dst = rect(node.dst, "image dst");
  const source = {
    x: src.x * image.width(),
    y: src.y * image.height(),
    width: src.width * image.width(),
    height: src.height * image.height(),
  };
  const localFromPixel = [
    dst.width / source.width,
    0,
    dst.x - (source.x * dst.width) / source.width,
    0,
    dst.height / source.height,
    dst.y - (source.y * dst.height) / source.height,
    0,
    0,
    1,
  ];
  const shader = normalizedImage
    ? imageShader(CanvasKit, normalizedImage, String(node.sampling))
    : normalizedExternalShader(CanvasKit, builtins, object, String(node.sampling));
  const paint = new CanvasKit.Paint();
  try {
    paint.setAntiAlias(true);
    paint.setAlphaf(finiteNumber(node.opacity, "image opacity") * (tint?.alpha ?? 1));
    if (tint && (tint.red !== 1 || tint.green !== 1 || tint.blue !== 1 || tint.alpha !== 1)) {
      const inverseAlpha = 1 / tint.alpha;
      const filter = CanvasKit.ColorFilter.MakeMatrix([
        tint.red * inverseAlpha,
        0,
        0,
        0,
        0,
        0,
        tint.green * inverseAlpha,
        0,
        0,
        0,
        0,
        0,
        tint.blue * inverseAlpha,
        0,
        0,
        0,
        0,
        0,
        1,
        0,
      ]);
      paint.setColorFilter(filter);
      filter.delete();
    }
    paint.setShader(shader);
    canvas.save();
    canvas.clipRect(skRect(CanvasKit, dst), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    canvas.drawRect(skRect(CanvasKit, source), paint);
    canvas.restore();
  } finally {
    paint.delete();
    shader.delete();
  }
}

function drawGlyphNode(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
  objects?: RasterDrawObjects,
): void {
  if (node.outline != null) {
    drawPathNode(
      CanvasKit,
      canvas,
      admitted.draw,
      {
        path: node.outline,
        fillRule: "nonZero",
        fill: node.paint,
        stroke: node.stroke,
      },
      objects,
      admitted.pathCache,
    );
    return;
  }
  const fontKeyWire = record(node.font, "glyph font");
  const face = required(
    admitted.fonts,
    fontKey(String(fontKeyWire.faceHash), positiveIdOrZero(fontKeyWire.faceIndex, "faceIndex")),
    "font face",
  );
  const font = new CanvasKit.Font(face, finiteNumber(node.fontSize, "font size"));
  try {
    // Match the Native Skia executor exactly. CanvasKit otherwise keeps its platform defaults,
    // which changes stem coverage (and therefore parity) even with identical glyph positions.
    configureGlyphCoverage(CanvasKit, font);
    const glyphs = array(node.glyphs, "glyphs");
    const ids = Uint16Array.from(
      glyphs.map((raw) => positiveIdOrZero(record(raw, "glyph").id, "glyph id")),
    );
    const positions = Float32Array.from(
      glyphs.flatMap((raw) => {
        const glyph = record(raw, "glyph");
        return [finiteNumber(glyph.x, "glyph x"), finiteNumber(glyph.y, "glyph y")];
      }),
    );
    const fill =
      objects?.fill(node.paint as number) ??
      makePaint(CanvasKit, requiredIndex(admitted.draw.paints, node.paint, "glyph paint"));
    try {
      canvas.drawGlyphs(ids, positions, 0, 0, font, fill);
    } finally {
      if (!objects) fill.delete();
    }
    if (node.stroke != null) {
      const stroke = record(node.stroke, "glyph stroke");
      const outline = makePaint(
        CanvasKit,
        requiredIndex(admitted.draw.paints, stroke.paint, "glyph stroke paint"),
      );
      try {
        const effect = configureStrokePaint(CanvasKit, outline, stroke);
        try {
          canvas.drawGlyphs(ids, positions, 0, 0, font, outline);
        } finally {
          effect?.delete();
        }
      } finally {
        outline.delete();
      }
    }
  } finally {
    font.delete();
  }
}

function configureGlyphCoverage(CanvasKit: CanvasKit, font: Font): void {
  const profile = CANVASKIT_GLYPH_COVERAGE_PROFILE;
  if (profile.hinting === "none") font.setHinting(CanvasKit.FontHinting.None);
  if (profile.edging === "antialias") font.setEdging(CanvasKit.FontEdging.AntiAlias);
  font.setSubpixel(profile.subpixelPositioning);
}

function drawShadowNode(CanvasKit: CanvasKit, canvas: Canvas, node: Wire): void {
  const original = record(node.shape, "shadow shape");
  const shapeBounds = rect(original.rect, "shadow shape bounds");
  const offset = pair(node.offset, "shadow offset");
  const spread = finiteNumber(node.spread, "shadow spread") * (node.inset === true ? -1 : 1);
  const radii = array(original.radii, "shadow radii").map((raw) => {
    const radius = pair(raw, "shadow radius");
    return [Math.max(0, radius[0] + spread), Math.max(0, radius[1] + spread)];
  });
  const shape = {
    rect: {
      x: shapeBounds.x + offset[0] - spread,
      y: shapeBounds.y + offset[1] - spread,
      width: shapeBounds.width + spread * 2,
      height: shapeBounds.height + spread * 2,
    },
    radii,
  };
  const paint = new CanvasKit.Paint();
  const sigma = Math.max(
    finiteNumber(node.sigmaX, "shadow sigmaX"),
    finiteNumber(node.sigmaY, "shadow sigmaY"),
  );
  const filter =
    sigma > 0 ? CanvasKit.MaskFilter.MakeBlur(CanvasKit.BlurStyle.Normal, sigma, true) : null;
  try {
    paint.setAntiAlias(true);
    setLinearColor(
      CanvasKit,
      paint,
      record(node.color, "shadow color") as unknown as LinearColorWire,
    );
    if (filter) paint.setMaskFilter(filter);
    if (node.inset === true) {
      canvas.save();
      canvas.clipRRect(roundRect(CanvasKit, original), CanvasKit.ClipOp.Intersect, true);
      const pad = sigma * 4 + 1;
      const left = Math.min(shapeBounds.x, shape.rect.x) - pad;
      const top = Math.min(shapeBounds.y, shape.rect.y) - pad;
      const right =
        Math.max(shapeBounds.x + shapeBounds.width, shape.rect.x + shape.rect.width) + pad;
      const bottom =
        Math.max(shapeBounds.y + shapeBounds.height, shape.rect.y + shape.rect.height) + pad;
      const ring = new CanvasKit.PathBuilder();
      try {
        ring.setFillType(CanvasKit.FillType.EvenOdd);
        ring.addRect(CanvasKit.LTRBRect(left, top, right, bottom));
        if (shape.rect.width > 0 && shape.rect.height > 0)
          ring.addRRect(roundRect(CanvasKit, shape));
        const path = ring.detach();
        try {
          canvas.drawPath(path, paint);
        } finally {
          path.delete();
        }
      } finally {
        ring.delete();
      }
      canvas.restore();
    } else if (shape.rect.width > 0 && shape.rect.height > 0) {
      canvas.save();
      canvas.clipRRect(roundRect(CanvasKit, original), CanvasKit.ClipOp.Difference, true);
      canvas.drawRRect(roundRect(CanvasKit, shape), paint);
      canvas.restore();
    }
  } finally {
    filter?.delete();
    paint.delete();
  }
}

function drawRuntimeShaderNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
): void {
  const shader = record(node.shader, "runtime shader key");
  const effect = required(admitted.shaders, String(shader.uri), "runtime shader");
  const bounds = rect(node.bounds, "runtime shader bounds");
  const runtime = instantiateRuntimeShader(
    CanvasKit,
    builtins,
    effect,
    bounds,
    array(node.uniforms, "shader uniforms"),
    array(node.textures, "shader textures"),
    admitted,
    null,
  );
  const paint = new CanvasKit.Paint();
  try {
    paint.setAntiAlias(true);
    paint.setShader(runtime);
    canvas.drawRect(skRect(CanvasKit, bounds), paint);
  } finally {
    paint.delete();
    runtime.delete();
  }
}

function drawSceneNode(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  node: Wire,
): void {
  const scene = record(node.scene, "Scene3D key");
  const object = required(admitted.scenes, String(scene.contentHash), "Scene3D image");
  const image = object.image!;
  const bounds = rect(node.bounds, "Scene3D bounds");
  const shader = normalizedExternalShader(CanvasKit, builtins, object);
  const paint = new CanvasKit.Paint();
  const localFromPixel = [
    bounds.width / image.width(),
    0,
    bounds.x,
    0,
    bounds.height / image.height(),
    bounds.y,
    0,
    0,
    1,
  ];
  try {
    paint.setShader(shader);
    canvas.save();
    canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    canvas.drawRect(CanvasKit.XYWHRect(0, 0, image.width(), image.height()), paint);
    canvas.restore();
  } finally {
    paint.delete();
    shader.delete();
  }
}

function applyProgramShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  admitted: AdmittedProgram,
  input: ImageValue,
  backdrops: ImageValue[],
  shaderWire: Wire,
  ownerToDevice: number[],
): void {
  const key = record(shaderWire.shader, "group shader key");
  const effect = required(admitted.shaders, String(key.uri), "group shader");
  const runtime = instantiateRuntimeShader(
    CanvasKit,
    builtins,
    effect,
    rect(shaderWire.bounds, "group shader bounds"),
    array(shaderWire.uniforms, "group uniforms"),
    array(shaderWire.textures, "group textures"),
    admitted,
    input,
    ownerToDevice,
    backdrops,
  );
  const paint = new CanvasKit.Paint();
  const bounds = rect(shaderWire.bounds, "group shader bounds");
  const padding = numberArray(shaderWire.padding, 4, "shader padding");
  if (padding.some((v) => !Number.isSafeInteger(v) || v < 0))
    fail("shader_bounds", "shader padding must contain non-negative integers");
  const output = {
    x: bounds.x - padding[0]!,
    y: bounds.y - padding[1]!,
    width: bounds.width + padding[0]! + padding[2]!,
    height: bounds.height + padding[1]! + padding[3]!,
  };
  canvas.save();
  try {
    canvas.concat(ownerToDevice);
    // The output box's edges take antialiased coverage, as a path's do and as on Native, so a
    // rotated layer or one on a CSS 3D plane keeps smooth edges.
    paint.setAntiAlias(true);
    paint.setShader(runtime);
    canvas.drawRect(skRect(CanvasKit, output), paint);
  } finally {
    canvas.restore();
    paint.delete();
    runtime.delete();
  }
}

function instantiateRuntimeShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  effect: RuntimeEffect,
  bounds: RectWire,
  uniforms: unknown[],
  textures: unknown[],
  admitted: AdmittedProgram,
  content: ImageValue | null,
  ownerToDevice: number[] = IDENTITY_MATRIX,
  backdrops: ImageValue[] = [],
): Shader {
  // The content and the backdrops are snapshots in device pixels. The runtime entry and its child
  // evaluations use pixels relative to the shader's local border box, so each is sampled where it lies.
  const inPlace = (value: ImageValue | null): Shader => {
    if (!value?.image)
      return CanvasKit.Shader.MakeColor(CanvasKit.TRANSPARENT, CanvasKit.ColorSpace.SRGB);
    const deviceToOwner = inverse3(ownerToDevice);
    if (!deviceToOwner) fail("shader_transform", "shader owner transform is not invertible");
    const imageToShader = mul3(
      [1, 0, -bounds.x, 0, 1, -bounds.y, 0, 0, 1],
      mul3(deviceToOwner, [1, 0, value.roi.x, 0, 1, value.roi.y, 0, 0, 1]),
    );
    const image = value.image.makeShaderOptions(
      CanvasKit.TileMode.Decal,
      CanvasKit.TileMode.Decal,
      CanvasKit.FilterMode.Linear,
      CanvasKit.MipmapMode.None,
      imageToShader,
    );
    if (!image) fail("shader_transform", "an image cannot become a shader child");
    return image;
  };
  const child = inPlace(content);
  const supplied: Shader[] = [];
  try {
    for (const backdrop of backdrops) supplied.push(inPlace(backdrop));
  } catch (error) {
    child.delete();
    for (const shader of supplied) shader.delete();
    throw error;
  }
  return makeRuntimeShader(
    CanvasKit,
    builtins,
    effect,
    bounds,
    uniforms,
    textures,
    admitted,
    [child],
    [],
    [1, 0, bounds.x, 0, 1, bounds.y, 0, 0, 1],
    supplied,
  );
}

/**
 * A runtime shader over the images its node supplies, which it takes and deletes, then its
 * textures. `system` holds the uniforms the node supplies beside `resolution`, and `backdrops`
 * the images its backdrop textures read, in binding order; an invisible layer reads none.
 */
function makeRuntimeShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  effect: RuntimeEffect,
  bounds: RectWire,
  uniforms: unknown[],
  textures: unknown[],
  admitted: AdmittedProgram,
  inputs: Shader[],
  system: [string, number[]][],
  local?: number[],
  backdrops: Shader[] = [],
): Shader {
  const children = [...inputs];
  const pending = [...backdrops];
  try {
    const values = new Map<string, number[]>([
      ["resolution", [bounds.width, bounds.height]],
      ...system,
    ]);
    for (const raw of uniforms) {
      const binding = record(raw, "shader uniform");
      const value = record(binding.value, "shader uniform value");
      const scalars: number[] = [];
      if (value.kind === "float") scalars.push(finiteNumber(value.value, "shader float"));
      else if (value.kind === "float2") scalars.push(...pair(value.value, "shader float2"));
      else if (
        ["float3", "float4", "float2x2", "float3x3", "float4x4"].includes(String(value.kind))
      ) {
        const widths: Record<string, number> = {
          float3: 3,
          float4: 4,
          float2x2: 4,
          float3x3: 9,
          float4x4: 16,
        };
        const components = array(value.value, "shader components");
        if (components.length !== widths[String(value.kind)])
          fail("shader_abi", "shader component count mismatch");
        scalars.push(...components.map((v) => finiteNumber(v, "shader component")));
      } else if (value.kind === "color") {
        const components = array(value.value, "shader linear sRGB color");
        if (components.length !== 4) fail("shader_abi", "shader color requires four components");
        scalars.push(...components.map((v) => finiteNumber(v, "shader color component")));
      } else if (value.kind === "bool") scalars.push(value.value === true ? 1 : 0);
      else fail("shader_abi", `unknown shader uniform kind '${String(value.kind)}'`);
      const name =
        value.kind === "bool" ? `valle_uniform_${String(binding.name)}` : String(binding.name);
      if (values.has(name)) fail("shader_abi", `duplicate shader uniform '${name}'`);
      values.set(name, scalars);
    }
    for (const raw of textures) {
      const binding = record(raw, "shader texture binding");
      const name = `valle_size_${String(binding.name)}`;
      if (values.has(name))
        fail("shader_abi", `duplicate shader texture '${String(binding.name)}'`);
      const source = shaderTextureSource(binding);
      if (source === null) {
        values.set(name, [1, 1]);
        continue;
      }
      // The backdrop lies under the layer, at its size.
      if (source.type === "backdrop") {
        values.set(name, [bounds.width, bounds.height]);
        continue;
      }
      const object = resolveProgramTexture(
        admitted.textures,
        record(source.texture, "shader texture key"),
        "shader texture",
      );
      if (!object.image) fail("shader_abi", "shader texture needs an image");
      const data = object.kind === "dataTexture";
      values.set(name, [
        object.image.width() / (data ? 2 : 1),
        object.image.height() / (data ? 4 : 1),
      ]);
    }
    const scalars = new Float32Array(effect.getUniformFloatCount());
    for (let i = 0; i < effect.getUniformCount(); i++) {
      const name = effect.getUniformName(i);
      const uniform = effect.getUniform(i);
      const value = values.get(name);
      if (!value || value.length !== uniform.columns * uniform.rows)
        fail("shader_abi", `missing or mismatched shader uniform '${name}'`);
      scalars.set(value, uniform.slot);
      values.delete(name);
    }
    if (values.size) fail("shader_abi", "shader has undeclared uniform bindings");
    for (const raw of textures) {
      const binding = record(raw, "shader texture binding");
      const source = shaderTextureSource(binding);
      const backdrop = source?.type === "backdrop" ? pending.shift() : undefined;
      if (backdrop) {
        children.push(backdrop);
        continue;
      }
      if (source === null || source.type === "backdrop") {
        children.push(CanvasKit.Shader.MakeColor(CanvasKit.TRANSPARENT, CanvasKit.ColorSpace.SRGB));
        continue;
      }
      const texture = record(source.texture, "shader texture key");
      children.push(
        normalizedExternalShader(
          CanvasKit,
          builtins,
          resolveProgramTexture(admitted.textures, texture, "shader texture"),
          // Decode each texel before the shared shader performs linear interpolation.
          "nearestClamp",
          String(binding.wrap),
        ),
      );
    }
    const shader = effect.makeShaderWithChildren(scalars, children, local);
    if (!shader) fail("shader_abi", "RuntimeEffect child/uniform ABI mismatch");
    return shader;
  } finally {
    for (const child of children) child.delete();
    for (const backdrop of pending) backdrop.delete();
  }
}

/** What a shader texture binding samples: nothing, an external texture, or the backdrop. */
function shaderTextureSource(binding: Wire): Wire | null {
  if (binding.source == null) return null;
  const source = record(binding.source, "shader texture source");
  if (source.type !== "external" && source.type !== "backdrop") {
    fail("shader_abi", `unknown shader texture source '${String(source.type)}'`);
  }
  return source;
}

function executeKernel(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  canvas: Canvas,
  invocation: Wire,
  resources: Map<number, ImageValue>,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  switch (invocation.kind) {
    case "group":
      drawImage(CanvasKit, canvas, image(resources, invocation.input), 1, "src");
      return;
    case "mask":
      drawImage(CanvasKit, canvas, image(resources, invocation.input), 1, "src");
      applyPreparedMask(CanvasKit, arena, canvas, record(invocation.mask, "prepared mask"));
      return;
    case "filter":
    case "adjustmentEffect":
      applyPreparedEffect(
        CanvasKit,
        builtins,
        canvas,
        image(resources, invocation.input),
        record(invocation.effect, "effect"),
        dynamic,
        extent,
      );
      return;
    case "transition": {
      drawImage(CanvasKit, canvas, image(resources, invocation.backdrop), 1, "src");
      const progress = dynamicScalar(dynamic, invocation.progress);
      const prepared = preparedTransition(record(invocation.kernel, "transition kernel"));
      const kernel = prepared.kind;
      const from = opacityShader(
        CanvasKit,
        builtins,
        image(resources, invocation.from),
        dynamicScalar(dynamic, invocation.fromOpacity),
      );
      const to = opacityShader(
        CanvasKit,
        builtins,
        image(resources, invocation.to),
        dynamicScalar(dynamic, invocation.toOpacity),
      );
      try {
        const transition = builtins.shader(
          kernel,
          [extent.width, extent.height, progress, ...prepared.params],
          [from, to],
        );
        try {
          drawShader(CanvasKit, canvas, transition, "srcOver");
        } finally {
          transition.delete();
        }
      } finally {
        from.delete();
        to.delete();
      }
      return;
    }
    default:
      fail("unsupported_kernel", `kernel '${String(invocation.kind)}' is not closed`);
  }
}

/** @internal Exported only for backend pixel-contract tests; package entrypoints do not re-export it. */
export function applyPreparedEffect(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  effect: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  const kernel = record(effect.kernel, "effect kernel");
  const region = effectRegion(kernel);
  const space = record(effect.space, "effect space");
  const restricted = space.kind === "layer" || region !== null;
  if (restricted) {
    drawImage(CanvasKit, canvas, input, 1, "src");
    canvas.save();
    applyEffectClip(CanvasKit, canvas, effect, region, dynamic, extent);
  }
  try {
    drawPreparedEffect(CanvasKit, builtins, canvas, input, effect, kernel, dynamic, extent);
  } finally {
    if (restricted) canvas.restore();
  }
}

function drawPreparedEffect(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  effect: Wire,
  kernel: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  if (kernel.kind === "gaussianBlur") {
    const sigma = dynamicScalar(dynamic, kernel.sigmaDevicePx);
    const filter = CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null);
    const paint = new CanvasKit.Paint();
    try {
      paint.setImageFilter(filter);
      drawImageWithPaint(CanvasKit, canvas, input, paint, "src");
    } finally {
      filter?.delete();
      paint.delete();
    }
    return;
  }
  if (kernel.kind === "directionalBlur") {
    const direction = kernel.axis === "horizontal" ? [1, 0] : [0, 1];
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "directionalBlur",
      [direction[0]!, direction[1]!, dynamicScalar(dynamic, kernel.spanDevicePx)],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "colorGrade") {
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "colorGrade",
      [
        extent.width,
        extent.height,
        finiteNumber(kernel.brightness, "color grade brightness"),
        finiteNumber(kernel.contrast, "color grade contrast"),
        finiteNumber(kernel.saturation, "color grade saturation"),
        finiteNumber(kernel.temperature, "color grade temperature"),
        finiteNumber(kernel.vignette, "color grade vignette"),
      ],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "mosaic") {
    const anchor = effectAnchor(effect, dynamic);
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "mosaic",
      [
        extent.width,
        extent.height,
        dynamicScalar(dynamic, kernel.blockSizeDevicePx),
        anchor[0],
        anchor[1],
      ],
      [input],
      "src",
    );
    return;
  }
  if (kernel.kind === "spotlight") {
    const center = pair(kernel.center, "spotlight center");
    drawBuiltinImage(
      CanvasKit,
      builtins,
      canvas,
      "spotlight",
      [
        extent.width,
        extent.height,
        center[0],
        center[1],
        finiteNumber(kernel.radius, "spotlight radius"),
        finiteNumber(kernel.feather, "spotlight feather"),
        finiteNumber(kernel.intensity, "spotlight intensity"),
      ],
      [input],
      "src",
    );
    return;
  }
  fail("unsupported_kernel", `prepared effect '${String(kernel.kind)}' is not valid in this pass`);
}

function applyEffectClip(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  effect: Wire,
  region: Wire | null,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  const space = record(effect.space, "effect space");
  if (space.kind === "root") {
    if (region) {
      const value = unitRegion(region);
      canvas.clipRect(
        CanvasKit.XYWHRect(
          value.x * extent.width,
          value.y * extent.height,
          value.width * extent.width,
          value.height * extent.height,
        ),
        CanvasKit.ClipOp.Intersect,
        false,
      );
    }
    return;
  }
  if (space.kind !== "layer")
    fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
  canvas.clipRect(
    deviceRect(CanvasKit, dynamicBounds(dynamic, space.bounds)),
    CanvasKit.ClipOp.Intersect,
    false,
  );
  if (!region) return;

  const transform = dynamicTransform(dynamic, space.transform);
  const inverse = inverse3(transform);
  if (!inverse)
    fail(
      "effect_space",
      `effect '${String(effect.semanticPath)}' has a non-invertible layer transform`,
    );
  const value = unitRegion(region);
  canvas.concat(transform);
  canvas.clipRect(
    CanvasKit.XYWHRect(value.x, value.y, value.width, value.height),
    CanvasKit.ClipOp.Intersect,
    false,
  );
  canvas.concat(inverse);
}

function effectRegion(kernel: Wire): Wire | null {
  if (kernel.kind !== "gaussianBlur" && kernel.kind !== "mosaic") return null;
  return kernel.region == null ? null : record(kernel.region, "effect region");
}

function unitRegion(region: Wire): RectWire {
  const value = rect(region, "effect region");
  if (
    value.x < 0 ||
    value.y < 0 ||
    value.width <= 0 ||
    value.height <= 0 ||
    value.x + value.width > 1 ||
    value.y + value.height > 1
  ) {
    fail("effect_region", "effect region must be a non-empty normalized unit rectangle");
  }
  return value;
}

function effectAnchor(effect: Wire, dynamic: Map<number, Wire>): [number, number] {
  const space = record(effect.space, "effect space");
  if (space.kind === "root") return [0, 0];
  if (space.kind !== "layer")
    fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
  const anchor = project3(dynamicTransform(dynamic, space.transform), [0, 0]);
  if (!anchor)
    fail("effect_space", `effect '${String(effect.semanticPath)}' has an invalid layer origin`);
  return anchor;
}

function applyPreparedMask(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  canvas: Canvas,
  mask: Wire,
): void {
  const transform = record(mask.deviceFromMask, "mask transform");
  const surface = arena.transient();
  const maskCanvas = surface.getCanvas();
  const shape = new CanvasKit.Paint();
  let snapshot: Image | null = null;
  try {
    maskCanvas.clear(mask.invert ? CanvasKit.WHITE : CanvasKit.TRANSPARENT);
    shape.setAntiAlias(true);
    shape.setBlendMode(CanvasKit.BlendMode.Src);
    shape.setColor(mask.invert ? CanvasKit.TRANSPARENT : CanvasKit.WHITE);
    maskCanvas.save();
    maskCanvas.concat(numberArray(transform.matrix, 9, "mask matrix"));
    if (mask.shape === "ellipse") maskCanvas.drawOval(CanvasKit.XYWHRect(0, 0, 1, 1), shape);
    else maskCanvas.drawRect(CanvasKit.XYWHRect(0, 0, 1, 1), shape);
    maskCanvas.restore();
    surface.flush();
    snapshot = surface.makeImageSnapshot();
  } finally {
    shape.delete();
    arena.releaseTransient(surface);
  }
  const coverage = new CanvasKit.Paint();
  const sigma = finiteNumber(mask.featherSigmaDevicePx, "mask feather sigma");
  const blur =
    sigma > 0 ? CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null) : null;
  try {
    coverage.setBlendMode(CanvasKit.BlendMode.DstIn);
    if (blur) coverage.setImageFilter(blur);
    canvas.drawImage(snapshot, 0, 0, coverage);
  } finally {
    blur?.delete();
    coverage.delete();
    snapshot.delete();
  }
}

function renderImport(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  sourceObject: CanvasKitExternalObject,
  pass: Wire,
  dynamic: Map<number, Wire>,
): void {
  const source = sourceObject.image!;
  const placement = record(pass.placement, "external placement");
  const transform = dynamicTransform(dynamic, pass.transform);
  const bounds = dynamicBounds(dynamic, pass.bounds);
  canvas.save();
  canvas.clipRect(deviceRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, false);
  const backdrop =
    placement.backdrop == null ? null : record(placement.backdrop, "external backdrop");
  if (backdrop?.kind === "color") {
    canvas.save();
    canvas.concat(transform);
    const paint = new CanvasKit.Paint();
    try {
      const [red, green, blue, alpha] = numberArray(
        backdrop.workingLinearRec2020Premul,
        4,
        "color",
      ) as [number, number, number, number];
      if (alpha > 0) setWorkingColor(CanvasKit, paint, red, green, blue, alpha);
      else setWorkingColor(CanvasKit, paint, 0, 0, 0, 0);
      canvas.drawRect(CanvasKit.XYWHRect(0, 0, 1, 1), paint);
    } finally {
      paint.delete();
      canvas.restore();
    }
  } else if (backdrop?.kind === "blur") {
    const sigma = dynamicScalar(dynamic, backdrop.sigmaDevicePx);
    const blur = CanvasKit.ImageFilter.MakeBlur(sigma, sigma, CanvasKit.TileMode.Decal, null);
    const paint = new CanvasKit.Paint();
    const shader = normalizedExternalShader(CanvasKit, builtins, sourceObject);
    try {
      paint.setImageFilter(blur);
      drawExternalSample(
        CanvasKit,
        canvas,
        source,
        record(backdrop.sample, "external blur sample"),
        rect(placement.clipRect, "blur destination"),
        rect(placement.clipRect, "blur clip"),
        transform,
        paint,
        shader,
      );
    } finally {
      shader.delete();
      paint.delete();
      blur.delete();
    }
  }
  const sourcePipeline = record(pass.sourcePipeline, "source pipeline");
  let sourceShader = normalizedExternalShader(CanvasKit, builtins, sourceObject);
  if (sourcePipeline.chromaKey != null) {
    const effect = record(sourcePipeline.chromaKey, "chroma key effect");
    const kernel = record(effect.kernel, "chroma key kernel");
    if (kernel.kind !== "chromaKey")
      fail("source_pipeline", "chroma key slot carries the wrong kernel");
    const key = numberArray(kernel.keyWorkingLinearRec2020, 3, "chroma key color");
    const child = sourceShader;
    try {
      sourceShader = builtins.shader(
        "chromaKey",
        [
          key[0]!,
          key[1]!,
          key[2]!,
          finiteNumber(kernel.intensity, "chroma key intensity"),
          finiteNumber(kernel.shadow, "chroma key shadow"),
          dynamicScalar(dynamic, kernel.featherSigmaDevicePx),
          finiteNumber(kernel.edgeClean, "chroma key edge clean"),
        ],
        [child],
      );
    } finally {
      child.delete();
    }
  }
  try {
    drawExternalSample(
      CanvasKit,
      canvas,
      source,
      record(placement.sample, "external sample"),
      rect(placement.contentRect, "content rect"),
      rect(placement.clipRect, "clip rect"),
      transform,
      undefined,
      sourceShader,
    );
  } finally {
    sourceShader.delete();
  }
  canvas.restore();
}

function drawExternalSample(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  image: Image,
  sample: Wire,
  destination: RectWire,
  clip: RectWire,
  transform: number[],
  paintOverride?: Paint,
  shaderOverride?: Shader,
): void {
  if (sample.kind === "empty") return;
  if (sample.kind !== "texture")
    fail("external_sample", `unknown sample kind '${String(sample.kind)}'`);
  const textureFromContent = numberArray(sample.textureFromContent, 9, "textureFromContent");
  const inverse = inverse3(textureFromContent);
  if (!inverse) fail("external_sample", "non-invertible texture mapping");
  const normalizedFromPixel = [1 / image.width(), 0, 0, 0, 1 / image.height(), 0, 0, 0, 1];
  const localFromContent = [
    destination.width,
    0,
    destination.x,
    0,
    destination.height,
    destination.y,
    0,
    0,
    1,
  ];
  const localFromPixel = mul3(localFromContent, mul3(inverse, normalizedFromPixel));
  const sourceBounds = rect(sample.inputSampleBounds, "input sample bounds");
  const src = CanvasKit.XYWHRect(
    sourceBounds.x * image.width(),
    sourceBounds.y * image.height(),
    sourceBounds.width * image.width(),
    sourceBounds.height * image.height(),
  );
  const paint = paintOverride ?? new CanvasKit.Paint();
  try {
    canvas.save();
    canvas.concat(transform);
    canvas.clipRect(skRect(CanvasKit, clip), CanvasKit.ClipOp.Intersect, true);
    canvas.clipRect(skRect(CanvasKit, destination), CanvasKit.ClipOp.Intersect, true);
    canvas.concat(localFromPixel);
    if (shaderOverride) {
      paint.setShader(shaderOverride);
      canvas.drawRect(src, paint);
      paint.setShader(null);
    } else {
      canvas.drawImageRectOptions(
        image,
        src,
        src,
        CanvasKit.FilterMode.Linear,
        CanvasKit.MipmapMode.None,
        paint,
      );
    }
    canvas.restore();
  } finally {
    if (!paintOverride) paint.delete();
  }
}

function applyFiltersAsLayerValue(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  input: ImageValue,
  filters: unknown[],
  bounds: RectWire | null,
): void {
  let current: ImageFilter | null = null;
  try {
    for (const raw of filters) {
      const filter = record(raw, "DrawProgram filter");
      const next = drawProgramImageFilter(CanvasKit, filter, current);
      if (next !== current) current?.delete();
      current = next;
    }
    const paint = new CanvasKit.Paint();
    try {
      if (current) paint.setImageFilter(current);
      if (bounds) canvas.clipRect(skRect(CanvasKit, bounds), CanvasKit.ClipOp.Intersect, true);
      drawImageValueWithPaint(CanvasKit, canvas, input, paint, "src");
    } finally {
      paint.delete();
    }
  } finally {
    current?.delete();
  }
}

function drawProgramImageFilter(
  CanvasKit: CanvasKit,
  filter: Wire,
  input: ImageFilter | null,
): ImageFilter | null {
  if (filter.kind === "blur") {
    const sigmaX = finiteNumber(filter.sigmaX, "sigmaX");
    const sigmaY = finiteNumber(filter.sigmaY, "sigmaY");
    return sigmaX === 0 && sigmaY === 0
      ? input
      : CanvasKit.ImageFilter.MakeBlur(sigmaX, sigmaY, CanvasKit.TileMode.Decal, input);
  }
  const matrix = drawProgramColorMatrix(filter);
  if (matrix) {
    const color = CanvasKit.ColorFilter.MakeMatrix(matrix);
    try {
      return CanvasKit.ImageFilter.MakeColorFilter(color, input);
    } finally {
      color.delete();
    }
  }
  if (filter.kind === "dropShadow") {
    const offset = pair(filter.offset, "drop shadow offset");
    return CanvasKit.ImageFilter.MakeDropShadow(
      offset[0],
      offset[1],
      finiteNumber(filter.sigmaX, "sigmaX"),
      finiteNumber(filter.sigmaY, "sigmaY"),
      linearColor(
        CanvasKit,
        record(filter.color, "drop shadow color") as unknown as LinearColorWire,
      ),
      input,
    );
  }
  if (filter.kind === "glow") {
    fail("unsupported_filter", "glow requires a scheduled F16 pass");
  }
  if (filter.kind === "bloom") {
    fail("unsupported_filter", "bloom requires a scheduled F16 pass");
  }
  if (filter.kind === "radialBlur") {
    fail("unsupported_filter", "radial blur requires a scheduled F16 pass");
  }
  if (filter.kind === "filmGrain") {
    fail("unsupported_filter", "film grain requires a scheduled F16 pass");
  }
  if (filter.kind === "lensDistortion") {
    fail("unsupported_filter", "lens distortion requires a scheduled F16 pass");
  }
  if (filter.kind === "chromaticAberration") {
    fail("unsupported_filter", "chromatic aberration requires a scheduled shader pass");
  }
  if (filter.kind === "noiseDisplacement") {
    fail("unsupported_filter", "noise displacement requires its kernel");
  }
  if (filter.kind === "velocityBlur") {
    const from = pair(filter.from, "velocity blur trail start");
    const to = pair(filter.to, "velocity blur trail end");
    const reach = Math.max(Math.abs(from[0]), Math.abs(from[1]), Math.abs(to[0]), Math.abs(to[1]));
    if (reach <= 1e-6) return input;
    const sampling = { filter: CanvasKit.FilterMode.Nearest, mipmap: CanvasKit.MipmapMode.None };
    // As Filter::velocity_blur_offsets places them: even steps of the exposure, half from `from`
    // to the input's place and half from there to `to`; a copy for every 2px of travel along the
    // longer half, 9 to 32.
    const samples = Math.min(32, Math.max(9, Math.ceil(reach) + 1));
    const weight = 1 / samples;
    const alpha = CanvasKit.ColorFilter.MakeMatrix([
      1,
      0,
      0,
      0,
      0,
      0,
      1,
      0,
      0,
      0,
      0,
      0,
      1,
      0,
      0,
      0,
      0,
      0,
      weight,
      0,
    ]);
    let sum: ImageFilter | null = null;
    try {
      for (let index = 0; index < samples; index++) {
        const phase = (2 * index) / (samples - 1) - 1;
        const end = phase < 0 ? from : to;
        const share = Math.abs(phase);
        const shifted = CanvasKit.ImageFilter.MakeMatrixTransform(
          CanvasKit.Matrix.translated(end[0] * share, end[1] * share),
          sampling,
          input,
        );
        let tap: ImageFilter;
        try {
          tap = CanvasKit.ImageFilter.MakeColorFilter(alpha, shifted);
        } finally {
          shifted.delete();
        }
        if (sum) {
          const previous = sum;
          sum = null;
          try {
            sum = CanvasKit.ImageFilter.MakeBlend(CanvasKit.BlendMode.Plus, previous, tap);
          } finally {
            previous.delete();
            tap.delete();
          }
        } else sum = tap;
      }
      return sum;
    } catch (error) {
      sum?.delete();
      throw error;
    } finally {
      alpha.delete();
    }
  }
  fail("unsupported_filter", `DrawProgram filter '${String(filter.kind)}' has no CanvasKit kernel`);
}

/**
 * A backdrop chain that ends in noise displacement, drawn over `outputRoi` under the canvas clip:
 * the kernel Native shares displaces by noise and vectors in device pixels, reading what the
 * filters before it produce within its reach of `outputRoi`.
 */
function drawDisplacedBackdrop(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  engineKernels: CanvasKitEngineKernels | null,
  arena: SurfaceArena,
  canvas: Canvas,
  input: ImageValue,
  filters: unknown[],
  displacement: Record<string, unknown>,
  outputRoi: DeviceRoi,
): void {
  const tables = engineKernels?.perlin_noise_tables;
  if (!tables) fail("noise_displacement_tables", "noise displacement tables are unavailable");
  const noise = tables.call(engineKernels, positiveIdOrZero(displacement.seed, "noise seed"));
  let source = input;
  let surface: Surface | null = null;
  try {
    if (filters.length > 0 && input.image) {
      // A pixel reads its nearest sample at most half the scale away on each axis.
      const reach =
        Math.ceil(Math.abs(finiteNumber(displacement.scale, "noise displacement scale")) * 0.5) + 1;
      const roi = {
        x: outputRoi.x - reach,
        y: outputRoi.y - reach,
        width: outputRoi.width + 2 * reach,
        height: outputRoi.height + 2 * reach,
      };
      surface = arena.transient(roi.width, roi.height);
      const layer = surface.getCanvas();
      layer.clear(CanvasKit.TRANSPARENT);
      layer.save();
      try {
        layer.translate(-roi.x, -roi.y);
        applyFiltersAsLayerValue(CanvasKit, layer, input, filters, null);
      } finally {
        layer.restore();
      }
      if (!arena.gpu || !arena.deferGpuFlush) surface.flush();
      source = {
        image: surface.makeImageSnapshot([0, 0, roi.width, roi.height]),
        roi,
        owned: true,
      };
    }
    drawNoiseDisplacement(
      CanvasKit,
      builtins,
      canvas,
      source,
      displacement,
      IDENTITY_MATRIX,
      noise,
    );
  } finally {
    if (source !== input) source.image?.delete();
    if (surface) arena.releaseTransient(surface);
  }
}

/**
 * Displace a scheduled pass input, drawn in device pixels, by the kernel Native shares: the noise
 * lattice and its vectors are in the units `localToDevice` maps to device pixels, a DrawProgram
 * group's local units or device pixels themselves, and only the input's ROI is sampled. The
 * kernel computes Skia's noise in float from `tables`, the engine's Perlin tables for the filter's
 * seed: CanvasKit's own noise differs left of and above the lattice's origin. A degenerate
 * transform leaves the input undisplaced.
 */
export function drawNoiseDisplacement(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: ImageValue,
  filter: Record<string, unknown>,
  localToDevice: readonly number[],
  tables: Uint8Array,
): void {
  if (!input.image || input.roi.width === 0 || input.roi.height === 0) return;
  const [a, b, c, d, e, f] = affineDeviceTransform(localToDevice, "noise displacement").map(
    checkedDeviceFilterValue,
  ) as [number, number, number, number, number, number];
  const determinant = a * e - b * d;
  const [ia, ib, id, ie] = [e / determinant, -b / determinant, -d / determinant, a / determinant];
  const { x, y, width, height } = input.roi;
  const uniforms = [
    ia,
    ib,
    id,
    ie,
    -(ia * c + ib * f),
    -(id * c + ie * f),
    a,
    b,
    d,
    e,
    finiteNumber(filter.scale, "noise displacement scale"),
    x,
    y,
    x + width,
    y + height,
  ];
  if (
    determinant === 0 ||
    uniforms.some((value) => !Number.isFinite(value) || Math.abs(value) > 3.4028234663852886e38)
  ) {
    drawImageValue(CanvasKit, canvas, input, 1, "src");
    return;
  }
  const frequency = pair(filter.frequency, "noise frequency");
  const octaves = positiveIdOrZero(filter.octaves, "noise octaves");
  const source = input.image.makeShaderOptions(
    CanvasKit.TileMode.Decal,
    CanvasKit.TileMode.Decal,
    CanvasKit.FilterMode.Nearest,
    CanvasKit.MipmapMode.None,
    [1, 0, x, 0, 1, y, 0, 0, 1],
  );
  if (!source)
    fail("noise_displacement_shader", "noise displacement input shader allocation failed");
  let lattice: Image | null = null;
  let noise: Shader | null = null;
  let shader: Shader | null = null;
  try {
    // The tables' bytes as they are: the image and CanvasKit's surfaces are both sRGB.
    lattice = CanvasKit.MakeImage(
      {
        width: 256,
        height: 5,
        colorType: CanvasKit.ColorType.RGBA_8888,
        alphaType: CanvasKit.AlphaType.Opaque,
        colorSpace: CanvasKit.ColorSpace.SRGB,
      },
      tables,
      256 * 4,
    );
    if (!lattice)
      fail("noise_displacement_tables", "noise displacement tables image allocation failed");
    noise = lattice.makeShaderOptions(
      CanvasKit.TileMode.Clamp,
      CanvasKit.TileMode.Clamp,
      CanvasKit.FilterMode.Nearest,
      CanvasKit.MipmapMode.None,
    );
    if (!noise)
      fail("noise_displacement_tables", "noise displacement tables shader allocation failed");
    shader = builtins.shader(
      "noiseDisplacement",
      [...uniforms, frequency[0], frequency[1], octaves, filter.turbulence === true ? 1 : 0],
      [source, noise],
    );
    drawShader(CanvasKit, canvas, shader, "src");
  } finally {
    shader?.delete();
    noise?.delete();
    lattice?.delete();
    source.delete();
  }
}

/**
 * Map program-local Blur/DropShadow parameters into a materialized device-space pass.
 * Direct-raster groups keep the local CTM active and do not call this helper. The scheduled
 * fallback has axis-aligned CanvasKit image-filter kernels. Caption similarities are exact; any
 * other affine transform is accepted only when its mapped Gaussian covariance remains
 * axis-aligned.
 */
export function programFilterInDeviceSpace(
  rawFilter: Record<string, unknown>,
  rawLocalToDevice: readonly number[],
): Record<string, unknown> {
  const filter = record(rawFilter, "program filter");
  if (
    filter.kind !== "blur" &&
    filter.kind !== "dropShadow" &&
    filter.kind !== "glow" &&
    filter.kind !== "bloom" &&
    filter.kind !== "radialBlur" &&
    filter.kind !== "filmGrain" &&
    filter.kind !== "chromaticAberration" &&
    filter.kind !== "velocityBlur"
  )
    return filter;
  const matrix = numberArray(rawLocalToDevice, 9, "program filter local-to-device matrix");
  if (filter.kind === "filmGrain") {
    const size = finiteNumber(filter.size, "film grain size");
    const [sx, sy] = programGaussianInDeviceSpace([size, size], matrix);
    if (Math.abs(sx - sy) > Math.max(sx, sy, 1) * 1e-9)
      fail("unsupported_filter", "film grain size needs a similarity device transform");
    return { ...filter, size: checkedDeviceFilterValue(Math.max(1, (sx + sy) * 0.5)) };
  }
  if (filter.kind === "radialBlur") {
    const amount = finiteNumber(filter.amount, "radial blur amount");
    const [sx, sy] = programGaussianInDeviceSpace([amount, amount], matrix);
    if (Math.abs(sx - sy) > Math.max(sx, sy, 1) * 1e-9)
      fail("unsupported_filter", "radial blur amount needs a similarity device transform");
    const [cx, cy] = pair(filter.center, "radial blur center");
    return {
      ...filter,
      center: [
        checkedDeviceFilterValue(matrix[0]! * cx + matrix[1]! * cy + matrix[2]!),
        checkedDeviceFilterValue(matrix[3]! * cx + matrix[4]! * cy + matrix[5]!),
      ],
      amount: checkedDeviceFilterValue((sx + sy) * 0.5),
    };
  }
  if (filter.kind === "chromaticAberration") {
    return {
      ...filter,
      offset: programVectorInDeviceSpace(
        pair(filter.offset, "chromatic aberration offset"),
        matrix,
        "chromatic aberration",
      ),
    };
  }
  if (filter.kind === "velocityBlur") {
    return {
      ...filter,
      from: programVectorInDeviceSpace(
        pair(filter.from, "velocity blur trail start"),
        matrix,
        "velocity blur",
      ),
      to: programVectorInDeviceSpace(
        pair(filter.to, "velocity blur trail end"),
        matrix,
        "velocity blur",
      ),
    };
  }
  if (filter.kind === "glow" || filter.kind === "bloom") {
    const radius = finiteNumber(filter.radius, `${filter.kind} radius`);
    const sigma = programGaussianInDeviceSpace([radius, radius], matrix);
    if (Math.abs(sigma[0] - sigma[1]) > Math.max(sigma[0], sigma[1], 1) * 1e-9)
      fail("unsupported_filter", `${filter.kind} radius needs a similarity device transform`);
    return { ...filter, radius: checkedDeviceFilterValue((sigma[0] + sigma[1]) * 0.5) };
  }
  const sigma = programGaussianInDeviceSpace(
    [finiteNumber(filter.sigmaX, "sigmaX"), finiteNumber(filter.sigmaY, "sigmaY")],
    matrix,
  );
  if (filter.kind === "blur") return { ...filter, sigmaX: sigma[0], sigmaY: sigma[1] };
  const offset = pair(filter.offset, "drop shadow offset");
  return {
    ...filter,
    offset: [
      checkedDeviceFilterValue(matrix[0]! * offset[0] + matrix[1]! * offset[1]),
      checkedDeviceFilterValue(matrix[3]! * offset[0] + matrix[4]! * offset[1]),
    ],
    sigmaX: sigma[0],
    sigmaY: sigma[1],
  };
}

/**
 * Map a backdrop chain's lengths, node units as in `filter`, into the device pixels its pass
 * filters in, through `backdropFilterMatrix`. Noise displacement stays in output pixels.
 */
export function backdropFiltersInDeviceSpace(
  filters: readonly unknown[],
  matrix: readonly number[],
  bounds: RectWire,
): Record<string, unknown>[] {
  return filters.map((raw) => {
    const filter = record(raw, "backdrop filter");
    return filter.kind === "noiseDisplacement"
      ? filter
      : programFilterInDeviceSpace(filter, backdropFilterMatrix(filter, matrix, bounds));
  });
}

/**
 * The transform a backdrop filter maps its lengths with. One that `programFilterInDeviceSpace`
 * represents for the filter maps them exactly, as for a node's `filter`; a skewed or projective
 * one gives way to `backdropSimilarity` at the centre of `bounds`, so the filter keeps its shape
 * instead of failing. Native chooses the same way.
 */
export function backdropFilterMatrix(
  filter: Wire,
  matrix: readonly number[],
  bounds: RectWire,
): readonly number[] {
  let exact = true;
  try {
    if (filter.kind === "lensDistortion") {
      // Lens distortion has no lengths, but its frame needs a similarity.
      const [x, y] = programGaussianInDeviceSpace([1, 1], matrix);
      exact = Math.abs(x - y) <= Math.max(x, y, 1) * 1e-9;
    } else {
      programFilterInDeviceSpace(filter, matrix);
    }
  } catch (error) {
    if (!(error instanceof CanvasKitCompositorError) || error.code !== "unsupported_filter")
      throw error;
    exact = false;
  }
  return exact
    ? matrix
    : (backdropSimilarity(matrix, [
        bounds.x + bounds.width * 0.5,
        bounds.y + bounds.height * 0.5,
      ]) ?? matrix);
}

/** The transform an ApplyFilter pass maps its filter with: a backdrop stage's, or the node's own. */
function programFilterMatrix(kind: Wire, exact: number[]): number[] {
  return kind.backdrop == null
    ? exact
    : [
        ...backdropFilterMatrix(
          record(kind.filter, "program filter"),
          exact,
          rect(kind.backdrop, "backdrop stage bounds"),
        ),
      ];
}

/**
 * The similarity nearest `matrix` at local point `at`: the Jacobian there, with its rotation or
 * reflection kept and its scale the square root of its area scale, translated to map `at` where
 * the transform does. A similarity returns itself. `null` when `at` is not in front of the camera.
 * Native computes it with the same operations, so both agree to the bit.
 */
export function backdropSimilarity(
  matrix: readonly number[],
  at: readonly [number, number],
): number[] | null {
  const [a, b, c, d, e, f, g, h, i] = matrix as [
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
  ];
  const [x, y] = at;
  const deviceX = a * x + b * y + c;
  const deviceY = d * x + e * y + f;
  const w = g * x + h * y + i;
  if (!(w > 0)) return null;
  const w2 = w * w;
  const j11 = (a * w - deviceX * g) / w2;
  const j12 = (b * w - deviceX * h) / w2;
  const j21 = (d * w - deviceY * g) / w2;
  const j22 = (e * w - deviceY * h) / w2;
  const det = j11 * j22 - j12 * j21;
  const scale = Math.sqrt(Math.abs(det));
  const [p, q]: [number, number] = det >= 0 ? [j11 + j22, j21 - j12] : [j11 - j22, j21 + j12];
  const k = scale === 0 ? 0 : scale / Math.sqrt(p * p + q * q);
  const [l11, l12, l21, l22]: [number, number, number, number] =
    det >= 0 ? [k * p, -(k * q), k * q, k * p] : [k * p, k * q, k * q, -(k * p)];
  const tx = deviceX / w - (l11 * x + l12 * y);
  const ty = deviceY / w - (l21 * x + l22 * y);
  const similarity = [l11, l12, tx, l21, l22, ty, 0, 0, 1];
  return similarity.every(Number.isFinite) ? similarity : null;
}

/** The 2 x 3 part of a local-to-device transform that has no projective row. */
function affineDeviceTransform(
  matrix: readonly number[],
  what: string,
): [number, number, number, number, number, number] {
  if (
    matrix.length !== 9 ||
    matrix.some((value) => !Number.isFinite(value)) ||
    Math.abs(matrix[6]!) > 1e-12 ||
    Math.abs(matrix[7]!) > 1e-12 ||
    Math.abs(matrix[8]! - 1) > 1e-12
  ) {
    fail("unsupported_filter", `${what} needs an affine device transform`);
  }
  return [matrix[0]!, matrix[1]!, matrix[2]!, matrix[3]!, matrix[4]!, matrix[5]!];
}

function programVectorInDeviceSpace(
  vector: [number, number],
  matrix: readonly number[],
  what: string,
): [number, number] {
  const [a, b, , d, e] = affineDeviceTransform(matrix, what);
  return [
    checkedDeviceFilterValue(a * vector[0] + b * vector[1]),
    checkedDeviceFilterValue(d * vector[0] + e * vector[1]),
  ];
}

function programGaussianInDeviceSpace(
  sigma: [number, number],
  matrix: readonly number[],
): [number, number] {
  const [a, b, , d, e, , g, h, i] = matrix as [
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
  ];
  if (
    matrix.some((value) => !Number.isFinite(value)) ||
    Math.abs(g) > 1e-12 ||
    Math.abs(h) > 1e-12 ||
    Math.abs(i - 1) > 1e-12
  ) {
    fail("unsupported_filter", "program-local spatial filter requires an affine device transform");
  }
  const varianceX = (a * sigma[0]) ** 2 + (b * sigma[1]) ** 2;
  const varianceY = (d * sigma[0]) ** 2 + (e * sigma[1]) ** 2;
  const covariance = a * d * sigma[0] ** 2 + b * e * sigma[1] ** 2;
  const varianceScale = Math.max(varianceX, varianceY, 1);
  if (Math.abs(covariance) > varianceScale * 1e-9) {
    fail(
      "unsupported_filter",
      "rotated program-local anisotropic blur has no axis-aligned CanvasKit equivalent",
    );
  }
  return [
    checkedDeviceFilterValue(Math.sqrt(varianceX)),
    checkedDeviceFilterValue(Math.sqrt(varianceY)),
  ];
}

function checkedDeviceFilterValue(value: number): number {
  if (!Number.isFinite(value) || Math.abs(value) > 3.4028234663852886e38) {
    fail("unsupported_filter", "program-local spatial filter exceeds the device numeric domain");
  }
  return value;
}

function drawProgramColorMatrix(filter: Wire): number[] | null {
  switch (filter.kind) {
    case "colorMatrix":
      return numberArray(filter.matrix, 20, "color matrix");
    case "brightness": {
      const amount = finiteNumber(filter.amount, "brightness amount");
      return diagonalColorMatrix(amount, amount, amount, 0);
    }
    case "contrast": {
      const amount = finiteNumber(filter.amount, "contrast amount");
      return diagonalColorMatrix(amount, amount, amount, 0.5 - 0.5 * amount);
    }
    case "grayscale":
      return grayscaleColorMatrix(finiteNumber(filter.amount, "grayscale amount"));
    case "hueRotate":
      return hueRotateColorMatrix(finiteNumber(filter.degrees, "hue rotation"));
    case "invert": {
      const amount = finiteNumber(filter.amount, "invert amount");
      return diagonalColorMatrix(1 - 2 * amount, 1 - 2 * amount, 1 - 2 * amount, amount);
    }
    case "opacity": {
      const matrix = diagonalColorMatrix(1, 1, 1, 0);
      matrix[18] = finiteNumber(filter.amount, "opacity amount");
      return matrix;
    }
    case "saturate":
      return saturateColorMatrix(finiteNumber(filter.amount, "saturation amount"));
    case "sepia":
      return sepiaColorMatrix(finiteNumber(filter.amount, "sepia amount"));
    default:
      return null;
  }
}

function diagonalColorMatrix(red: number, green: number, blue: number, offset: number): number[] {
  return [red, 0, 0, 0, offset, 0, green, 0, 0, offset, 0, 0, blue, 0, offset, 0, 0, 0, 1, 0];
}

function grayscaleColorMatrix(amount: number): number[] {
  const a = amount;
  return [
    0.2126 + 0.7874 * (1 - a),
    0.7152 - 0.7152 * (1 - a),
    0.0722 - 0.0722 * (1 - a),
    0,
    0,
    0.2126 - 0.2126 * (1 - a),
    0.7152 + 0.2848 * (1 - a),
    0.0722 - 0.0722 * (1 - a),
    0,
    0,
    0.2126 - 0.2126 * (1 - a),
    0.7152 - 0.7152 * (1 - a),
    0.0722 + 0.9278 * (1 - a),
    0,
    0,
    0,
    0,
    0,
    1,
    0,
  ];
}

function saturateColorMatrix(amount: number): number[] {
  return [
    0.213 + 0.787 * amount,
    0.715 - 0.715 * amount,
    0.072 - 0.072 * amount,
    0,
    0,
    0.213 - 0.213 * amount,
    0.715 + 0.285 * amount,
    0.072 - 0.072 * amount,
    0,
    0,
    0.213 - 0.213 * amount,
    0.715 - 0.715 * amount,
    0.072 + 0.928 * amount,
    0,
    0,
    0,
    0,
    0,
    1,
    0,
  ];
}

function hueRotateColorMatrix(degrees: number): number[] {
  const radians = (degrees * Math.PI) / 180;
  const cosine = Math.cos(radians);
  const sine = Math.sin(radians);
  return [
    0.213 + cosine * 0.787 - sine * 0.213,
    0.715 - cosine * 0.715 - sine * 0.715,
    0.072 - cosine * 0.072 + sine * 0.928,
    0,
    0,
    0.213 - cosine * 0.213 + sine * 0.143,
    0.715 + cosine * 0.285 + sine * 0.14,
    0.072 - cosine * 0.072 - sine * 0.283,
    0,
    0,
    0.213 - cosine * 0.213 - sine * 0.787,
    0.715 - cosine * 0.715 + sine * 0.715,
    0.072 + cosine * 0.928 + sine * 0.072,
    0,
    0,
    0,
    0,
    0,
    1,
    0,
  ];
}

function sepiaColorMatrix(amount: number): number[] {
  const inverse = 1 - amount;
  return [
    0.393 * amount + inverse,
    0.769 * amount,
    0.189 * amount,
    0,
    0,
    0.349 * amount,
    0.686 * amount + inverse,
    0.168 * amount,
    0,
    0,
    0.272 * amount,
    0.534 * amount,
    0.131 * amount + inverse,
    0,
    0,
    0,
    0,
    0,
    1,
    0,
  ];
}

function drawPathClipCoverage(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  draw: DrawProgramWire,
  clip: Wire,
  cache?: RasterPathCache,
): void {
  const value = record(clip.value, "path clip");
  const wire = requiredIndex(draw.paths, value.path, "clip path"),
    fillRule = String(value.fillRule);
  const lease = cache?.acquire(wire, fillRule, () => buildPath(CanvasKit, wire, fillRule));
  const path = lease?.path ?? buildPath(CanvasKit, wire, fillRule);
  const paint = new CanvasKit.Paint();
  try {
    paint.setAntiAlias(true);
    paint.setColor(CanvasKit.WHITE);
    canvas.drawPath(path, paint);
  } finally {
    paint.delete();
    if (!lease || lease.owned) path.delete();
  }
}

export function applyClip(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  draw: DrawProgramWire,
  clip: Wire,
  matrix: number[],
): void {
  // Clip geometry is local, while scheduled images are already in device space.
  // Transform the path without changing the canvas matrix used to draw those images.
  let builder: InstanceType<CanvasKit["PathBuilder"]>;
  if (clip.kind === "path") {
    const value = record(clip.value, "path clip");
    const path = buildPath(
      CanvasKit,
      requiredIndex(draw.paths, value.path, "clip path"),
      String(value.fillRule),
    );
    try {
      builder = new CanvasKit.PathBuilder(path);
    } finally {
      path.delete();
    }
  } else {
    builder = new CanvasKit.PathBuilder();
    if (clip.kind === "rect") builder.addRect(skRect(CanvasKit, rect(clip.value, "clip rect")));
    else if (clip.kind === "roundRect")
      builder.addRRect(roundRect(CanvasKit, record(clip.value, "round clip")));
    else {
      builder.delete();
      fail("unsupported_clip", `clip '${String(clip.kind)}' is not closed`);
    }
  }
  builder.transform(matrix);
  const path = builder.detachAndDelete();
  try {
    canvas.clipPath(path, CanvasKit.ClipOp.Intersect, true);
  } finally {
    path.delete();
  }
}

function buildPath(
  CanvasKit: CanvasKit,
  wire: { verbs: string[]; points: Array<[number, number]> },
  fillRule: string,
): Path {
  // Submit the same f32 path commands in one Wasm call instead of crossing Embind for every
  // verb. MakeFromCmds runs Skia's builder, retaining contour normalization and curve coverage.
  const commands = new Float32Array(wire.verbs.length + wire.points.length * 2);
  let point = 0,
    offset = 0;
  for (const verb of wire.verbs) {
    let count: number;
    if (verb === "moveTo") {
      commands[offset++] = CanvasKit.MOVE_VERB;
      count = 1;
    } else if (verb === "lineTo") {
      commands[offset++] = CanvasKit.LINE_VERB;
      count = 1;
    } else if (verb === "quadTo") {
      commands[offset++] = CanvasKit.QUAD_VERB;
      count = 2;
    } else if (verb === "cubicTo") {
      commands[offset++] = CanvasKit.CUBIC_VERB;
      count = 3;
    } else if (verb === "close") {
      commands[offset++] = CanvasKit.CLOSE_VERB;
      count = 0;
    } else fail("path_verb", `unknown path verb '${verb}'`);
    for (let i = 0; i < count; i++) {
      const p = wire.points[point++]!;
      commands[offset++] = p[0];
      commands[offset++] = p[1];
    }
  }
  const path = CanvasKit.Path.MakeFromCmds(commands.subarray(0, offset));
  if (!path) fail("path_allocation", "cannot construct admitted DrawProgram path");
  path.setFillType(
    fillRule === "evenOdd" ? CanvasKit.FillType.EvenOdd : CanvasKit.FillType.Winding,
  );
  return path;
}

export function makePaint(CanvasKit: CanvasKit, raw: unknown): Paint {
  const wire = record(raw, "DrawProgram paint");
  const paint = new CanvasKit.Paint();
  paint.setAntiAlias(true);
  try {
    if (wire.kind === "solid") {
      setLinearColor(
        CanvasKit,
        paint,
        record(wire.value, "solid color") as unknown as LinearColorWire,
      );
      return paint;
    }
    const value = record(wire.value, `${String(wire.kind)} paint`);
    const stops = array(value.stops, "gradient stops").map((rawStop) =>
      record(rawStop, "gradient stop"),
    );
    const colors = stops.map((stop) =>
      linearColor(
        CanvasKit,
        record(stop.color, "gradient stop color") as unknown as LinearColorWire,
      ),
    );
    const positions = stops.map((stop) => finiteNumber(stop.offset, "gradient stop offset"));
    const mode = tileMode(CanvasKit, String(value.spread));
    let shader: Shader | null = null;
    if (wire.kind === "linearGradient") {
      shader = CanvasKit.Shader.MakeLinearGradient(
        pair(value.start, "linear gradient start"),
        pair(value.end, "linear gradient end"),
        colors,
        positions,
        mode,
        undefined,
        1,
        CanvasKit.ColorSpace.SRGB,
      );
    } else if (wire.kind === "radialGradient") {
      const center = pair(value.center, "radial gradient center");
      const radii = pair(value.radii, "radial gradient radii");
      const radius = Math.max(radii[0], radii[1]);
      if (!(radius > 0)) fail("gradient", "radial gradient radius must be positive");
      shader = CanvasKit.Shader.MakeRadialGradient(
        center,
        radius,
        colors,
        positions,
        mode,
        CanvasKit.Matrix.scaled(radii[0] / radius, radii[1] / radius, center[0], center[1]),
        1,
        CanvasKit.ColorSpace.SRGB,
      );
    } else if (wire.kind === "twoCircleGradient") {
      shader = CanvasKit.Shader.MakeTwoPointConicalGradient(
        pair(value.start, "gradient start"),
        finiteNumber(value.startRadius, "gradient start radius"),
        pair(value.end, "gradient end"),
        finiteNumber(value.endRadius, "gradient end radius"),
        colors,
        positions,
        mode,
        undefined,
        1,
        CanvasKit.ColorSpace.SRGB,
      );
    } else if (wire.kind === "conicGradient") {
      const center = pair(value.center, "conic gradient center");
      const start = finiteNumber(value.startAngleDegrees, "conic gradient start angle");
      // Skia measures sweep angles in [0, 360) from the positive x axis, so the start angle
      // rotates the shader: as a sweep start it would clamp the turn before it to an end color.
      shader = CanvasKit.Shader.MakeSweepGradient(
        center[0],
        center[1],
        colors,
        positions,
        mode,
        CanvasKit.Matrix.rotated((start * Math.PI) / 180, center[0], center[1]),
        1,
        0,
        finiteNumber(value.sweepAngleDegrees, "conic gradient sweep angle"),
        CanvasKit.ColorSpace.SRGB,
      );
    } else {
      fail("unsupported_paint", `paint '${String(wire.kind)}' is not closed in CanvasKit`);
    }
    if (!shader) fail("gradient", `${String(wire.kind)} shader creation failed`);
    paint.setShader(shader);
    shader.delete();
    return paint;
  } catch (error) {
    paint.delete();
    throw error;
  }
}

function roundRect(CanvasKit: CanvasKit, wire: Wire): Float32Array {
  const bounds = rect(wire.rect, "round rect bounds");
  const radii = array(wire.radii, "round rect radii").map((raw) => pair(raw, "round rect radius"));
  if (radii.length !== 4) fail("round_rect", "round rect needs four radii");
  return Float32Array.from([
    bounds.x,
    bounds.y,
    bounds.x + bounds.width,
    bounds.y + bounds.height,
    radii[0]![0],
    radii[0]![1],
    radii[1]![0],
    radii[1]![1],
    radii[2]![0],
    radii[2]![1],
    radii[3]![0],
    radii[3]![1],
  ]);
}

function configureStrokePaint(CanvasKit: CanvasKit, paint: Paint, stroke: Wire) {
  paint.setStyle(CanvasKit.PaintStyle.Stroke);
  paint.setStrokeWidth(finiteNumber(stroke.width, "stroke width"));
  paint.setStrokeCap(
    stroke.cap === "round"
      ? CanvasKit.StrokeCap.Round
      : stroke.cap === "square"
        ? CanvasKit.StrokeCap.Square
        : CanvasKit.StrokeCap.Butt,
  );
  paint.setStrokeJoin(
    stroke.join === "round"
      ? CanvasKit.StrokeJoin.Round
      : stroke.join === "bevel"
        ? CanvasKit.StrokeJoin.Bevel
        : CanvasKit.StrokeJoin.Miter,
  );
  paint.setStrokeMiter(finiteNumber(stroke.miterLimit, "stroke miter"));
  const dash = array(stroke.dash, "stroke dash").map((value) => finiteNumber(value, "dash value"));
  const effect = dash.length
    ? CanvasKit.PathEffect.MakeDash(dash, finiteNumber(stroke.dashOffset, "dash offset"))
    : null;
  if (effect) paint.setPathEffect(effect);
  return effect;
}

function tileMode(CanvasKit: CanvasKit, spread: string) {
  if (spread === "pad") return CanvasKit.TileMode.Clamp;
  if (spread === "repeat") return CanvasKit.TileMode.Repeat;
  if (spread === "reflect") return CanvasKit.TileMode.Mirror;
  fail("gradient", `gradient spread '${spread}' is outside the closed set`);
}

function programMatrix(
  admitted: ProgramStructure,
  localPlan: Wire,
  output: unknown,
  device: number[],
): number[] {
  const id = positiveId(output, "program output resource");
  const resource = array(localPlan.resources, "localPlan.resources")
    .map((value) => record(value, "local resource"))
    .find((value) => value.id === id);
  if (!resource) fail("program_resource", `local resource ${id} is absent`);
  return programTransformMatrix(
    admitted,
    numberArray(resource.localToProgram, 9, "localToProgram"),
    device,
  );
}

/**
 * Lens distortion's frame in device pixels: the bounds, under the node's exact transform, of a
 * backdrop stage's border box or else of the uncropped input. `matrix`, the transform the pass
 * filters with, must be a similarity. Native maps the frame the same way.
 */
function programLensFrame(
  localPlan: Wire,
  kind: Wire,
  matrix: number[],
  exact: number[],
): [number, number, number, number] {
  const [sx, sy] = programGaussianInDeviceSpace([1, 1], matrix);
  if (Math.abs(sx - sy) > Math.max(sx, sy, 1) * 1e-9)
    fail("unsupported_filter", "lens distortion needs a similarity device transform");
  let source: RectWire;
  if (kind.backdrop == null) {
    const id = positiveId(kind.input, "lens input resource");
    const resources = array(localPlan.resources, "localPlan.resources");
    const resource = record(
      requiredIndex(resources, id - 1, "lens input resource"),
      "lens input resource",
    );
    if (resource.id !== id) fail("program_resource", "lens input resource identity is invalid");
    const bounds = record(resource.bounds, "lens input bounds");
    if (bounds.kind !== "finite")
      fail("unsupported_filter", "lens distortion has no finite source frame");
    source = rect(bounds.rect, "lens source frame");
  } else {
    source = rect(kind.backdrop, "lens backdrop frame");
  }
  const corners = [
    [source.x, source.y],
    [source.x + source.width, source.y],
    [source.x + source.width, source.y + source.height],
    [source.x, source.y + source.height],
  ];
  const w = corners.map(([x, y]) => exact[6]! * x! + exact[7]! * y! + exact[8]!);
  if (
    w.some(
      (value) =>
        !Number.isFinite(value) ||
        Math.abs(value) <= Number.EPSILON ||
        Math.sign(value) !== Math.sign(w[0]!),
    )
  )
    fail("unsupported_filter", "lens distortion has no finite source frame");
  const points = corners.map(([x, y], i) => [
    (exact[0]! * x! + exact[1]! * y! + exact[2]!) / w[i]!,
    (exact[3]! * x! + exact[4]! * y! + exact[5]!) / w[i]!,
  ]);
  const left = Math.min(...points.map((point) => point[0]!));
  const top = Math.min(...points.map((point) => point[1]!));
  const right = Math.max(...points.map((point) => point[0]!));
  const bottom = Math.max(...points.map((point) => point[1]!));
  return [left, top, right - left, bottom - top].map(checkedDeviceFilterValue) as [
    number,
    number,
    number,
    number,
  ];
}

function programTransformMatrix(
  admitted: ProgramStructure,
  localToProgram: number[],
  device: number[],
): number[] {
  const viewport = admitted.draw.viewport;
  if (!(viewport.width > 0) || !(viewport.height > 0))
    fail("program_viewport", "DrawProgram viewport is empty during execution");
  const normalize = [
    1 / viewport.width,
    0,
    -viewport.x / viewport.width,
    0,
    1 / viewport.height,
    -viewport.y / viewport.height,
    0,
    0,
    1,
  ];
  return mul3(device, mul3(normalize, localToProgram));
}

/** The working-surface envelope of one frame, and where its surfaces are next needed. */
interface SurfaceFrame {
  readonly envelopeBytes: bigint;
  /** Lends bytes beyond the envelope, as `CanvasKitExecutionTarget.borrowSurfaceBytes`. */
  readonly borrow: ((bytes: bigint) => boolean) | null;
  /** Without a plan, any slot or output the frame has not used yet may be evicted. */
  readonly liveness: SurfaceLiveness | null;
}

interface PooledSlot {
  readonly surface: Surface;
  readonly width: number;
  readonly height: number;
  /** The program the slot belongs to, or 0 for an outer slot. */
  readonly program: number;
  readonly slot: number;
}

class SurfaceArena {
  trees: RasterTreeCache | null = null;
  treeSignatures = new Map<number, RasterTreeSignature | null>();
  recordingClipPaints = false;
  onLocalPassTiming: ((timing: CanvasKitPassTiming) => void) | null = null;
  onRasterLeafTiming: ((kind: string, milliseconds: number) => void) | null = null;
  deferGpuFlush = false;
  boundedPathClipLayers = false;
  fuseClipPaints = false;
  pathClipElisions = 0;
  geometryRenderer: GpuInstances | null = null;
  yieldToPresentation: (() => Promise<void>) | null = null;
  private readonly slots = new Map<string, PooledSlot>();
  private readonly freeTransients = new Map<Surface, { width: number; height: number }>();
  private readonly checkedOutTransients = new Map<Surface, { width: number; height: number }>();
  private readonly frameTransients = new Set<Surface>();
  /** Backings of `boundedTransient`, whose consumers read only the leading extent they asked for. */
  private readonly boundedBackings = new Set<Surface>();
  private readonly rasterMemory = new Map<Surface, MallocObj>();
  private readonly surfaceBytes = new Map<Surface, bigint>();
  private readonly usedSlotKeys = new Set<string>();
  private outputSurface: Surface | null = null;
  private outputUsed = false;
  private gpuCacheLimit = -1;
  private readonly surfaceKinds = new Map<Surface, CanvasKitSurfaceKind>();
  private readonly residentByKind: Record<CanvasKitSurfaceKind, bigint> = {
    output: 0n,
    outer: 0n,
    program: 0n,
    transient: 0n,
  };
  residentBytes = 0n;
  frameAllocations = 0;
  frameReuses = 0;
  frameAllocatedBytes = 0n;
  framePeakBytes = 0n;
  /** Resident bytes by surface role at the moment of the frame peak. */
  framePeakByKind: Record<CanvasKitSurfaceKind, bigint> = { ...this.residentByKind };
  frameEvictions = 0;
  /** Live surface bytes beyond the frame's envelope at their highest point. */
  frameOverflowBytes = 0n;
  /** The outer pass being run, and the program running inside it. */
  outerPass = 0;
  activeProgram = 0;
  private liveness: SurfaceLiveness | null = null;
  /** The frame may hold its envelope or what the pool carried into it, whichever is larger. */
  private allowance = 0n;
  private envelope = 0n;
  private borrowed = 0n;
  private borrow: ((bytes: bigint) => boolean) | null = null;
  get gpu(): boolean {
    return this.context !== null;
  }
  /** CanvasKit 0.42's public cache getters drop their return values, so read usage through the
   * bound method; the limit is the one this arena configured. */
  gpuCacheReport(): { gpuCacheUsageBytes: number; gpuCacheLimitBytes: number } {
    if (!this.context) return { gpuCacheUsageBytes: 0, gpuCacheLimitBytes: 0 };
    const bound = this.context as unknown as { _getResourceCacheUsageBytes(): number };
    return {
      gpuCacheUsageBytes: Number(bound._getResourceCacheUsageBytes()),
      gpuCacheLimitBytes: this.gpuCacheLimit,
    };
  }
  constructor(
    private readonly CanvasKit: CanvasKit,
    private readonly context: GrDirectContext | null,
    private readonly width: number,
    private readonly height: number,
    private readonly maxSurfaceBytes: bigint,
    private readonly maxFrameBytes: bigint,
  ) {
    this.configureGpuCache(0n);
  }
  private configureGpuCache(residentBytes: bigint): void {
    if (!this.context) return;
    // Circle batch targets are pinned outside this pool.
    const pinned = residentBytes + BigInt(this.geometryRenderer?.stats.allocatedBytes ?? 0);
    const limit = canvasKitGpuCacheBudget(this.maxFrameBytes, pinned);
    if (limit === this.gpuCacheLimit) return;
    this.context.setResourceCacheLimitBytes(limit);
    this.gpuCacheLimit = limit;
  }
  matches(
    context: GrDirectContext | null,
    width: number,
    height: number,
    maxSurfaceBytes: bigint,
    maxFrameBytes: bigint,
  ): boolean {
    return (
      this.context === context &&
      this.width === width &&
      this.height === height &&
      this.maxSurfaceBytes === maxSurfaceBytes &&
      this.maxFrameBytes === maxFrameBytes
    );
  }
  beginFrame(frame: SurfaceFrame): void {
    this.pathClipElisions = 0;
    if (this.checkedOutTransients.size !== 0)
      fail("surface_lifetime", "transient surface escaped the prior frame");
    this.usedSlotKeys.clear();
    this.frameTransients.clear();
    this.outputUsed = false;
    this.frameAllocations = 0;
    this.frameReuses = 0;
    this.frameAllocatedBytes = 0n;
    this.framePeakBytes = this.residentBytes;
    this.framePeakByKind = { ...this.residentByKind };
    this.frameEvictions = 0;
    this.frameOverflowBytes = 0n;
    this.outerPass = 0;
    this.activeProgram = 0;
    this.liveness = frame.liveness;
    this.envelope = frame.envelopeBytes;
    this.allowance = maxBigInt(frame.envelopeBytes, this.residentBytes);
    this.borrowed = 0n;
    this.borrow = frame.borrow;
    this.configureGpuCache(this.residentBytes);
  }
  endFrame(): void {
    this.borrow = null;
    const leaked = this.checkedOutTransients.size;
    if (leaked !== 0) {
      // Restore the arena before rejecting the frame so one failed draw cannot poison every later
      // seek. Snapshots own their pixels independently; checked-out surfaces remain arena-owned.
      for (const [surface, dimensions] of this.checkedOutTransients) {
        this.freeTransients.set(surface, dimensions);
      }
      this.checkedOutTransients.clear();
    }
    if (leaked !== 0)
      fail("surface_lifetime", `${leaked} transient surface(s) escaped frame execution`);
  }
  /**
   * Keep only the working set of the frame that just executed. Ganesh retains a deleted backing as
   * purgeable scratch within the fixed cache headroom, so a size that returns is reacquired without
   * a new GPU allocation, while one that does not return stops pinning memory.
   */
  releaseUnused(): void {
    this.releaseCarriedOver();
    const slots = [...this.slots];
    for (const [key, entry] of slots) {
      if (this.usedSlotKeys.has(key)) continue;
      this.slots.delete(key);
      this.evictSurface(entry.surface);
    }
    if (!this.outputUsed && this.outputSurface) {
      this.evictSurface(this.outputSurface);
      this.outputSurface = null;
    }
    this.configureGpuCache(this.residentBytes);
  }
  prepareReport(): void {
    if (this.checkedOutTransients.size !== 0) {
      fail("surface_lifetime", "transient surface escaped successful frame execution");
    }
  }
  get count(): number {
    return (
      this.slots.size +
      this.freeTransients.size +
      this.checkedOutTransients.size +
      Number(this.outputSurface !== null)
    );
  }
  slot(id: number, width: number, height: number): Surface {
    return this.keyedSlot(`outer:${id}`, width, height, 0, id);
  }
  programSlot(program: number, id: number, width: number, height: number): Surface {
    return this.keyedSlot(`program:${program}:${id}`, width, height, program, id);
  }
  private keyedSlot(
    key: string,
    width: number,
    height: number,
    program: number,
    slot: number,
  ): Surface {
    this.usedSlotKeys.add(key);
    const existing = this.slots.get(key);
    // Physical extents follow the plan exactly, as on Native GPU. Snapshots cover the whole backing,
    // so a larger backing left by an earlier frame would let history change what consumers sample.
    if (existing && existing.width === width && existing.height === height) {
      this.frameReuses += 1;
      return existing.surface;
    }
    if (existing) {
      this.slots.delete(key);
      this.evictSurface(existing.surface);
    }
    const surface = this.create(width, height, program === 0 ? "outer" : "program");
    this.slots.set(key, { surface, width, height, program, slot });
    return surface;
  }
  output(): Surface {
    this.outputUsed = true;
    if (this.outputSurface) {
      this.frameReuses += 1;
      return this.outputSurface;
    }
    return (this.outputSurface = this.create(this.width, this.height, "output"));
  }
  /**
   * A transient for consumers that read only [0, width) x [0, height): a free backing of the
   * extent's 64-pixel class, else a new one. Extents that move with content, such as each Shutter
   * sample of a spinning glow, then share a few classes instead of allocating per extent, and the
   * frame plan holds each checkout at its class. Exact when the class does not fit the budgets.
   */
  boundedTransient(width: number, height: number): Surface {
    const dimensions = { width: Math.ceil(width / 64) * 64, height: Math.ceil(height / 64) * 64 };
    let surface: Surface | null = null;
    for (const candidate of this.boundedBackings) {
      const free = this.freeTransients.get(candidate);
      if (free && free.width === dimensions.width && free.height === dimensions.height) {
        surface = candidate;
        break;
      }
    }
    if (surface) {
      this.freeTransients.delete(surface);
      this.frameReuses += 1;
    } else {
      const bytes = BigInt(dimensions.width) * BigInt(dimensions.height) * 8n;
      if (bytes > this.maxSurfaceBytes || this.residentBytes + bytes > this.maxFrameBytes)
        return this.transient(width, height);
      surface = this.create(dimensions.width, dimensions.height);
      this.boundedBackings.add(surface);
    }
    this.checkedOutTransients.set(surface, dimensions);
    this.frameTransients.add(surface);
    return surface;
  }
  transient(width = this.width, height = this.height): Surface {
    let surface: Surface | null = null;
    for (const [candidate, dimensions] of this.freeTransients) {
      if (dimensions.width === width && dimensions.height === height) {
        surface = candidate;
        break;
      }
    }
    if (surface) {
      this.freeTransients.delete(surface);
      this.frameReuses += 1;
    } else {
      // An extent this frame has not seen means the working set moved on. Backings carried over
      // from the previous frame and not reused yet go back to Ganesh, which keeps them as purgeable
      // scratch within the cache headroom for any size that comes back. Bounded backings, and
      // extents the frame's plan still checks out, stay until the frame ends without them.
      this.releaseCarriedOver(true);
      surface = this.create(width, height);
    }
    this.checkedOutTransients.set(surface, { width, height });
    this.frameTransients.add(surface);
    return surface;
  }
  private releaseCarriedOver(midFrame = false): void {
    const planned = midFrame ? this.liveness?.transientExtents : undefined;
    const transients = [...this.freeTransients];
    for (const [surface, { width, height }] of transients) {
      if (
        this.frameTransients.has(surface) ||
        (midFrame && this.boundedBackings.has(surface)) ||
        planned?.has(`${width}x${height}`)
      )
        continue;
      this.freeTransients.delete(surface);
      this.evictSurface(surface);
    }
  }
  releaseTransient(surface: Surface): void {
    const dimensions = this.checkedOutTransients.get(surface);
    if (!dimensions || !this.checkedOutTransients.delete(surface)) {
      fail("surface_lifetime", "transient surface was released twice or by the wrong arena");
    }
    this.freeTransients.set(surface, dimensions);
  }
  delete(): void {
    const all = new Set<Surface>([
      ...[...this.slots.values()].map((entry) => entry.surface),
      ...this.freeTransients.keys(),
      ...this.checkedOutTransients.keys(),
      ...(this.outputSurface ? [this.outputSurface] : []),
    ]);
    for (const surface of all) this.deleteSurface(surface);
    this.slots.clear();
    this.freeTransients.clear();
    this.checkedOutTransients.clear();
    this.frameTransients.clear();
    this.outputSurface = null;
  }
  private create(width: number, height: number, kind: CanvasKitSurfaceKind = "transient"): Surface {
    const bytes = BigInt(width) * BigInt(height) * 8n;
    if (width <= 0 || height <= 0 || bytes <= 0n || bytes > this.maxSurfaceBytes) {
      fail(
        "surface_budget",
        `${width}x${height} working surface needs ${bytes} bytes, budget ${this.maxSurfaceBytes}`,
      );
    }
    this.ensureCapacity(bytes);
    this.configureGpuCache(this.residentBytes + bytes);
    const info = {
      width,
      height,
      colorType: this.CanvasKit.ColorType.RGBA_F16,
      alphaType: this.CanvasKit.AlphaType.Premul,
      colorSpace: this.CanvasKit.ColorSpace.SRGB,
    };
    let memory: MallocObj | null = null;
    let surface: Surface | null;
    if (this.context) {
      surface = this.CanvasKit.MakeRenderTarget(this.context, info);
    } else {
      const rowBytes = width * 8;
      const byteLength = rowBytes * height;
      if (!Number.isSafeInteger(byteLength) || byteLength <= 0) {
        fail("surface_allocation", "working F16 byte length is outside the exact integer range");
      }
      memory = this.CanvasKit.Malloc(Uint8Array, byteLength);
      surface = this.CanvasKit.MakeRasterDirectSurface(info, memory, rowBytes);
      if (!surface) this.CanvasKit.Free(memory);
    }
    if (!surface) fail("surface_allocation", `cannot allocate ${width}x${height} working surface`);
    if (memory) this.rasterMemory.set(surface, memory);
    this.surfaceBytes.set(surface, bytes);
    this.surfaceKinds.set(surface, kind);
    this.residentBytes += bytes;
    this.residentByKind[kind] += bytes;
    this.frameAllocations += 1;
    this.frameAllocatedBytes += bytes;
    if (this.residentBytes > this.framePeakBytes) {
      this.framePeakBytes = this.residentBytes;
      this.framePeakByKind = { ...this.residentByKind };
    }
    return surface;
  }
  /**
   * Keep the pool inside the frame's allowance. What the frame never uses again goes first: a free
   * transient of an extent it does not check out, a slot of another extent or of a program that runs
   * without it, an unwritten output it does not need. Surfaces it uses again stay while the budget
   * lends their room; otherwise free transients go, carried-over ones first, then the slot or output
   * needed farthest ahead. Only live surfaces beyond the plan are overflow. Without a lender the pool
   * may then grow up to `maxFrameBytes`.
   */
  private ensureCapacity(requiredBytes: bigint): void {
    let lends = this.borrow !== null;
    while (this.residentBytes + requiredBytes > this.allowance + this.borrowed) {
      const missing = this.residentBytes + requiredBytes - this.allowance - this.borrowed;
      const unused = this.evictionVictim(false);
      if (unused) {
        this.evictSurface(unused);
        continue;
      }
      if (lends && this.residentBytes + requiredBytes <= this.maxFrameBytes) {
        if (this.borrow!(missing)) {
          this.borrowed += missing;
          continue;
        }
        lends = false;
      }
      const needed = this.evictionVictim(true);
      if (needed) {
        this.evictSurface(needed);
        continue;
      }
      if (this.residentBytes + requiredBytes > this.maxFrameBytes) {
        fail(
          "frame_budget",
          `working surface pool needs ${this.residentBytes + requiredBytes} bytes, budget ${this.maxFrameBytes}`,
        );
      }
      if (this.borrow && !this.borrow(missing)) {
        fail(
          "frame_budget",
          `live working surfaces need ${missing} bytes beyond the frame's ${this.allowance}-byte envelope`,
        );
      }
      this.borrowed += missing;
    }
    const overflow = this.liveBytes() + requiredBytes - this.envelope;
    if (overflow > this.frameOverflowBytes) this.frameOverflowBytes = overflow;
  }
  /** Checked-out transients, live slots and the written output. */
  private liveBytes(): bigint {
    let bytes = 0n;
    for (const surface of this.checkedOutTransients.keys())
      bytes += this.surfaceBytes.get(surface) ?? 0n;
    for (const [key, entry] of this.slots)
      if (this.nextUse(key, entry) === null) bytes += this.surfaceBytes.get(entry.surface) ?? 0n;
    if (this.outputSurface && this.outputUsed)
      bytes += this.surfaceBytes.get(this.outputSurface) ?? 0n;
    return bytes;
  }
  /** Remove and return a surface to evict: one the frame never uses again, or with `needed` any. */
  private evictionVictim(needed: boolean): Surface | null {
    const planned = this.liveness?.transientExtents;
    let free: Surface | null = null,
      unused = false;
    for (const [surface, { width, height }] of this.freeTransients) {
      if (planned && !planned.has(`${width}x${height}`)) {
        free = surface;
        unused = true;
        break;
      }
      if (!free || (this.frameTransients.has(free) && !this.frameTransients.has(surface)))
        free = surface;
    }
    let victim: string | null = null,
      farthest = -1;
    for (const [key, entry] of this.slots) {
      const next = this.nextUse(key, entry);
      if (next !== null && next > farthest) {
        victim = key;
        farthest = next;
      }
    }
    const output =
      this.outputSurface && !this.outputUsed ? (this.liveness?.outputPass ?? Infinity) : -1;
    if (free && (unused || (needed && Math.max(farthest, output) !== Infinity))) {
      this.freeTransients.delete(free);
      return free;
    }
    if (!needed && Math.max(farthest, output) !== Infinity) return null;
    if (output > farthest) {
      const surface = this.outputSurface!;
      this.outputSurface = null;
      return surface;
    }
    if (victim === null) return null;
    const surface = this.slots.get(victim)!.surface;
    this.slots.delete(victim);
    return surface;
  }
  /** The outer pass that next needs a pooled slot, or null while the slot is live. */
  private nextUse(key: string, entry: PooledSlot): number | null {
    if (!this.liveness) return this.usedSlotKeys.has(key) ? null : Infinity;
    // A backing of another extent, or of a program this frame runs without it, is never reused.
    const extent = this.liveness.slotExtents.get(entry.program)?.get(entry.slot);
    if (!extent || extent.width !== entry.width || extent.height !== entry.height) return Infinity;
    if (entry.program !== 0) {
      if (entry.program === this.activeProgram) return null;
      const pass = this.liveness.programPasses.get(entry.program);
      return pass === undefined || pass < this.outerPass ? Infinity : pass;
    }
    let next = Infinity;
    for (const [first, last] of this.liveness.slotIntervals.get(entry.slot) ?? []) {
      if (first <= this.outerPass && this.outerPass <= last) return null;
      if (first > this.outerPass && first < next) next = first;
    }
    return next;
  }
  private evictSurface(surface: Surface): void {
    this.frameEvictions += 1;
    this.deleteSurface(surface);
  }
  private deleteSurface(surface: Surface): void {
    const bytes = this.surfaceBytes.get(surface) ?? 0n;
    surface.delete();
    const memory = this.rasterMemory.get(surface);
    if (memory) {
      this.CanvasKit.Free(memory);
      this.rasterMemory.delete(surface);
    }
    this.surfaceBytes.delete(surface);
    this.boundedBackings.delete(surface);
    this.residentBytes -= bytes;
    const kind = this.surfaceKinds.get(surface);
    if (kind) this.residentByKind[kind] -= bytes;
    this.surfaceKinds.delete(surface);
  }
}

interface BoundOuterSchedule {
  readonly slotExtents: Map<number, { width: number; height: number } | null>;
  /** Each slot's allocation intervals, as inclusive 1-based pass indices. */
  readonly slotIntervals: Map<number, Array<readonly [number, number]>>;
  /** Bytes of the outer slots allocated during each pass, before program surfaces. */
  readonly outerActiveBytes: readonly bigint[];
  readonly programs: Map<number, BoundProgramRuntimeSchedule>;
  readonly passes: readonly BoundExecutionPass[];
}

interface BoundExecutionPass {
  readonly output: number;
  readonly deviceRoi: DeviceRoi;
  readonly surfaceSlot: number | null;
  readonly retireAfter: readonly number[];
}

function admitBoundOuterSchedule(
  plan: Wire,
  programs: Wire[],
  schedule: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): BoundOuterSchedule {
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  const resourceRois = new Map<number, DeviceRoi>();
  const planResources = array(plan.resources, "plan.resources");
  const boundResources = array(schedule.resources, "schedule.resources");
  if (planResources.length !== boundResources.length)
    fail("bound_schedule", "resource count mismatch");
  for (let index = 0; index < planResources.length; index += 1) {
    const resource = record(planResources[index], `plan.resources[${index}]`);
    const id = positiveId(resource.id, `plan.resources[${index}].id`);
    const roiWire = record(resource.roi, `plan.resources[${index}].roi`);
    const expected =
      roiWire.kind === "fullFrame"
        ? root
        : roiWire.kind === "static"
          ? deviceRoi(record(roiWire.rect, "static ROI"), extent)
          : roiWire.kind === "dynamic"
            ? deviceRoi(dynamicBounds(dynamic, roiWire.binding), extent)
            : fail("bound_schedule", `unknown ROI kind '${String(roiWire.kind)}'`);
    const actual = record(boundResources[index], `schedule.resources[${index}]`);
    if (
      positiveId(actual.resource, "bound resource id") !== id ||
      !sameRoi(deviceRoi(record(actual.deviceRoi, "bound device ROI"), extent), expected)
    ) {
      fail("bound_schedule", `resource ${id} ROI is not the exact plan/binding projection`);
    }
    resourceRois.set(id, expected);
  }

  const slotExtents = new Map<number, { width: number; height: number } | null>();
  const slotIntervals = new Map<number, Array<readonly [number, number]>>();
  const slotByResource = new Map<number, number>();
  const planSlots = array(plan.surfaceSlots, "plan.surfaceSlots");
  const boundSlots = array(schedule.surfaceSlots, "schedule.surfaceSlots");
  if (planSlots.length !== boundSlots.length) fail("bound_schedule", "surface slot count mismatch");
  const finalOutput = positiveId(plan.output, "plan.output");
  const activeBytes = Array.from({ length: array(plan.passes, "plan.passes").length }, () => 0n);
  const retireAfter = activeBytes.map(() => [] as number[]);
  for (let index = 0; index < planSlots.length; index += 1) {
    const slot = record(planSlots[index], `plan.surfaceSlots[${index}]`);
    const bound = record(boundSlots[index], `schedule.surfaceSlots[${index}]`);
    const id = positiveId(slot.id, "surface slot id");
    if (positiveId(bound.id, "bound surface slot id") !== id)
      fail("bound_schedule", "surface slot IDs diverge");
    const allocations = array(slot.allocations, "surface allocations");
    const boundAllocations = array(bound.allocations, "bound surface allocations");
    if (allocations.length !== boundAllocations.length)
      fail("bound_schedule", `slot ${id} allocation count mismatch`);
    let width = 0;
    let height = 0;
    for (let allocationIndex = 0; allocationIndex < allocations.length; allocationIndex += 1) {
      const allocation = record(allocations[allocationIndex], "surface allocation");
      const actual = record(boundAllocations[allocationIndex], "bound surface allocation");
      const resource = positiveId(allocation.resource, "surface allocation resource");
      const roi = required(resourceRois, resource, "bound resource ROI");
      if (
        positiveId(actual.resource, "bound allocation resource") !== resource ||
        String(actual.reason) !== String(allocation.reason) ||
        !deepEqual(actual.interval, allocation.interval) ||
        !sameRoi(deviceRoi(record(actual.deviceRoi, "bound allocation ROI"), extent), roi)
      ) {
        fail("bound_schedule", `slot ${id} allocation ${allocationIndex} diverges from the plan`);
      }
      const regional =
        allocation.reason === "layerIntermediate" ||
        allocation.reason === "backdropResolve" ||
        allocation.reason === "programIntermediate";
      width = Math.max(width, regional ? roi.width : extent.width);
      height = Math.max(height, regional ? roi.height : extent.height);
      slotByResource.set(resource, id);
    }
    const expectedExtent = width === 0 || height === 0 ? null : { width, height };
    const actualExtent =
      bound.extent == null
        ? null
        : {
            width: positiveId(record(bound.extent, "bound slot extent").width, "bound slot width"),
            height: positiveId(
              record(bound.extent, "bound slot extent").height,
              "bound slot height",
            ),
          };
    if (!deepEqual(actualExtent, expectedExtent))
      fail("bound_schedule", `slot ${id} extent is not derived from its allocations`);
    const bytes = BigInt(width) * BigInt(height) * 8n;
    if (BigInt(positiveIdOrZero(bound.estimatedBytes, "bound slot bytes")) !== bytes) {
      fail("bound_schedule", `slot ${id} byte estimate mismatch`);
    }
    for (const rawAllocation of allocations) {
      const allocation = record(rawAllocation, "surface allocation");
      const resource = positiveId(allocation.resource, "surface allocation resource");
      const interval = record(allocation.interval, "surface interval");
      const first = positiveId(interval.first, "surface interval first");
      const last = positiveId(interval.last, "surface interval last");
      if (first > last || last > activeBytes.length) {
        fail("bound_schedule", `slot ${id} allocation interval escapes the pass list`);
      }
      for (let pass = first; pass <= last; pass += 1)
        activeBytes[pass - 1] = (activeBytes[pass - 1] ?? 0n) + bytes;
      if (bytes !== 0n) slotIntervals.set(id, [...(slotIntervals.get(id) ?? []), [first, last]]);
      if (resource !== finalOutput) {
        const retired = retireAfter[last - 1];
        if (!retired) fail("bound_schedule", `slot ${id} allocation ends outside the pass list`);
        retired.push(resource);
      }
    }
    slotExtents.set(id, expectedExtent);
  }
  const outerPeak = activeBytes.reduce((peak, value) => (value > peak ? value : peak), 0n);
  if (BigInt(positiveIdOrZero(schedule.outerPeakSurfaceBytes, "outer peak bytes")) !== outerPeak) {
    fail("bound_schedule", "outer peak byte estimate mismatch");
  }
  const outerActiveBytes = [...activeBytes];
  const boundPrograms = admitBoundProgramSchedules(
    plan,
    programs,
    schedule,
    dynamic,
    extent,
    activeBytes,
  );
  const combinedPeak = activeBytes.reduce((peak, value) => (value > peak ? value : peak), 0n);
  if (
    BigInt(positiveIdOrZero(schedule.estimatedPeakSurfaceBytes, "combined peak bytes")) !==
    combinedPeak
  ) {
    fail("bound_schedule", "combined plan/program peak byte estimate mismatch");
  }
  const planPasses = array(plan.passes, "plan.passes");
  const boundPasses = array(schedule.passes, "schedule.passes");
  if (planPasses.length !== boundPasses.length)
    fail("bound_schedule", "execution pass count mismatch");
  const passes = planPasses.map((raw, index): BoundExecutionPass => {
    const pass = record(raw, `plan.passes[${index}]`);
    const bound = record(boundPasses[index], `schedule.passes[${index}]`);
    const id = positiveId(pass.id, "execution pass id");
    const output = passOutput(record(pass.kind, "execution pass kind"));
    const outputRoi = required(resourceRois, output, "bound pass output ROI");
    const surfaceSlot = slotByResource.get(output) ?? null;
    const actualSlot =
      bound.surfaceSlot == null ? null : positiveId(bound.surfaceSlot, "bound pass surface slot");
    const actualRoi = deviceRoi(record(bound.deviceRoi, "bound pass device ROI"), extent);
    const expectedRetire = [...retireAfter[index]!].sort((a, b) => a - b);
    const uniqueRetire = expectedRetire.filter(
      (value, position) => position === 0 || value !== expectedRetire[position - 1],
    );
    const actualRetire = array(bound.retireAfter, "bound pass retireAfter").map((value) =>
      positiveId(value, "retired resource"),
    );
    if (
      id !== index + 1 ||
      positiveId(bound.pass, "bound pass id") !== id ||
      positiveId(bound.output, "bound pass output") !== output ||
      !sameRoi(actualRoi, outputRoi) ||
      actualSlot !== surfaceSlot ||
      !deepEqual(actualRetire, uniqueRetire)
    ) {
      fail("bound_schedule", `execution pass ${id} diverges from its plan and allocations`);
    }
    return { output, deviceRoi: outputRoi, surfaceSlot, retireAfter: actualRetire };
  });
  return { slotExtents, slotIntervals, outerActiveBytes, programs: boundPrograms, passes };
}

function admitBoundProgramSchedules(
  plan: Wire,
  framePrograms: Wire[],
  schedule: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
  activeBytes: bigint[],
): Map<number, BoundProgramRuntimeSchedule> {
  const result = new Map<number, BoundProgramRuntimeSchedule>();
  let frameOperations = 0n,
    frameSamples = 0n;
  const programLayouts = array(plan.programs, "plan.programs");
  const boundPrograms = array(schedule.programs, "schedule.programs");
  if (
    programLayouts.length !== framePrograms.length ||
    framePrograms.length !== boundPrograms.length
  ) {
    fail("program_schedule", "bound program count does not match the plan");
  }

  const executionByProgram = new Map<number, { pass: number; kind: Wire }>();
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const pass = record(rawPass, "plan pass");
    const kind = record(pass.kind, "plan pass kind");
    if (kind.kind !== "rasterProgram" && kind.kind !== "rasterCaption") continue;
    const program = positiveId(kind.program, "program execution id");
    if (executionByProgram.has(program))
      fail("program_schedule", `program ${program} has more than one execution pass`);
    executionByProgram.set(program, { pass: positiveId(pass.id, "program execution pass"), kind });
  }

  for (let index = 0; index < framePrograms.length; index += 1) {
    const layout = record(programLayouts[index], `plan.programs[${index}]`);
    const program = record(framePrograms[index], `bindings.programs[${index}]`);
    const bound = record(boundPrograms[index], `schedule.programs[${index}]`);
    const id = positiveId(program.id, `bindings.programs[${index}].id`);
    if (
      id !== index + 1 ||
      positiveId(layout.id, `plan.programs[${index}].id`) !== id ||
      positiveId(bound.program, "bound program id") !== id
    ) {
      fail("program_schedule", "program IDs are not canonical and aligned");
    }
    const execution = required(executionByProgram, id, "program execution pass");
    if (
      positiveId(bound.executionPass, "bound program execution pass") !== execution.pass ||
      String(bound.boundsReason) !== String(execution.kind.boundsReason)
    ) {
      fail("program_schedule", `program ${id} execution identity diverges from the plan`);
    }

    const localPlan = record(program.localPlan, `program ${id} localPlan`);
    const localSchedule = record(program.localSchedule, `program ${id} localSchedule`);
    const storageByResource = new Map<number, Wire>();
    for (const rawStorage of array(localSchedule.resources, "program schedule resources")) {
      const storage = record(rawStorage, "program resource storage");
      const resource = positiveId(storage.resource, "program storage resource");
      if (storageByResource.has(resource))
        fail("program_schedule", `program ${id} repeats resource ${resource}`);
      storageByResource.set(resource, record(storage.storage, "program storage kind"));
    }

    const destinationRois = new Map<number, DeviceRoi>();
    for (const rawPass of array(localPlan.passes, "program local passes")) {
      const localKind = record(
        record(rawPass, "program local pass").kind,
        "program local pass kind",
      );
      if (localKind.kind !== "readDestination" || localKind.external == null) continue;
      const destination = positiveId(localKind.external, "program destination id");
      const use = record(
        requiredIndex(
          array(program.destinationUses, "program destination uses"),
          destination - 1,
          "program destination use",
        ),
        "program destination use",
      );
      const roi = deviceRoi(dynamicBounds(dynamic, use.sampleBounds), extent);
      const output = positiveId(localKind.output, "program destination output");
      const prior = destinationRois.get(output);
      if (prior && !sameRoi(prior, roi))
        fail("program_schedule", `program ${id} destination ROI has conflicting derivations`);
      destinationRois.set(output, roi);
    }

    const device = dynamicTransform(dynamic, execution.kind.transform);
    const localResources = array(localPlan.resources, "program local resources");
    const unclipped = (resourceId: number) =>
      projectProgramResource(
        program,
        record(
          requiredIndex(localResources, resourceId - 1, "program local resource"),
          "program local resource",
        ),
        device,
        extent,
        true,
      );
    // Mirror Engine's bind_program: these kernels can sample predecessors outside the final
    // viewport, so their inputs keep their complete projection. Blurs, drop shadows,
    // displacements and velocity blurs read their input up to their footprint away from the
    // pixels they write, including beyond the root. A predecessor needs only what can reach those
    // pixels: a clip passes on its own region, composites pass on what their output needs, and
    // other samplers keep their whole input.
    const root = { x: 0, y: 0, ...extent };
    const beyondRoot = new Map<number, DeviceRoi>();
    const need = (resourceId: number, region: DeviceRoi) => {
      const prior = beyondRoot.get(resourceId);
      beyondRoot.set(resourceId, prior === undefined ? region : unionRoi(prior, region));
    };
    for (const raw of [...array(localPlan.passes, "program local passes")].reverse()) {
      const kind = record(record(raw, "program local pass").kind, "program local pass kind");
      const output = passOutput(kind),
        needed = beyondRoot.get(output);
      const filter =
        kind.kind === "applyFilter"
          ? String(record(kind.filter, "program filter").kind)
          : undefined;
      if (kind.kind === "applyShader") {
        for (const input of programPassReads(kind)) need(input, unclipped(input));
      } else if (
        ["lensDistortion", "glow", "bloom", "radialBlur", "chromaticAberration"].includes(
          filter ?? "",
        )
      ) {
        const input = positiveId(kind.input, "kernel input");
        need(input, unclipped(input));
      } else if (
        ["blur", "dropShadow", "noiseDisplacement", "velocityBlur"].includes(filter ?? "")
      ) {
        const region =
          needed ??
          (String(execution.kind.boundsReason) === "exact"
            ? intersectRoi(unclipped(output), root)
            : emptyRoi());
        if (region.width > 0 && region.height > 0) {
          const input = positiveId(kind.input, "sampler input");
          need(input, samplerFootprint(region, unclipped(input), unclipped(output)));
        }
      } else if (kind.kind === "applyTransition") {
        for (const input of programPassReads(kind)) need(input, unclipped(input));
      } else if (kind.kind === "applyClip") {
        if (needed)
          need(positiveId(kind.input, "clip input"), intersectRoi(needed, unclipped(output)));
      } else if (COMPOSITE_PASSES.has(String(kind.kind))) {
        if (needed) for (const input of programPassReads(kind)) need(input, needed);
      } else if (needed) {
        for (const input of programPassReads(kind)) need(input, unclipped(input));
      }
    }
    const resourceRois = new Map<number, DeviceRoi>();
    const boundResources = array(bound.resources, "bound program resources");
    if (
      localResources.length !== boundResources.length ||
      storageByResource.size !== localResources.length
    ) {
      fail("program_schedule", `program ${id} resource count mismatch`);
    }
    for (let resourceIndex = 0; resourceIndex < localResources.length; resourceIndex += 1) {
      const resource = record(
        localResources[resourceIndex],
        `program ${id} resource ${resourceIndex}`,
      );
      const resourceId = positiveId(resource.id, "program resource id");
      if (resourceId !== resourceIndex + 1)
        fail("program_schedule", `program ${id} resource IDs are not canonical`);
      const storage = required(storageByResource, resourceId, "program storage");
      const region = beyondRoot.get(resourceId);
      resourceRois.set(
        resourceId,
        storage.kind === "transparent"
          ? emptyRoi()
          : region !== undefined && !destinationRois.has(resourceId)
            ? intersectRoi(
                projectProgramResource(program, resource, device, extent, true),
                unionRoi(region, root),
              )
            : String(execution.kind.boundsReason) === "conservativeCameraTarget"
              ? { x: 0, y: 0, ...extent }
              : (destinationRois.get(resourceId) ??
                projectProgramResource(program, resource, device, extent)),
      );
    }
    restrictCompositesToDemand(
      array(localPlan.passes, "program local passes"),
      destinationRois,
      resourceRois,
    );
    for (let resourceIndex = 0; resourceIndex < localResources.length; resourceIndex += 1) {
      const resourceId = resourceIndex + 1;
      const actual = record(boundResources[resourceIndex], "bound program resource");
      if (
        positiveId(actual.resource, "bound program resource id") !== resourceId ||
        !sameRoi(
          deviceRoi(
            record(actual.deviceRoi, "bound program device ROI"),
            extent,
            beyondRoot.has(resourceId),
          ),
          required(resourceRois, resourceId, "program resource ROI"),
        )
      ) {
        fail(
          "program_schedule",
          `program ${id} resource ${resourceId} ROI is not the exact plan/binding projection`,
        );
      }
    }

    const slotExtents = new Map<number, { width: number; height: number } | null>();
    const slotByResource = new Map<number, number>();
    const localSlots = array(localSchedule.surfaceSlots, "program surface slots");
    const boundSlots = array(bound.surfaceSlots, "bound program surface slots");
    if (localSlots.length !== boundSlots.length)
      fail("program_schedule", `program ${id} surface slot count mismatch`);
    let estimatedSurfaceBytes = 0n;
    for (let slotIndex = 0; slotIndex < localSlots.length; slotIndex += 1) {
      const slot = record(localSlots[slotIndex], "program surface slot");
      const boundSlot = record(boundSlots[slotIndex], "bound program surface slot");
      const slotId = positiveId(slot.id, "program surface slot id");
      if (
        slotId !== slotIndex + 1 ||
        positiveId(boundSlot.id, "bound program slot id") !== slotId
      ) {
        fail("program_schedule", `program ${id} surface slot IDs diverge`);
      }
      const allocations = array(slot.allocations, "program slot allocations");
      const boundAllocations = array(boundSlot.allocations, "bound program slot allocations");
      if (allocations.length !== boundAllocations.length)
        fail("program_schedule", `program ${id} slot ${slotId} allocation count mismatch`);
      let width = 0;
      let height = 0;
      for (let allocationIndex = 0; allocationIndex < allocations.length; allocationIndex += 1) {
        const allocation = record(allocations[allocationIndex], "program surface allocation");
        const actual = record(
          boundAllocations[allocationIndex],
          "bound program surface allocation",
        );
        const resource = positiveId(allocation.resource, "program allocation resource");
        const roi = required(resourceRois, resource, "program allocation ROI");
        if (
          positiveId(actual.resource, "bound program allocation resource") !== resource ||
          String(actual.reason) !== String(allocation.reason) ||
          !deepEqual(actual.interval, allocation.interval) ||
          !sameRoi(
            deviceRoi(
              record(actual.deviceRoi, "bound program allocation ROI"),
              extent,
              beyondRoot.has(resource),
            ),
            roi,
          )
        ) {
          fail(
            "program_schedule",
            `program ${id} slot ${slotId} allocation ${allocationIndex} diverges`,
          );
        }
        if (slotByResource.has(resource)) {
          fail(
            "program_schedule",
            `program ${id} resource ${resource} has more than one physical slot`,
          );
        }
        slotByResource.set(resource, slotId);
        width = Math.max(width, roi.width);
        height = Math.max(height, roi.height);
      }
      const expectedExtent = width === 0 || height === 0 ? null : { width, height };
      const actualExtent =
        boundSlot.extent == null
          ? null
          : {
              width: positiveId(
                record(boundSlot.extent, "bound program extent").width,
                "bound program width",
              ),
              height: positiveId(
                record(boundSlot.extent, "bound program extent").height,
                "bound program height",
              ),
            };
      if (!deepEqual(actualExtent, expectedExtent))
        fail("program_schedule", `program ${id} slot ${slotId} extent mismatch`);
      const bytes = BigInt(width) * BigInt(height) * 8n;
      if (
        BigInt(positiveIdOrZero(boundSlot.estimatedBytes, "bound program slot bytes")) !== bytes
      ) {
        fail("program_schedule", `program ${id} slot ${slotId} byte estimate mismatch`);
      }
      estimatedSurfaceBytes += bytes;
      slotExtents.set(slotId, expectedExtent);
    }
    if (
      BigInt(positiveIdOrZero(bound.estimatedSurfaceBytes, "bound program bytes")) !==
      estimatedSurfaceBytes
    ) {
      fail("program_schedule", `program ${id} total surface byte estimate mismatch`);
    }
    const localPasses = array(localPlan.passes, "program local passes");
    const boundPasses = array(bound.passes, "bound program passes");
    if (localPasses.length !== boundPasses.length)
      fail("program_schedule", `program ${id} pass count mismatch`);
    const lastUses = new Map<number, number>();
    for (const [index, rawPass] of localPasses.entries()) {
      const pass = record(rawPass, "program local pass");
      const kind = record(pass.kind, "program local pass kind");
      const passId = positiveId(pass.id, "program local pass id");
      if (passId !== index + 1)
        fail("program_schedule", `program ${id} pass IDs are not canonical`);
      const output = passOutput(kind);
      lastUses.set(output, passId);
      for (const input of programPassReads(kind)) lastUses.set(input, passId);
      const storage = required(storageByResource, output, "program pass storage");
      if (storage.kind === "alias") {
        const source = positiveId(storage.source, "program alias source");
        lastUses.set(source, Math.max(lastUses.get(source) ?? 0, passId));
      }
      if (
        storage.kind === "surface" &&
        slotByResource.get(output) !== positiveId(storage.slot, "program surface slot")
      ) {
        fail(
          "program_schedule",
          `program ${id} pass ${passId} surface slot diverges from its allocation`,
        );
      }
    }
    const retireByPass = localPasses.map(() => [] as number[]);
    const programOutput = positiveId(localPlan.output, "program local output");
    for (const [resource, last] of lastUses) {
      if (resource !== programOutput) retireByPass[last - 1]!.push(resource);
    }
    for (const retired of retireByPass) retired.sort((left, right) => left - right);
    const passes = localPasses.map((rawPass, index): BoundProgramExecutionPass => {
      const pass = record(rawPass, "program local pass");
      const kind = record(pass.kind, "program local pass kind");
      const actual = record(boundPasses[index], "bound program pass");
      const output = passOutput(kind);
      const roi = required(resourceRois, output, "bound program pass ROI");
      const storage = required(storageByResource, output, "bound program pass storage");
      const actualRoi = deviceRoi(
        record(actual.deviceRoi, "bound program pass device ROI"),
        extent,
        beyondRoot.has(output),
      );
      const actualRetire = array(actual.retireAfter, "bound program pass retireAfter").map(
        (resource) => positiveId(resource, "retired program resource"),
      );
      if (
        positiveId(actual.pass, "bound program pass id") !== index + 1 ||
        positiveId(actual.output, "bound program pass output") !== output ||
        !sameRoi(actualRoi, roi) ||
        !deepEqual(actual.storage, storage) ||
        !deepEqual(actualRetire, retireByPass[index])
      ) {
        fail(
          "program_schedule",
          `program ${id} pass ${index + 1} diverges from its plan and bindings`,
        );
      }
      return { output, deviceRoi: actualRoi, storage, retireAfter: actualRetire };
    });
    let operations = 0n,
      samples = 0n;
    for (const rawPass of localPasses) {
      const pass = record(rawPass, "program pass");
      const kind = record(pass.kind, "program pass kind");
      const cost = record(pass.shaderWorkPerPixel, "shader work per pixel");
      const roi = required(
        resourceRois,
        positiveId(kind.output, "program output"),
        "program output ROI",
      );
      const pixels = BigInt(roi.width) * BigInt(roi.height);
      operations += BigInt(positiveIdOrZero(cost.operations, "shader operations")) * pixels;
      samples += BigInt(positiveIdOrZero(cost.samples, "shader samples")) * pixels;
    }
    const actualWork = record(bound.shaderWork, "bound program shader work");
    if (
      BigInt(positiveIdOrZero(actualWork.operations, "bound shader operations")) !== operations ||
      BigInt(positiveIdOrZero(actualWork.samples, "bound shader samples")) !== samples
    ) {
      fail("shader_budget", "program Shader work differs from its exact pass/ROI projection");
    }
    frameOperations += operations;
    frameSamples += samples;
    const active = activeBytes[execution.pass - 1];
    if (active === undefined)
      fail("program_schedule", `program ${id} execution pass is out of range`);
    activeBytes[execution.pass - 1] = active + estimatedSurfaceBytes;
    result.set(id, { resourceRois, slotExtents, passes, estimatedSurfaceBytes });
  }

  if (executionByProgram.size !== result.size) {
    fail("program_schedule", "every program must have exactly one bound execution schedule");
  }
  const actualWork = record(schedule.shaderWork, "bound frame shader work");
  if (
    BigInt(positiveIdOrZero(actualWork.operations, "frame shader operations")) !==
      frameOperations ||
    BigInt(positiveIdOrZero(actualWork.samples, "frame shader samples")) !== frameSamples
  ) {
    fail("shader_budget", "frame Shader work is not the sum of its programs");
  }
  return result;
}

/** When each pooled surface is next needed in the frame being run. */
interface SurfaceLiveness {
  /** Outer slot allocation intervals, as inclusive 1-based outer pass indices. */
  readonly slotIntervals: ReadonlyMap<number, readonly (readonly [number, number])[]>;
  /** The extent of each slot the frame allocates, by program, with 0 for the outer slots. */
  readonly slotExtents: ReadonlyMap<
    number,
    ReadonlyMap<number, { width: number; height: number } | null>
  >;
  /** The extents of the transients the frame checks out, as `${width}x${height}`. */
  readonly transientExtents: ReadonlySet<string>;
  /** The outer pass that runs each program. */
  readonly programPasses: ReadonlyMap<number, number>;
  /** The outer pass that writes the output surface. */
  readonly outputPass: number;
}

/** A value's device ROI and the extent of the image that holds it. */
interface HeldValue {
  readonly roi: DeviceRoi;
  readonly width: number;
  readonly height: number;
}

/** Crop each program's inputs and choose its direct or scheduled execution for this frame. */
function planProgramRuns(
  executor: CanvasKitExecutor,
  plan: Wire,
  structures: ReadonlyMap<number, ProgramStructure>,
  schedule: BoundOuterSchedule,
  dynamic: Map<number, Wire>,
  geometryRenderer: GpuInstances | null,
): Map<number, ProgramRun> {
  const runs = new Map<number, ProgramRun>();
  const planner = executor.engineKernels?.plan_pyramid_filter?.bind(executor.engineKernels);
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const kind = record(record(rawPass, "plan pass").kind, "plan pass kind");
    if (kind.kind !== "rasterProgram" && kind.kind !== "rasterCaption") continue;
    const id = positiveId(kind.program, "program execution id");
    const program = required(structures, id, "program");
    const localPlan = record(program.plan.localPlan, "program.localPlan");
    const device = dynamicTransform(dynamic, kind.transform);
    const bound = cropProgramInputs(
      localPlan,
      required(schedule.programs, id, "bound program schedule"),
      planner,
      (local) =>
        programFilterInDeviceSpace(
          record(local.filter, "program filter"),
          programFilterMatrix(local, programMatrix(program, localPlan, local.output, device)),
        ),
      (local) => programPassReads(local),
      (local) => {
        const exact = programMatrix(program, localPlan, local.output, device);
        return programLensFrame(localPlan, local, programFilterMatrix(local, exact), exact);
      },
      (local) =>
        geometryRenderer?.canCrop(
          program.draw,
          local.roots as number[],
          programMatrix(program, localPlan, local.output, device),
        ) ?? false,
    );
    const direct =
      program.directRaster !== null &&
      !(
        program.directRaster.spatialFilters &&
        !isScaleTranslateMatrix(programMatrix(program, localPlan, localPlan.output, device))
      );
    runs.set(id, { bound, direct, device });
  }
  return runs;
}

/**
 * Peak bytes of the working surfaces a frame holds at once, and of the images it holds beside
 * them. Surfaces count each outer slot inside its allocation intervals, the output once written,
 * a scheduled program's slots while it runs, and each transient a pass checks out at the size the
 * arena allocates. The arena keeps the live surfaces inside this envelope.
 */
function frameWorkingSet(
  executor: CanvasKitExecutor,
  plan: Wire,
  structures: ReadonlyMap<number, ProgramStructure>,
  runs: ReadonlyMap<number, ProgramRun>,
  schedule: BoundOuterSchedule,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
  gpu: boolean,
): { surfaceBytes: bigint; imageBytes: bigint; liveness: SurfaceLiveness } {
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  const rootBytes = roiBytes(root);
  const externalSlots = array(
    record(plan.bindingLayout, "plan.bindingLayout").externalSlots,
    "externalSlots",
  );
  const live = new Map<number, HeldValue>();
  const programPasses = new Map<number, number>();
  const transients = new Set<string>();
  let outputPass = Infinity,
    surfaceBytes = 0n,
    imageBytes = 0n,
    layerBytes = 0n;
  // Kernels and creative blends read values at the root extent; other ROIs are copied there.
  const copies = (ids: readonly number[]) =>
    BigInt(
      ids.filter((id) => {
        const value = live.get(id);
        return value !== undefined && !sameRoi(value.roi, root);
      }).length,
    ) * rootBytes;
  for (const [index, raw] of array(plan.passes, "plan.passes").entries()) {
    const kind = record(record(raw, "plan pass").kind, "plan pass kind");
    const step = schedule.passes[index]!;
    let surfaces = 0n,
      images = 0n;
    if (kind.kind === "bindBackdropView" || kind.kind === "aliasResource") {
      const source = live.get(positiveId(kind.input, "plan resource id"));
      if (source) live.set(step.output, source);
    } else if (step.deviceRoi.width !== 0 && step.deviceRoi.height !== 0) {
      if (step.surfaceSlot === null) outputPass = Math.min(outputPass, index + 1);
      if (kind.kind === "rasterProgram" || kind.kind === "rasterCaption") {
        const id = positiveId(kind.program, "program execution id");
        programPasses.set(id, index + 1);
        const program = required(structures, id, "program"),
          run = required(runs, id, "program run");
        const destinations =
          kind.kind === "rasterProgram"
            ? array(kind.destinationInputs, "destinationInputs").map(
                (input) => live.get(positiveId(input, "plan resource id")) ?? null,
              )
            : [];
        const need = programWorkingSet(
          executor,
          program,
          run,
          destinations,
          externalSlots,
          gpu,
          transients,
        );
        surfaces = need.surfaces;
        images = need.images;
        // A host frame under a viewport short of the frame is cut into a transient of its own.
        const frames = kind.kind === "rasterProgram" ? frameInputs(kind).length : 0;
        const region = frames === 0 ? root : frameRegion(run.device, extent);
        if (!sameRoi(region, root)) {
          transients.add(`${region.width}x${region.height}`);
          surfaces += BigInt(frames) * roiBytes(region);
        }
        if (
          kind.kind === "rasterProgram" &&
          rasterLayerKey(program, kind, run.device, step.deviceRoi, plan) !== null
        ) {
          const output = required(
            run.bound.resourceRois,
            positiveId(
              record(program.plan.localPlan, "program.localPlan").output,
              "program output",
            ),
            "program output ROI",
          );
          const bytes = roiBytes(output);
          if (bytes <= BigInt(RASTER_LAYER_CACHE_BYTES)) layerBytes += bytes;
        }
      } else if (kind.kind === "dispatchKernel") {
        const invocation = record(kind.invocation, "kernel invocation");
        surfaces =
          copies([...new Set(kernelInputs(invocation))]) +
          (invocation.kind === "mask" ? rootBytes : 0n);
      } else if (kind.kind === "compositeRegion") {
        const mode = record(kind.mode, "composite mode");
        if (mode.kind === "blend" && String(mode.mode) !== "normal") {
          surfaces = copies([
            positiveId(kind.layer, "plan resource id"),
            positiveId(kind.backdrop, "plan resource id"),
          ]);
        }
      } else if (kind.kind === "copyConvert") {
        surfaces = copies([positiveId(kind.input, "plan resource id")]);
        // The CPU sRGB transform holds CanvasKit storage and the Engine's WASM copy of the input.
        if (!gpu && sameRoi(step.deviceRoi, root)) {
          images =
            srgbPreviewScratchBytes(
              extent.width,
              extent.height,
              typeof executor.engineKernels?.transform_srgb_preview_f16 === "function",
            ) * 2n;
        }
      }
      // Root copies are the only transients outside programs.
      if (surfaces !== 0n && kind.kind !== "rasterProgram" && kind.kind !== "rasterCaption") {
        transients.add(`${extent.width}x${extent.height}`);
      }
      const slot =
        step.surfaceSlot === null ? extent : (schedule.slotExtents.get(step.surfaceSlot) ?? extent);
      live.set(step.output, { roi: step.deviceRoi, width: slot.width, height: slot.height });
    }
    const held = schedule.outerActiveBytes[index]! + (outputPass <= index + 1 ? rootBytes : 0n);
    surfaceBytes = maxBigInt(surfaceBytes, held + surfaces);
    imageBytes = maxBigInt(imageBytes, images);
    for (const id of step.retireAfter) live.delete(id);
  }
  // Raster layers this frame retains keep their snapshots until the cache evicts them.
  const layerRoom = BigInt(Math.max(0, RASTER_LAYER_CACHE_BYTES - executor.rasterLayerBytes));
  imageBytes += layerBytes < layerRoom ? layerBytes : layerRoom;
  // A CPU surface's snapshot copies its pixels, so each live surface may be held twice.
  if (!gpu) imageBytes += surfaceBytes;
  const slotExtents = new Map([[0, schedule.slotExtents]]);
  for (const [id, run] of runs) slotExtents.set(id, run.direct ? new Map() : run.bound.slotExtents);
  return {
    surfaceBytes,
    imageBytes,
    liveness: {
      slotIntervals: schedule.slotIntervals,
      slotExtents,
      transientExtents: transients,
      programPasses,
      outputPass,
    },
  };
}

/**
 * Transient and slot bytes one program holds at the peak of its run, and the image bytes it holds
 * beside them: a direct raster program's backdrop copy of its surface, CPU kernel buffers and noise.
 * `transients` collects the extents of the transients it checks out.
 */
function programWorkingSet(
  executor: CanvasKitExecutor,
  program: ProgramStructure,
  run: ProgramRun,
  destinations: readonly (HeldValue | null)[],
  externalSlots: unknown[],
  gpu: boolean,
  transients: Set<string>,
): { surfaces: bigint; images: bigint } {
  const localPlan = record(program.plan.localPlan, "program.localPlan");
  const { bound } = run;
  const atlas = atlasWorkingBytes(program, externalSlots, transients);
  if (run.direct) {
    const output = required(
      bound.resourceRois,
      positiveId(localPlan.output, "direct raster output"),
      "direct raster output ROI",
    );
    if (output.width === 0 || output.height === 0) return { surfaces: 0n, images: 0n };
    transients.add(`${output.width}x${output.height}`);
    const bytes = roiBytes(output);
    const backdrops = [...program.directRaster!.backdrops.values()].filter(
      (backdrop) => backdrop.hasLocalInputs,
    );
    const combined = backdrops.some((backdrop) => backdrop.destination !== null) ? bytes : 0n;
    // Drawing the backdrop over the local snapshot copies the GPU surface on write.
    return {
      surfaces: bytes + maxBigInt(combined, atlas),
      images: gpu && backdrops.length !== 0 ? bytes : 0n,
    };
  }
  let slots = 0n;
  for (const slot of bound.slotExtents.values())
    if (slot) slots += BigInt(slot.width) * BigInt(slot.height) * 8n;
  const kernels = executor.engineKernels;
  const values = new Map<number, HeldValue | null>();
  let peak = 0n,
    images = 0n;
  for (const [index, rawPass] of array(localPlan.passes, "localPlan.passes").entries()) {
    const kind = record(record(rawPass, "program pass").kind, "program pass kind");
    const step = bound.passes[index];
    if (!step) fail("program_schedule", `bound program pass ${index + 1} is absent`);
    const { output, storage, deviceRoi: roi } = step;
    if (storage.kind === "destination") {
      values.set(output, destinations[positiveId(storage.destination, "destination") - 1] ?? null);
      continue;
    }
    if (storage.kind === "alias") {
      values.set(output, values.get(positiveId(storage.source, "program alias source")) ?? null);
      continue;
    }
    if (storage.kind === "transparent" || roi.width === 0 || roi.height === 0) {
      values.set(output, null);
      continue;
    }
    let need = 0n;
    if (storage.kind === "output") {
      need = roiBytes(roi);
      transients.add(`${roi.width}x${roi.height}`);
    }
    if (kind.kind === "rasterNode" || kind.kind === "rasterTree") need += atlas;
    if (kind.kind === "applyFilter") {
      const filter = programFilterInDeviceSpace(
        record(kind.filter, "program filter"),
        programFilterMatrix(kind, programMatrix(program, localPlan, kind.output, run.device)),
      );
      const input = values.get(positiveId(kind.input, "program input")) ?? null;
      const pyramid = filter.kind === "glow" || filter.kind === "bloom";
      if (filter.kind === "radialBlur" && input) {
        const planner = kernels?.plan_radial_blur;
        if (!planner) fail("radial_blur_plan", "radial blur pass planner is unavailable");
        need += radialBlurWorkingBytes(
          planner.call(
            kernels,
            finiteNumber(filter.amount, "radial blur amount"),
            input.roi.x,
            input.roi.y,
            input.roi.width,
            input.roi.height,
            roi.x,
            roi.y,
            roi.width,
            roi.height,
          ),
          transients,
        );
      } else if (
        (pyramid || filter.kind === "lensDistortion") &&
        input &&
        !(gpu && filter.kind === "lensDistortion")
      ) {
        const planner = kernels?.plan_pyramid_filter;
        const recipe =
          pyramid && planner
            ? planner.call(
                kernels,
                input.roi.width,
                input.roi.height,
                roi.width,
                roi.height,
                input.roi.x - roi.x,
                input.roi.y - roi.y,
                JSON.stringify(filter),
              )
            : null;
        if (gpu && recipe) {
          need += BigInt(
            pyramidWorkingBytes(
              recipe,
              roi,
              filter.kind as "glow" | "bloom",
              input.width !== input.roi.width || input.height !== input.roi.height,
              transients,
            ),
          );
        } else {
          // The read and the result each exist in JavaScript and in WASM memory.
          const read = !recipe
            ? input.roi
            : intersectRoi(
                input.roi,
                recipe.length >= 15
                  ? {
                      x: roi.x + recipe[0]!,
                      y: roi.y + recipe[1]!,
                      width: recipe[2]!,
                      height: recipe[3]!,
                    }
                  : roi,
              );
          images = maxBigInt(images, (roiBytes(read) + roiBytes(roi)) * 2n);
        }
      } else if (filter.kind === "filmGrain") {
        // The noise lattice exists in JavaScript and as a CanvasKit image.
        const size = finiteNumber(filter.size, "film grain size");
        const width = Math.floor((roi.x + roi.width - 1) / size) + 2 - Math.floor(roi.x / size);
        const height = Math.floor((roi.y + roi.height - 1) / size) + 2 - Math.floor(roi.y / size);
        images = maxBigInt(images, BigInt(width) * BigInt(height) * 8n);
      }
    }
    const slot =
      storage.kind === "surface"
        ? bound.slotExtents.get(positiveId(storage.slot, "program surface slot"))
        : null;
    values.set(output, {
      roi,
      width: slot?.width ?? roi.width,
      height: slot?.height ?? roi.height,
    });
    peak = maxBigInt(peak, need);
  }
  return { surfaces: slots + peak, images };
}

/** Peak bytes of the ping-pong transients `drawRadialBlur` holds for a pass plan, and their extents. */
function radialBlurWorkingBytes(plan: Float64Array, transients: Set<string>): bigint {
  let peak = 0n,
    prior = 0n;
  // Every pass but the last writes a transient while the previous one is still read.
  for (let at = 0; at + 5 < plan.length; at += 5) {
    transients.add(`${plan[at + 2]}x${plan[at + 3]}`);
    const bytes = BigInt(Math.ceil(plan[at + 2]!)) * BigInt(Math.ceil(plan[at + 3]!)) * 8n;
    peak = maxBigInt(peak, prior + bytes);
    prior = bytes;
  }
  return peak;
}

const atlasRegions = new WeakMap<DrawProgramWire, readonly { texture: Wire; src: RectWire }[]>();

/** The largest crop an image batch of the program checks out, from its textures' admitted extents. */
function atlasWorkingBytes(
  program: ProgramStructure,
  externalSlots: unknown[],
  transients: Set<string>,
): bigint {
  let regions = atlasRegions.get(program.draw);
  if (!regions) {
    const found: { texture: Wire; src: RectWire }[] = [];
    for (const raw of program.draw.nodes) {
      const node = record(raw, "DrawProgram node");
      if (node.kind !== "instanceBatch") continue;
      const shape = record(record(node.value, "batch node").shape, "instance shape");
      if (shape.kind !== "image") continue;
      const region = record(shape.value, "instance image region");
      found.push({
        texture: record(region.texture, "atlas texture"),
        src: rect(region.src, "atlas source"),
      });
    }
    atlasRegions.set(program.draw, (regions = found));
  }
  if (regions.length === 0) return 0n;
  const resources = record(program.plan.resources, "program.resources");
  const requirements = array(
    record(program.plan.requirements, "program.requirements").externalTextures,
    "program texture requirements",
  );
  const bindings = array(resources.textures, "program textures");
  const extents = new Map<string, { width: number; height: number }>();
  for (const [index, raw] of bindings.entries()) {
    const slot = positiveId(record(raw, "texture binding").slot, "texture slot");
    const expected = record(
      record(requiredIndex(externalSlots, slot - 1, "texture slot"), "external slot").expected,
      "external descriptor",
    );
    if (expected.extent == null) continue;
    const size = record(expected.extent, "texture extent");
    const width = positiveId(size.width, "texture width"),
      height = positiveId(size.height, "texture height");
    // Data textures are stored as an Alpha_8 image of 2 x 4 bytes per texel.
    extents.set(
      textureIdentity(record(requirements[index], "texture requirement")),
      expected.kind === "dataTexture"
        ? { width: width * 2, height: height * 4 }
        : { width, height },
    );
  }
  let peak = 0n;
  for (const region of regions) {
    const extent = extents.get(textureIdentity(region.texture));
    if (!extent) continue;
    const crop = atlasCrop(region.src, extent.width, extent.height);
    if (crop.width <= 0 || crop.height <= 0) continue;
    peak = maxBigInt(peak, BigInt(crop.width) * BigInt(crop.height) * 8n);
    transients.add(`${crop.width}x${crop.height}`);
  }
  return peak;
}

/** The texels an atlas region crops, and the region's normalized rectangle inside the crop. */
function atlasCrop(
  region: RectWire,
  width: number,
  height: number,
): {
  left: number;
  top: number;
  width: number;
  height: number;
  source: RectWire;
} {
  const edges = [
    region.x * width,
    region.y * height,
    (region.x + region.width) * width,
    (region.y + region.height) * height,
  ];
  const left = Math.max(0, Math.floor(edges[0]!)),
    top = Math.max(0, Math.floor(edges[1]!));
  const right = Math.min(width, Math.ceil(edges[2]!)),
    bottom = Math.min(height, Math.ceil(edges[3]!));
  const cropWidth = right - left,
    cropHeight = bottom - top;
  return {
    left,
    top,
    width: cropWidth,
    height: cropHeight,
    source: {
      x: (edges[0]! - left) / cropWidth,
      y: (edges[1]! - top) / cropHeight,
      width: (edges[2]! - edges[0]!) / cropWidth,
      height: (edges[3]! - edges[1]!) / cropHeight,
    },
  };
}

/** The outer values a kernel invocation reads. */
function kernelInputs(invocation: Wire): number[] {
  switch (invocation.kind) {
    case "group":
    case "mask":
    case "filter":
    case "adjustmentEffect":
      return [positiveId(invocation.input, "kernel input")];
    case "transition":
      return [invocation.backdrop, invocation.from, invocation.to].map((id) =>
        positiveId(id, "transition input"),
      );
    default:
      return fail("unsupported_kernel", `kernel '${String(invocation.kind)}' is not closed`);
  }
}

function roiBytes(roi: { width: number; height: number }): bigint {
  return BigInt(roi.width) * BigInt(roi.height) * 8n;
}

function projectProgramResource(
  program: Wire,
  resource: Wire,
  device: number[],
  extent: { width: number; height: number },
  unclipped = false,
): DeviceRoi {
  const bounds = record(resource.bounds, "program resource bounds");
  if (bounds.kind === "empty") return emptyRoi();
  if (bounds.kind !== "finite")
    fail("program_schedule", `unknown local bounds kind '${String(bounds.kind)}'`);
  return projectProgramResourceRoi(
    rect(program.viewport, "program viewport"),
    rect(bounds.rect, "program finite bounds"),
    numberArray(resource.localToProgram, 9, "program localToProgram"),
    device,
    extent,
    unclipped,
  );
}

/**
 * Mirror Rust's conservative two-stage program projection exactly.
 *
 * A resource is first mapped through `localToProgram` and collapsed to a program-space AABB;
 * only that AABB is normalized through the program viewport and projected into device space.
 * Composing both matrices and projecting the original four corners is geometrically tighter but
 * is not the same storage proof when the two transforms contain opposing shear/rotation.
 */
export function projectProgramResourceRoi(
  viewport: RectWire,
  value: RectWire,
  localToProgram: number[],
  device: number[],
  extent: { width: number; height: number },
  unclipped = false,
): DeviceRoi {
  if (viewport.width <= 0 || viewport.height <= 0)
    fail("program_schedule", "program viewport is empty");
  if (value.width <= 0 || value.height <= 0) return emptyRoi();
  const localCorners: Array<[number, number]> = [
    [value.x, value.y],
    [value.x + value.width, value.y],
    [value.x + value.width, value.y + value.height],
    [value.x, value.y + value.height],
  ];
  validateProjectionDomain(localToProgram, localCorners, Number.EPSILON, "program local resource");
  const localProjected = localCorners.map((corner) => projectValidated(localToProgram, corner));
  const mapped = {
    left: Math.min(...localProjected.map((point) => point[0])),
    top: Math.min(...localProjected.map((point) => point[1])),
    right: Math.max(...localProjected.map((point) => point[0])),
    bottom: Math.max(...localProjected.map((point) => point[1])),
  };
  const corners: Array<[number, number]> = [
    [(mapped.left - viewport.x) / viewport.width, (mapped.top - viewport.y) / viewport.height],
    [(mapped.right - viewport.x) / viewport.width, (mapped.top - viewport.y) / viewport.height],
    [(mapped.left - viewport.x) / viewport.width, (mapped.bottom - viewport.y) / viewport.height],
    [(mapped.right - viewport.x) / viewport.width, (mapped.bottom - viewport.y) / viewport.height],
  ];
  validateProjectionDomain(device, corners, 1e-12, "program device resource");
  const points = corners.map((corner) => projectValidated(device, corner));
  if (points.some((point) => point.some((v) => Math.abs(v) > 1_073_741_824))) {
    fail("program_schedule", "program projection exceeds the device coordinate budget");
  }
  const clip = (v: number, limit: number) => (unclipped ? v : Math.max(0, Math.min(limit, v)));
  const left = clip(Math.floor(Math.min(...points.map((point) => point[0]))), extent.width);
  const top = clip(Math.floor(Math.min(...points.map((point) => point[1]))), extent.height);
  const right = clip(Math.ceil(Math.max(...points.map((point) => point[0]))), extent.width);
  const bottom = clip(Math.ceil(Math.max(...points.map((point) => point[1]))), extent.height);
  return right <= left || bottom <= top
    ? emptyRoi()
    : { x: left, y: top, width: right - left, height: bottom - top };
}

function validateProjectionDomain(
  matrix: number[],
  corners: Array<[number, number]>,
  relativeTolerance: number,
  label: string,
): void {
  const denominators = corners.map(([x, y]) => matrix[6]! * x + matrix[7]! * y + matrix[8]!);
  const maximum = denominators.reduce((current, value) => Math.max(current, Math.abs(value)), 0);
  // Transform2d::map_bounds uses an absolute f64::EPSILON threshold. DeviceTransform uses a
  // scale-relative 1e-12 horizon threshold. The caller selects the matching policy.
  const tolerance =
    relativeTolerance === Number.EPSILON ? Number.EPSILON : maximum * relativeTolerance;
  if (
    !Number.isFinite(maximum) ||
    maximum === 0 ||
    denominators.some((value) => !Number.isFinite(value) || Math.abs(value) <= tolerance) ||
    denominators.some((value) => Math.sign(value) !== Math.sign(denominators[0]!))
  ) {
    fail("program_schedule", `${label} crosses the perspective horizon`);
  }
}

function projectValidated(matrix: number[], point: [number, number]): [number, number] {
  const w = matrix[6]! * point[0] + matrix[7]! * point[1] + matrix[8]!;
  const projected: [number, number] = [
    (matrix[0]! * point[0] + matrix[1]! * point[1] + matrix[2]!) / w,
    (matrix[3]! * point[0] + matrix[4]! * point[1] + matrix[5]!) / w,
  ];
  if (!projected.every(Number.isFinite))
    fail("program_schedule", "program resource projection is invalid");
  return projected;
}

function emptyRoi(): DeviceRoi {
  return { x: 0, y: 0, width: 0, height: 0 };
}

function deviceRoi(
  value: Wire,
  extent: { width: number; height: number },
  unclipped = false,
): DeviceRoi {
  const roi = {
    x: finiteNumber(value.x, "ROI x"),
    y: finiteNumber(value.y, "ROI y"),
    width: positiveIdOrZero(value.width, "ROI width"),
    height: positiveIdOrZero(value.height, "ROI height"),
  };
  if (
    ![roi.x, roi.y, roi.x + roi.width, roi.y + roi.height].every(
      (v) => Number.isSafeInteger(v) && Math.abs(v) <= 1_073_741_824,
    )
  ) {
    fail("bound_schedule", "ROI exceeds the device coordinate budget");
  }
  if (
    !unclipped &&
    (roi.x < 0 ||
      roi.y < 0 ||
      roi.x + roi.width > extent.width ||
      roi.y + roi.height > extent.height)
  ) {
    fail("bound_schedule", "ROI escapes the render root");
  }
  if ((roi.width === 0 || roi.height === 0) && !sameRoi(roi, { x: 0, y: 0, width: 0, height: 0 })) {
    fail("bound_schedule", "empty ROI is not canonical");
  }
  return roi;
}

const COMPOSITE_PASSES = new Set([
  "readDestination",
  "sourceOver",
  "applyMask",
  "applyOpacity",
  "applyTransform",
  "blend",
]);

/**
 * Mirror Engine's restrict_composites_to_demand. Composites write each output pixel from their
 * inputs' pixels at the same device position, so a composite output keeps only the union of what
 * its readers need; a clip reads its input only inside the clip output. Raster, clip and kernel
 * outputs keep their ROI.
 */
function restrictCompositesToDemand(
  passes: unknown[],
  destinationRois: ReadonlyMap<number, DeviceRoi>,
  rois: Map<number, DeviceRoi>,
): void {
  // undefined: no reader yet. null: a reader may sample anywhere in the resource.
  const demand = new Map<number, DeviceRoi | null>();
  for (const raw of [...passes].reverse()) {
    const kind = record(record(raw, "program local pass").kind, "program local pass kind");
    const output = passOutput(kind),
      composite = COMPOSITE_PASSES.has(String(kind.kind));
    const needed = demand.get(output);
    if (composite && !destinationRois.has(output) && needed) {
      const restricted = intersectRoi(required(rois, output, "program composite ROI"), needed);
      rois.set(output, restricted.width === 0 || restricted.height === 0 ? emptyRoi() : restricted);
    }
    const need =
      composite || kind.kind === "applyClip" ? required(rois, output, "program reader ROI") : null;
    for (const input of programPassReads(kind)) {
      const prior = demand.get(input);
      demand.set(
        input,
        prior === undefined ? need : prior === null || need === null ? null : unionRoi(prior, need),
      );
    }
  }
}

/** Mirror Engine's sampler_footprint: the input region a sampling filter reads to write `region`. */
function samplerFootprint(region: DeviceRoi, input: DeviceRoi, output: DeviceRoi): DeviceRoi {
  const left = region.x - Math.max(0, output.x + output.width - (input.x + input.width)) - 1;
  const top = region.y - Math.max(0, output.y + output.height - (input.y + input.height)) - 1;
  const right = region.x + region.width + Math.max(0, input.x - output.x) + 1;
  const bottom = region.y + region.height + Math.max(0, input.y - output.y) + 1;
  return { x: left, y: top, width: right - left, height: bottom - top };
}

function intersectRoi(left: DeviceRoi, right: DeviceRoi): DeviceRoi {
  const x = Math.max(left.x, right.x),
    y = Math.max(left.y, right.y);
  const width = Math.min(left.x + left.width, right.x + right.width) - x;
  const height = Math.min(left.y + left.height, right.y + right.height) - y;
  return width <= 0 || height <= 0 ? emptyRoi() : { x, y, width, height };
}

function unionRoi(left: DeviceRoi, right: DeviceRoi): DeviceRoi {
  if (left.width === 0 || left.height === 0) return right;
  if (right.width === 0 || right.height === 0) return left;
  const x = Math.min(left.x, right.x),
    y = Math.min(left.y, right.y);
  return {
    x,
    y,
    width: Math.max(left.x + left.width, right.x + right.width) - x,
    height: Math.max(left.y + left.height, right.y + right.height) - y,
  };
}

function sameRoi(left: DeviceRoi, right: DeviceRoi): boolean {
  return (
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height
  );
}

function retireOuterResources(retired: readonly number[], values: Map<number, ImageValue>): void {
  for (const id of retired) {
    const value = values.get(id);
    if (value?.owned) value.image?.delete();
    values.delete(id);
  }
}

async function drawRadialBlur(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  arena: SurfaceArena,
  canvas: Canvas,
  input: ImageValue,
  output: DeviceRoi,
  center: number[],
  amount: number,
  plan: Float64Array,
): Promise<void> {
  if (!input.image) return;
  const makeSource = (image: Image): Shader => {
    const shader = image.makeShaderOptions(
      CanvasKit.TileMode.Clamp,
      CanvasKit.TileMode.Clamp,
      CanvasKit.FilterMode.Nearest,
      CanvasKit.MipmapMode.None,
    );
    if (!shader) fail("radial_blur_shader", "radial blur input shader allocation failed");
    return shader;
  };
  const original = makeSource(input.image);
  let yieldAt = performance.now();
  let current = input;
  let currentSurface: Surface | null = null;
  let nextSurface: Surface | null = null;
  try {
    for (let at = 0; at < plan.length; at += 5) {
      if (arena.yieldToPresentation && performance.now() - yieldAt >= 6) {
        await arena.yieldToPresentation();
        yieldAt = performance.now();
      }
      const roi: DeviceRoi = {
        x: plan[at]!,
        y: plan[at + 1]!,
        width: plan[at + 2]!,
        height: plan[at + 3]!,
      };
      const finalPass = at + 5 === plan.length;
      const source = makeSource(current.image!);
      let shader: Shader | null = null;
      try {
        shader = builtins.shader(
          "radialBlur",
          [
            input.roi.width,
            input.roi.height,
            input.roi.x - roi.x,
            input.roi.y - roi.y,
            finalPass ? 1 : 0,
            current.roi.width,
            current.roi.height,
            current.roi.x - roi.x,
            current.roi.y - roi.y,
            roi.x,
            roi.y,
            center[0]!,
            center[1]!,
            amount,
            plan[at + 4]!,
          ],
          [source, original],
        );
        if (finalPass) {
          canvas.save();
          try {
            canvas.translate(output.x, output.y);
            drawShader(CanvasKit, canvas, shader, "src");
          } finally {
            canvas.restore();
          }
        } else {
          nextSurface = arena.transient(roi.width, roi.height);
          drawShader(CanvasKit, nextSurface.getCanvas(), shader, "src");
          if (!arena.gpu || !arena.deferGpuFlush) nextSurface.flush();
        }
      } finally {
        shader?.delete();
        source.delete();
      }
      if (currentSurface) {
        current.image?.delete();
        arena.releaseTransient(currentSurface);
        currentSurface = null;
        current = input;
      }
      if (nextSurface) {
        current = {
          image: nextSurface.makeImageSnapshot([0, 0, roi.width, roi.height]),
          roi,
          owned: true,
        };
        currentSurface = nextSurface;
        nextSurface = null;
      }
    }
  } finally {
    original.delete();
    if (currentSurface) {
      current.image?.delete();
      arena.releaseTransient(currentSurface);
    }
    if (nextSurface) arena.releaseTransient(nextSurface);
  }
}

/** Upload only deterministic noise, never read the rendered source back to the CPU. */
function drawFilmGrain(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: ImageValue,
  output: RectWire,
  seed: number,
  amount: number,
  size: number,
): void {
  if (!input.image) return;
  const originX = Math.floor(output.x / size),
    originY = Math.floor(output.y / size);
  const width = Math.floor((output.x + output.width - 1) / size) + 2 - originX;
  const height = Math.floor((output.y + output.height - 1) / size) + 2 - originY;
  if (width * height > 33_600_000)
    fail("film_grain_lattice", "film grain noise lattice is too large");
  const pixels = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) {
      let hash = seed ^ Math.imul(originX + x, 0x9e3779b9) ^ Math.imul(originY + y, 0x85ebca6b);
      hash ^= hash >>> 16;
      hash = Math.imul(hash, 0x7feb352d);
      hash ^= hash >>> 15;
      hash = Math.imul(hash, 0x846ca68b);
      hash ^= hash >>> 16;
      const at = (y * width + x) * 4;
      pixels[at] = hash >>> 8;
      pixels[at + 1] = hash >>> 16;
      pixels[at + 2] = hash >>> 24;
      pixels[at + 3] = 255;
    }
  const lattice = CanvasKit.MakeImage(
    {
      width,
      height,
      colorType: CanvasKit.ColorType.RGBA_8888,
      alphaType: CanvasKit.AlphaType.Opaque,
      colorSpace: CanvasKit.ColorSpace.SRGB,
    },
    pixels,
    width * 4,
  );
  if (!lattice) fail("film_grain_lattice", "film grain noise image allocation failed");
  let source: Shader | null = null,
    noise: Shader | null = null,
    shader: Shader | null = null;
  canvas.save();
  try {
    source = input.image.makeShaderOptions(
      CanvasKit.TileMode.Clamp,
      CanvasKit.TileMode.Clamp,
      CanvasKit.FilterMode.Nearest,
      CanvasKit.MipmapMode.None,
    );
    noise = lattice.makeShaderOptions(
      CanvasKit.TileMode.Clamp,
      CanvasKit.TileMode.Clamp,
      CanvasKit.FilterMode.Nearest,
      CanvasKit.MipmapMode.None,
    );
    if (!source || !noise) fail("film_grain_shader", "film grain child shader allocation failed");
    shader = builtins.shader(
      "filmGrain",
      [
        input.roi.width,
        input.roi.height,
        input.roi.x - output.x,
        input.roi.y - output.y,
        output.x,
        output.y,
        originX,
        originY,
        amount,
        size,
      ],
      [source, noise],
    );
    canvas.translate(output.x, output.y);
    drawShader(CanvasKit, canvas, shader, "src");
  } finally {
    canvas.restore();
    shader?.delete();
    source?.delete();
    noise?.delete();
    lattice.delete();
  }
}

function disposeValues(values: Map<number, ImageValue>, retained: Image | null): void {
  const deleted = new Set<Image>();
  for (const value of values.values())
    if (value.owned && value.image && value.image !== retained && !deleted.has(value.image)) {
      value.image.delete();
      deleted.add(value.image);
    }
  if (retained && !deleted.has(retained)) retained.delete();
  values.clear();
}

function disposePrograms(programs: Map<number, AdmittedProgram>): void {
  // Backend font/shader objects are executor-owned caches. Per-frame maps only borrow them.
  programs.clear();
}

function requiredBuiltinKernels(plan: Wire, programs: Wire[]): Set<BuiltinKernel> {
  const kernels = new Set<BuiltinKernel>();
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const kind = record(record(rawPass, "plan pass").kind, "plan pass kind");
    if (kind.kind === "dispatchKernel") {
      const invocation = record(kind.invocation, "kernel invocation");
      if (invocation.kind === "transition") {
        kernels.add(preparedTransition(record(invocation.kernel, "transition kernel")).kind);
        kernels.add("opacity");
      } else if (invocation.kind === "filter" || invocation.kind === "adjustmentEffect") {
        const effect = record(record(invocation.effect, "effect").kernel, "effect kernel");
        const required = effectBuiltin(String(effect.kind));
        if (required) kernels.add(required);
      }
    } else if (kind.kind === "importRegion") {
      const pipeline = record(kind.sourcePipeline, "source pipeline");
      if (pipeline.chromaKey != null) kernels.add("chromaKey");
    } else if (kind.kind === "compositeRegion") {
      const mode = record(kind.mode, "composite mode");
      if (mode.kind === "blend" && String(mode.mode) !== "normal") kernels.add("creativeBlend");
    } else if (kind.kind === "copyConvert") {
      kernels.add("outputTransform");
    }
  }
  for (const rawProgram of programs) {
    const localPlan = record(record(rawProgram, "binding program").localPlan, "local plan");
    for (const rawPass of array(localPlan.passes, "local plan passes")) {
      const kind = record(record(rawPass, "local pass").kind, "local pass kind");
      if (kind.kind === "blend" && String(kind.mode) !== "normal") kernels.add("creativeBlend");
      if (kind.kind === "applyTransition") {
        const kernel = record(
          record(kind.transition, "local transition").kernel,
          "transition kernel",
        );
        if (kernel.type !== "shader") kernels.add(transitionKernel(String(kernel.kind)));
        kernels.add("transitionInput");
      }
      if (kind.kind === "applyFilter") {
        const filter = record(kind.filter, "program filter").kind;
        if (
          filter === "chromaticAberration" ||
          filter === "velocityBlur" ||
          filter === "filmGrain" ||
          filter === "radialBlur" ||
          filter === "lensDistortion"
        )
          kernels.add(filter);
        if (filter === "noiseDisplacement") kernels.add("noiseDisplacement");
        if (filter === "glow" || filter === "bloom") {
          for (const kernel of [
            "pyramidDownsample",
            "pyramidBlur",
            "pyramidUpsample",
            "pyramidUpsampleRows",
            filter === "glow" ? "pyramidCoverageDownsample" : "pyramidThreshold",
            filter === "glow" ? "pyramidGlowUpsampleComposite" : "pyramidUpsampleComposite",
          ] as BuiltinKernel[])
            kernels.add(kernel);
        }
      }
      if (kind.kind === "applyMask" && String(kind.mode).startsWith("luminance"))
        kernels.add("maskCoverage");
    }
  }
  return kernels;
}

function admitCanvasKitOutput(plan: Wire): void {
  const output = record(record(plan.renderSpec, "render spec").output, "output spec");
  if (output.bitDepth !== "eight") {
    fail(
      "output_storage",
      `CanvasKit product targets accept eight-bit delivery, not '${String(output.bitDepth)}'`,
    );
  }
  const dither = record(output.dither, "output dither");
  if (dither.kind !== "none" && dither.kind !== "triangular") {
    fail("output_dither", "unsupported output dither");
  }
}

async function drawOutputTransform(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  input: Image | null,
  operation: Wire,
  cpuKernel: CanvasKitEngineKernels | null,
): Promise<void> {
  if (operation.kind !== "outputTransform") {
    fail("copy_operation", `copy operation '${String(operation.kind)}' is outside the closed set`);
  }
  const spec = record(operation.spec, "output transform spec");
  const target = record(spec.target, "output color target");
  const luminance = record(spec.luminance, "output luminance");
  const dither = record(spec.dither, "output dither");
  const uniforms = [
    primariesCode(String(target.primaries)),
    transferCode(String(target.transfer)),
    enumCode(String(spec.toneMap), ["none", "reinhardLuminance"], "tone map"),
    enumCode(String(spec.gamutMap), ["clip", "chromaCompress"], "gamut map"),
    enumCode(
      String(spec.alpha),
      ["opaque", "straightCoverage", "premultipliedCoverage"],
      "output alpha",
    ),
    positiveId(luminance.referenceWhite, "output reference white"),
    positiveId(luminance.peak, "output peak luminance"),
    dither.kind === "none" ? 0 : 1,
  ];
  if (
    cpuKernel &&
    dither.kind === "none" &&
    input &&
    target.primaries === "rec709" &&
    target.transfer === "srgb" &&
    spec.toneMap === "none" &&
    spec.gamutMap === "chromaCompress" &&
    luminance.referenceWhite === 100 &&
    luminance.peak === 100
  ) {
    // An author-transparent canvas can still produce an opaque frame (background View/video).
    // Prove actual coverage each time; partial coverage keeps the established shader path.
    if (
      await drawSrgbPreviewCpu(
        CanvasKit,
        canvas,
        input,
        cpuKernel,
        spec.alpha === "opaque",
        spec.alpha !== "opaque",
      )
    )
      return;
  }
  const child = imageOrTransparentShader(CanvasKit, input);
  const noise = builtins.outputNoise(dither.kind === "none" ? null : Number(dither.seed));
  try {
    const shader = builtins.shader("outputTransform", uniforms, [child, noise]);
    try {
      drawShader(CanvasKit, canvas, shader, "src");
    } finally {
      shader.delete();
    }
  } finally {
    child.delete();
    noise.delete();
  }
}

function enumCode(value: string, values: readonly string[], label: string): number {
  const index = values.indexOf(value);
  if (index < 0) fail("wire_enum", `${label} '${value}' is outside the closed set`);
  return index;
}

function transitionValues(value: unknown, kind: string): number[] {
  const values = array(value, "transition parameters");
  if (values.length !== 4)
    fail("transition_params", "transition requires four resolved scalar slots");
  const specs = transitionParameterSpecs[kind as keyof typeof transitionParameterSpecs];
  if (!specs) fail("transition_kernel", `unknown transition kind '${kind}'`);
  const resolved = values.map((value, index) => {
    const scalar = Math.fround(finiteNumber(value, `transition parameter ${index}`));
    const spec = specs[index];
    // Rust resolves author numbers to f32; compare the same representable bounds.
    if (spec ? scalar < Math.fround(spec.min) || scalar > Math.fround(spec.max) : scalar !== 0) {
      fail("transition_params", `invalid ${kind} parameter '${spec?.name ?? index}'`);
    }
    return scalar;
  });
  if (kind === "directionalWarp" && Math.abs(resolved[0]!) + Math.abs(resolved[1]!) < 0.000001)
    fail("transition_params", "transition direction must be nonzero");
  return resolved;
}

function preparedTransition(value: Wire): { kind: BuiltinKernel; params: number[] } {
  if (value.type === "extensionCrossFade") return { kind: "fade", params: [0, 0, 0, 0] };
  if (value.type !== "builtin") fail("transition_kernel", "unknown prepared transition type");
  return {
    kind: transitionKernel(String(value.kind)),
    params: transitionValues(value.params, String(value.kind)),
  };
}

function transitionKernel(value: string): BuiltinKernel {
  if (!TRANSITION_KERNELS.has(value as BuiltinKernel)) {
    fail("transition_kernel", `transition kernel '${value}' is outside the closed set`);
  }
  return value as BuiltinKernel;
}

function effectBuiltin(kind: string): BuiltinKernel | null {
  switch (kind) {
    case "chromaKey":
      return "chromaKey";
    case "colorGrade":
      return "colorGrade";
    case "mosaic":
      return "mosaic";
    case "directionalBlur":
      return "directionalBlur";
    case "spotlight":
      return "spotlight";
    case "gaussianBlur":
      return null;
    default:
      fail("effect_kernel", `effect kernel '${kind}' is outside the closed set`);
  }
}

function drawBuiltinImage(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  kernel: BuiltinKernel,
  uniforms: readonly number[],
  images: readonly (Image | null)[],
  mode: string,
): void {
  const children = images.map((value) => imageOrTransparentShader(CanvasKit, value));
  try {
    const shader = builtins.shader(kernel, uniforms, children);
    try {
      drawShader(CanvasKit, canvas, shader, mode);
    } finally {
      shader.delete();
    }
  } finally {
    for (const child of children) child.delete();
  }
}

function opacityShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  image: Image | null,
  opacity: number,
): Shader {
  const child = imageOrTransparentShader(CanvasKit, image);
  try {
    return builtins.shader("opacity", [opacity], [child]);
  } finally {
    child.delete();
  }
}

function imageOrTransparentShader(CanvasKit: CanvasKit, image: Image | null): Shader {
  return image
    ? imageShader(CanvasKit, image, "linearClamp")
    : CanvasKit.Shader.MakeColor(CanvasKit.Color4f(0, 0, 0, 0), CanvasKit.ColorSpace.SRGB);
}

function imageValueOrTransparentShader(CanvasKit: CanvasKit, value: ImageValue): Shader {
  return value.image
    ? imageValueShader(CanvasKit, value, "linearClamp")
    : CanvasKit.Shader.MakeColor(CanvasKit.Color4f(0, 0, 0, 0), CanvasKit.ColorSpace.SRGB);
}

function drawShader(CanvasKit: CanvasKit, canvas: Canvas, shader: Shader, mode: string): void {
  const paint = new CanvasKit.Paint();
  try {
    paint.setShader(shader);
    paint.setBlendMode(blendMode(CanvasKit, mode));
    canvas.drawPaint(paint);
  } finally {
    paint.delete();
  }
}

function drawCreativeBlend(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  source: Image | null,
  destination: Image | null,
  mode: string,
  space: string,
  opacity: number,
  compositeResult: boolean,
): void {
  if (!source) {
    if (compositeResult) drawImage(CanvasKit, canvas, destination, 1, "src");
    return;
  }
  const children = [
    imageOrTransparentShader(CanvasKit, source),
    imageOrTransparentShader(CanvasKit, destination),
  ];
  try {
    const shader = builtins.shader(
      "creativeBlend",
      [blendCode(mode), opacity, compositeResult ? 1 : 0, blendSpaceCode(space)],
      children,
    );
    try {
      drawShader(CanvasKit, canvas, shader, "src");
    } finally {
      shader.delete();
    }
  } finally {
    for (const child of children) child.delete();
  }
}

function drawCreativeBlendValues(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  canvas: Canvas,
  source: ImageValue,
  destination: ImageValue,
  mode: string,
  space: string,
  opacity: number,
  compositeResult: boolean,
): void {
  if (!source.image) {
    if (compositeResult) drawImageValue(CanvasKit, canvas, destination, 1, "src");
    return;
  }
  const children = [
    imageValueOrTransparentShader(CanvasKit, source),
    imageValueOrTransparentShader(CanvasKit, destination),
  ];
  try {
    const shader = builtins.shader(
      "creativeBlend",
      [blendCode(mode), opacity, compositeResult ? 1 : 0, blendSpaceCode(space)],
      children,
    );
    try {
      drawShader(CanvasKit, canvas, shader, "src");
    } finally {
      shader.delete();
    }
  } finally {
    for (const child of children) child.delete();
  }
}

function blendSpaceCode(space: string): number {
  if (space === "linear") return 1;
  if (space === "srgb") return 0;
  return fail("unsupported_blend_space", `blend space '${space}' is not closed`);
}

function blendCode(mode: string): number {
  const modes: Record<string, number> = {
    normal: 0,
    multiply: 1,
    screen: 2,
    overlay: 3,
    darken: 4,
    lighten: 5,
    colorDodge: 6,
    colorBurn: 7,
    linearBurn: 8,
    hardLight: 9,
    softLight: 10,
    difference: 11,
    exclusion: 12,
    hue: 13,
    saturation: 14,
    color: 15,
    luminosity: 16,
    plus: 17,
  };
  const code = modes[mode];
  if (code === undefined) fail("blend_mode", `blend mode '${mode}' is outside the closed set`);
  return code;
}

function validatePacketTriple(plan: Wire, schedule: Wire, templateHash: string): void {
  if (schedule.templateHash !== templateHash) {
    fail("schedule_hash", "bound schedule does not name the supplied plan");
  }
  if (schedule.capabilityFingerprint !== plan.capabilityFingerprint) {
    fail("schedule_capability", "bound schedule capability fingerprint does not match the plan");
  }
  admitOptimizationReport(plan);
}

function exactPacketBytes(input: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}

/**
 * Admit the optimizer proof at the Web execution boundary.
 *
 * Rust already validates and hashes the normalized final state before packing. Web additionally
 * verifies that every reason-coded physical alias is named exactly once and points at the final
 * resource/pass pair it claims. This keeps a checksum-consistent but structurally forged packet
 * from silently turning optimizer evidence into an unreported execution shortcut.
 */
export function admitOptimizationReport(plan: Wire): void {
  const report = record(plan.optimization, "plan.optimization");
  const inputHash = digestString(report.inputPhysicalHash, "optimization.inputPhysicalHash");
  const outputHash = digestString(report.outputPhysicalHash, "optimization.outputPhysicalHash");
  const resources = array(plan.resources, "plan.resources").map((value, index) =>
    record(value, `plan.resources[${index}]`),
  );
  const passes = array(plan.passes, "plan.passes").map((value, index) =>
    record(value, `plan.passes[${index}]`),
  );
  const rewrites = array(report.rewrites, "optimization.rewrites");
  if (rewrites.length === 0) {
    if (inputHash !== outputHash) {
      fail("optimization_proof", "an identity optimization must preserve its physical hash");
    }
  } else if (inputHash === outputHash) {
    fail("optimization_proof", "a non-empty rewrite set must change the normalized physical state");
  }

  let previousPass = 0;
  const outputs = new Set<number>();
  const rewritePasses = new Set<number>();
  for (const [index, raw] of rewrites.entries()) {
    const rewrite = record(raw, `optimization.rewrites[${index}]`);
    const passId = positiveId(rewrite.pass, `optimization.rewrites[${index}].pass`);
    const outputId = positiveId(rewrite.output, `optimization.rewrites[${index}].output`);
    const sourceId = positiveId(rewrite.source, `optimization.rewrites[${index}].source`);
    if (passId <= previousPass || rewritePasses.has(passId)) {
      fail("optimization_proof", "rewrites must be ordered by unique execution pass");
    }
    previousPass = passId;
    rewritePasses.add(passId);
    if (outputs.has(outputId) || sourceId >= outputId) {
      fail("optimization_proof", "rewrite source/output identity is not canonical");
    }
    outputs.add(outputId);

    const resource = record(resources[outputId - 1], `plan.resources[${outputId - 1}]`);
    const pass = record(passes[passId - 1], `plan.passes[${passId - 1}]`);
    if (
      positiveId(resource.id, "optimization resource id") !== outputId ||
      positiveId(pass.id, "optimization pass id") !== passId
    ) {
      fail("optimization_proof", "rewrite resource/pass ids are not canonical");
    }
    const resourceKind = record(resource.kind, "optimization resource kind");
    const passKind = record(pass.kind, "optimization pass kind");
    const expected =
      rewrite.kind === "backdropResolveReuse"
        ? { resourceReason: "reusedBackdropResolve", passKind: "bindBackdropView" }
        : rewrite.kind === "noOpGroupElimination"
          ? { resourceReason: "noOpGroup", passKind: "aliasResource" }
          : null;
    if (
      !expected ||
      resourceKind.kind !== "alias" ||
      resourceKind.reason !== expected.resourceReason ||
      positiveId(resourceKind.source, "optimization alias source") !== sourceId ||
      passKind.kind !== expected.passKind ||
      passKind.reason !== expected.resourceReason ||
      positiveId(passKind.input, "optimization pass input") !== sourceId ||
      positiveId(passKind.output, "optimization pass output") !== outputId
    ) {
      fail("optimization_proof", `rewrite ${index} does not match the final plan`);
    }
  }

  for (const resource of resources) {
    const kind = record(resource.kind, "plan resource kind");
    if (
      kind.kind === "alias" &&
      (kind.reason === "reusedBackdropResolve" || kind.reason === "noOpGroup") &&
      !outputs.has(positiveId(resource.id, "optimizer alias resource id"))
    ) {
      fail(
        "optimization_proof",
        "an optimizer-owned resource alias is missing from the rewrite proof",
      );
    }
  }
  for (const pass of passes) {
    const kind = record(pass.kind, "plan pass kind");
    const optimizerOwned =
      (kind.kind === "bindBackdropView" && kind.reason === "reusedBackdropResolve") ||
      (kind.kind === "aliasResource" && kind.reason === "noOpGroup");
    if (optimizerOwned && !rewritePasses.has(positiveId(pass.id, "optimizer alias pass id"))) {
      fail(
        "optimization_proof",
        "an optimizer-owned execution alias is missing from the rewrite proof",
      );
    }
  }
}

function digestString(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    fail("optimization_proof", `${label} must be a sha256:<64 lowercase hex> ContentDigest`);
  }
  return value;
}

function dynamicBindings(bindings: Wire): Map<number, Wire> {
  const result = new Map<number, Wire>();
  for (const [index, raw] of array(bindings.dynamic, "bindings.dynamic").entries()) {
    const binding = record(raw, `dynamic[${index}]`);
    const id = positiveId(binding.id, `dynamic[${index}].id`);
    if (id !== index + 1) fail("dynamic_layout", "dynamic ids are not canonical");
    result.set(id, binding);
  }
  return result;
}

function preflightEffectDomains(
  plan: Wire,
  dynamic: Map<number, Wire>,
  extent: { width: number; height: number },
): void {
  for (const rawPass of array(plan.passes, "plan.passes")) {
    const kind = record(record(rawPass, "plan pass").kind, "plan pass kind");
    if (kind.kind !== "dispatchKernel") continue;
    const invocation = record(kind.invocation, "kernel invocation");
    if (invocation.kind !== "filter" && invocation.kind !== "adjustmentEffect") continue;
    const effect = record(invocation.effect, "effect");
    const kernel = record(effect.kernel, "effect kernel");
    const region = effectRegion(kernel);
    if (region) unitRegion(region);
    const space = record(effect.space, "effect space");
    if (space.kind === "root") continue;
    if (space.kind !== "layer")
      fail("effect_space", `effect space '${String(space.kind)}' is outside the closed set`);
    const transform = dynamicTransform(dynamic, space.transform);
    const bounds = dynamicBounds(dynamic, space.bounds);
    if (kernel.kind === "mosaic" && !project3(transform, [0, 0])) {
      fail("effect_space", `effect '${String(effect.semanticPath)}' has an invalid layer origin`);
    }
    if (region && deviceRectIntersectsRoot(bounds, extent) && !inverse3(transform)) {
      fail(
        "effect_space",
        `effect '${String(effect.semanticPath)}' has a non-invertible layer transform`,
      );
    }
  }
}

function deviceRectIntersectsRoot(
  bounds: Wire,
  extent: { width: number; height: number },
): boolean {
  const x = finiteNumber(bounds.x, "effect bounds.x");
  const y = finiteNumber(bounds.y, "effect bounds.y");
  const width = positiveIdOrZero(bounds.width, "effect bounds.width");
  const height = positiveIdOrZero(bounds.height, "effect bounds.height");
  return (
    width > 0 &&
    height > 0 &&
    x < extent.width &&
    y < extent.height &&
    x + width > 0 &&
    y + height > 0
  );
}

function dynamicScalar(dynamic: Map<number, Wire>, id: unknown): number {
  const value = record(
    required(dynamic, positiveId(id, "dynamic scalar id"), "dynamic scalar").value,
    "dynamic scalar value",
  );
  if (value.kind !== "scalar") fail("dynamic_type", "dynamic binding is not scalar");
  return finiteNumber(value.value, "dynamic scalar");
}

function dynamicTransform(dynamic: Map<number, Wire>, id: unknown): number[] {
  const value = record(
    required(dynamic, positiveId(id, "dynamic transform id"), "dynamic transform").value,
    "dynamic transform value",
  );
  if (value.kind !== "deviceTransform")
    fail("dynamic_type", "dynamic binding is not DeviceTransform");
  return numberArray(record(value.value, "DeviceTransform").matrix, 9, "DeviceTransform.matrix");
}

function dynamicBounds(dynamic: Map<number, Wire>, id: unknown): Wire {
  const value = record(
    required(dynamic, positiveId(id, "dynamic bounds id"), "dynamic bounds").value,
    "dynamic bounds value",
  );
  if (value.kind !== "bounds") fail("dynamic_type", "dynamic binding is not Bounds");
  return record(value.value, "DeviceRect");
}

function renderExtent(plan: Wire): { width: number; height: number } {
  const spec = record(plan.renderSpec, "plan.renderSpec");
  return {
    width: positiveId(spec.width, "render width"),
    height: positiveId(spec.height, "render height"),
  };
}

function preflightTarget(surface: Surface, extent: { width: number; height: number }): void {
  const canvas = surface.getCanvas();
  const bounds = canvas.getDeviceClipBounds();
  if (bounds[2] - bounds[0] !== extent.width || bounds[3] - bounds[1] !== extent.height)
    fail("target_extent", "target extent does not match RenderSpec");
}

function externalObject(
  resources: Map<number, CanvasKitExternalObject>,
  id: unknown,
): CanvasKitExternalObject {
  const object = required(resources, positiveId(id, "external resource id"), "external resource");
  if (!object.image || (object.kind !== "visual" && object.kind !== "scene3d"))
    fail("external_type", "external resource is not an image");
  return object;
}

function resourceValue(values: Map<number, ImageValue>, id: unknown): ImageValue {
  return required(values, positiveId(id, "plan resource id"), "plan resource");
}
function image(values: Map<number, ImageValue>, id: unknown): Image | null {
  return resourceValue(values, id).image;
}
function programValue(values: Map<number, ImageValue>, id: unknown): ImageValue {
  return required(values, positiveId(id, "program resource id"), "program resource");
}
function borrowed(image: Image | null, roi: DeviceRoi): ImageValue {
  return { image, owned: false, roi };
}
function borrowedValue(value: ImageValue): ImageValue {
  return { image: value.image, owned: false, roi: value.roi };
}
function transparentValue(): ImageValue {
  return { image: null, owned: false, roi: { x: 0, y: 0, width: 0, height: 0 } };
}
function liveImages(values: Map<number, ImageValue>): number {
  return new Set([...values.values()].map((value) => value.image).filter(Boolean)).size;
}

function drawImageValue(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  value: ImageValue,
  alpha: number,
  mode: string,
): void {
  if (!value.image || value.roi.width === 0 || value.roi.height === 0) return;
  const paint = new CanvasKit.Paint();
  try {
    paint.setAlphaf(alpha);
    drawImageValueWithPaint(CanvasKit, canvas, value, paint, mode);
  } finally {
    paint.delete();
  }
}

function drawImageValueWithPaint(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  value: ImageValue,
  paint: Paint,
  mode: string,
): void {
  if (!value.image || value.roi.width === 0 || value.roi.height === 0) return;
  paint.setBlendMode(blendMode(CanvasKit, mode));
  const source = CanvasKit.XYWHRect(0, 0, value.roi.width, value.roi.height);
  const destination = CanvasKit.XYWHRect(
    value.roi.x,
    value.roi.y,
    value.roi.width,
    value.roi.height,
  );
  canvas.drawImageRectOptions(
    value.image,
    source,
    destination,
    CanvasKit.FilterMode.Nearest,
    CanvasKit.MipmapMode.None,
    paint,
  );
}

/** A value at the root extent, and the release of the copy that may hold it. */
interface MaterializedValue {
  readonly value: ImageValue;
  release(): void;
}

/**
 * `value` at the root extent. A copy keeps its transient checked out until `release`, so no later
 * draw into a pooled surface copies the pixels that this snapshot still shares.
 */
function materializeRootValue(
  CanvasKit: CanvasKit,
  arena: SurfaceArena,
  value: ImageValue,
  extent: { width: number; height: number },
): MaterializedValue {
  const root: DeviceRoi = { x: 0, y: 0, ...extent };
  if (value.image === null) return { value: borrowed(null, root), release: () => {} };
  if (sameRoi(value.roi, root)) return { value: borrowedValue(value), release: () => {} };
  const surface = arena.transient();
  let snapshot: Image;
  try {
    const canvas = surface.getCanvas();
    canvas.clear(CanvasKit.TRANSPARENT);
    drawImageValue(CanvasKit, canvas, value, 1, "src");
    surface.flush();
    snapshot = surface.makeImageSnapshot();
  } catch (error) {
    arena.releaseTransient(surface);
    throw error;
  }
  return {
    value: { image: snapshot, owned: true, roi: root },
    release: () => {
      snapshot.delete();
      arena.releaseTransient(surface);
    },
  };
}

function drawImage(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  image: Image | null,
  alpha: number,
  mode: string,
): void {
  if (!image) return;
  const paint = new CanvasKit.Paint();
  try {
    paint.setAlphaf(alpha);
    paint.setBlendMode(blendMode(CanvasKit, mode));
    canvas.drawImage(image, 0, 0, paint);
  } finally {
    paint.delete();
  }
}

function drawImageWithPaint(
  CanvasKit: CanvasKit,
  canvas: Canvas,
  image: Image | null,
  paint: Paint,
  mode: string,
): void {
  if (!image) return;
  paint.setBlendMode(blendMode(CanvasKit, mode));
  canvas.drawImage(image, 0, 0, paint);
}

function blendMode(CanvasKit: CanvasKit, mode: string): BlendMode {
  const names: Record<string, string> = {
    src: "Src",
    srcOver: "SrcOver",
    srcIn: "SrcIn",
    dstIn: "DstIn",
    dstOut: "DstOut",
    plus: "Plus",
    normal: "SrcOver",
    multiply: "Multiply",
    screen: "Screen",
    overlay: "Overlay",
    darken: "Darken",
    lighten: "Lighten",
    colorDodge: "ColorDodge",
    colorBurn: "ColorBurn",
    hardLight: "HardLight",
    softLight: "SoftLight",
    difference: "Difference",
    exclusion: "Exclusion",
    hue: "Hue",
    saturation: "Saturation",
    color: "Color",
    luminosity: "Luminosity",
    linearBurn: "SrcOver",
    dstInLuminance: "DstIn",
  };
  const name = names[mode];
  if (!name) fail("blend_mode", `blend mode '${mode}' is not closed`);
  return (CanvasKit.BlendMode as unknown as Record<string, BlendMode>)[name]!;
}

function imageShader(CanvasKit: CanvasKit, image: Image, sampling: string, wrap = "pad"): Shader {
  const shader = image.makeShaderOptions(
    tileMode(CanvasKit, wrap),
    tileMode(CanvasKit, wrap),
    sampling === "nearestClamp" ? CanvasKit.FilterMode.Nearest : CanvasKit.FilterMode.Linear,
    CanvasKit.MipmapMode.None,
  );
  if (!shader) fail("image_shader", "image cannot become a shader child");
  return shader;
}

function imageValueShader(CanvasKit: CanvasKit, value: ImageValue, sampling: string): Shader {
  if (!value.image) return imageValueOrTransparentShader(CanvasKit, value);
  const shader = value.image.makeShaderOptions(
    CanvasKit.TileMode.Clamp,
    CanvasKit.TileMode.Clamp,
    sampling === "nearestClamp" ? CanvasKit.FilterMode.Nearest : CanvasKit.FilterMode.Linear,
    CanvasKit.MipmapMode.None,
    [1, 0, value.roi.x, 0, 1, value.roi.y, 0, 0, 1],
  );
  if (!shader) fail("image_shader", "ROI image cannot become a shader child");
  return shader;
}

function decodeBase64Bytes(value: unknown, label: string): Uint8Array {
  if (typeof value !== "string") fail("program_contract", `${label} must be a base64 string`);
  let decoded: string;
  try {
    const direct = (Uint8Array as unknown as { fromBase64?: (text: string) => Uint8Array })
      .fromBase64;
    if (direct) return direct(value);
    decoded = atob(value);
  } catch {
    fail("program_contract", `${label} is not valid base64`);
  }
  const bytes = new Uint8Array(decoded.length);
  for (let index = 0; index < decoded.length; index += 1) bytes[index] = decoded.charCodeAt(index);
  return bytes;
}

function normalizedExternalShader(
  CanvasKit: CanvasKit,
  builtins: CanvasKitBuiltinRuntime,
  object: CanvasKitExternalObject,
  sampling = "linearClamp",
  wrap = "pad",
): Shader {
  if (!object.image) fail("external_type", "normalized external shader needs an image");
  if (object.kind === "dataTexture") {
    // Generated sampling wraps logical coordinates before selecting each numeric plane.
    return imageShader(CanvasKit, object.image, "nearestClamp", "pad");
  }
  // A host frame is already working-space pixels.
  if (object.kind === "frame") return imageShader(CanvasKit, object.image, sampling, wrap);
  let primaries = "rec709";
  let transfer = "srgb";
  let referenceWhite = 100;
  let peak = 100;
  if (object.kind === "visual") {
    const key = record(object.key, "visual resource key");
    const resourceInterpretation = record(key.interpretation, "visual resource interpretation");
    if (resourceInterpretation.kind !== "visual")
      fail("external_type", "visual object key has a non-visual interpretation");
    const interpretation = record(resourceInterpretation.interpretation, "visual interpretation");
    const color = record(interpretation.color, "visual color description");
    const luminance = record(interpretation.luminance, "visual signal luminance");
    primaries = String(color.primaries);
    transfer = String(color.transfer);
    referenceWhite = positiveId(luminance.referenceWhite, "visual reference white");
    peak = positiveId(luminance.peak, "visual peak luminance");
  } else if (object.kind !== "scene3d") {
    fail("external_type", "only visual and Scene3D images enter the working color domain");
  }
  const mipmapped =
    object.kind === "scene3d" && sampling === "linearClamp"
      ? object.image.makeCopyWithDefaultMipmaps()
      : null;
  const child = mipmapped
    ? mipmapped.makeShaderOptions(
        CanvasKit.TileMode.Clamp,
        CanvasKit.TileMode.Clamp,
        CanvasKit.FilterMode.Linear,
        CanvasKit.MipmapMode.Linear,
      )
    : imageShader(CanvasKit, object.image, sampling, wrap);
  try {
    return builtins.shader(
      "normalizeInput",
      [transferCode(transfer), primariesCode(primaries), referenceWhite, peak],
      [child],
    );
  } finally {
    child.delete();
    mipmapped?.delete();
  }
}

function transferCode(value: string): number {
  if (value === "linear") return 0;
  if (value === "srgb") return 1;
  if (value === "rec709") return 2;
  if (value === "pq") return 3;
  if (value === "hlg") return 4;
  fail("color_transfer", `transfer '${value}' is outside the closed set`);
}

function primariesCode(value: string): number {
  if (value === "rec709") return 0;
  if (value === "displayP3") return 1;
  if (value === "rec2020") return 2;
  fail("color_primaries", `primaries '${value}' are outside the closed set`);
}

// Native ARM/x86 F16 stores round to nearest even; CanvasKit CPU stores truncate.
// Put premultiplied solid-paint channels on the same representable lattice first.
function workingF16(value: number): number {
  value = Math.fround(value);
  const magnitude = Math.abs(value);
  if (magnitude === 0) return value;
  const step = 2 ** Math.max(-24, Math.floor(Math.log2(magnitude)) - 10);
  const scaled = magnitude / step;
  const integral = Math.floor(scaled);
  const fraction = scaled - integral;
  const rounded = integral + Number(fraction > 0.5 || (fraction === 0.5 && integral % 2 === 1));
  return Math.sign(value) * rounded * step;
}

function linearColor(CanvasKit: CanvasKit, color: LinearColorWire): Float32Array {
  const alpha = finiteNumber(color.alpha, "color alpha");
  if (alpha <= 0) return CanvasKit.Color4f(0, 0, 0, 0);
  return CanvasKit.Color4f(
    workingF16(color.red) / alpha,
    workingF16(color.green) / alpha,
    workingF16(color.blue) / alpha,
    alpha,
  );
}

// Native's `set_working_color` uses the same shader.
const WORKING_COLOR_SKSL = "uniform float4 color; half4 main(float2 xy) { return half4(color); }";
const workingColorEffects = new WeakMap<CanvasKit, RuntimeEffect>();

/** Paints a premultiplied working color through a float uniform. Ganesh keeps an in-range paint
 * color as an 8-bit vertex color, and the output transform's Rec.2020 to sRGB matrix amplifies
 * that rounding to several levels in saturated or dark channels. `opacity` scales the result. */
function setWorkingColor(
  CanvasKit: CanvasKit,
  paint: Paint,
  red: number,
  green: number,
  blue: number,
  alpha: number,
  opacity = 1,
): void {
  let effect = workingColorEffects.get(CanvasKit);
  if (!effect) {
    effect =
      CanvasKit.RuntimeEffect.Make(WORKING_COLOR_SKSL) ??
      fail("shader_compile", "working color shader failed admission");
    workingColorEffects.set(CanvasKit, effect);
  }
  const shader = effect.makeShader([red, green, blue, alpha]);
  paint.setColor(CanvasKit.WHITE);
  paint.setAlphaf(opacity);
  paint.setShader(shader);
  shader.delete();
}

function setLinearColor(CanvasKit: CanvasKit, paint: Paint, color: LinearColorWire): void {
  const alpha = finiteNumber(color.alpha, "color alpha");
  if (alpha <= 0) setWorkingColor(CanvasKit, paint, 0, 0, 0, 0);
  else
    setWorkingColor(
      CanvasKit,
      paint,
      workingF16(color.red),
      workingF16(color.green),
      workingF16(color.blue),
      alpha,
    );
}

function color4(value: unknown): Float32Array {
  const channels = numberArray(value, 4, "color");
  const alpha = channels[3]!;
  if (alpha <= 0) return Float32Array.from([0, 0, 0, 0]);
  return Float32Array.from([
    channels[0]! / alpha,
    channels[1]! / alpha,
    channels[2]! / alpha,
    alpha,
  ]);
}

function skRect(CanvasKit: CanvasKit, value: RectWire) {
  return CanvasKit.XYWHRect(value.x, value.y, value.width, value.height);
}
function deviceRect(CanvasKit: CanvasKit, value: Wire) {
  return CanvasKit.XYWHRect(
    finiteNumber(value.x, "rect.x"),
    finiteNumber(value.y, "rect.y"),
    positiveIdOrZero(value.width, "rect.width"),
    positiveIdOrZero(value.height, "rect.height"),
  );
}
function rect(value: unknown, label: string): RectWire {
  const wire = record(value, label);
  return {
    x: finiteNumber(wire.x, `${label}.x`),
    y: finiteNumber(wire.y, `${label}.y`),
    width: finiteNumber(wire.width, `${label}.width`),
    height: finiteNumber(wire.height, `${label}.height`),
  };
}
function pair(value: unknown, label: string): [number, number] {
  const values = numberArray(value, 2, label);
  return [values[0]!, values[1]!];
}
function numberArray(value: unknown, length: number, label: string): number[] {
  const values = array(value, label).map((item) => finiteNumber(item, label));
  if (values.length !== length) fail("wire_shape", `${label} must contain ${length} numbers`);
  return values;
}
function finiteNumber(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    fail("wire_number", `${label} must be finite`);
  return value;
}
function positiveId(value: unknown, label: string): number {
  const result = positiveIdOrZero(value, label);
  if (result === 0) fail("wire_id", `${label} must be positive`);
  return result;
}
function positiveIdOrZero(value: unknown, label: string): number {
  const number = typeof value === "bigint" ? Number(value) : value;
  if (!Number.isSafeInteger(number) || Number(number) < 0)
    fail("wire_id", `${label} must be an exact non-negative integer`);
  return Number(number);
}
function exactPositiveInteger(value: unknown, label: string): bigint {
  const number = typeof value === "bigint" ? value : BigInt(positiveId(value, label));
  if (number <= 0n) fail("wire_id", `${label} must be positive`);
  return number;
}
function array(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) fail("wire_shape", `${label} must be an array`);
  return value;
}
function record(value: unknown, label: string): Wire {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    fail("wire_shape", `${label} must be an object`);
  return value as Wire;
}
function passOutput(kind: Wire): number {
  if (kind.kind === "dispatchKernel")
    return positiveId(record(kind.invocation, "kernel invocation").output, "kernel output");
  return positiveId(kind.output, "pass output");
}
function programPassReads(kind: Wire): number[] {
  switch (kind.kind) {
    case "clear":
    case "rasterNode":
    case "rasterTree":
      return [];
    case "readDestination":
      return array(kind.localInputs, "program destination local inputs").map((input) =>
        positiveId(input, "program destination local input"),
      );
    case "sourceOver":
    case "blend":
      return [
        positiveId(kind.source, "program source"),
        positiveId(kind.destination, "program destination"),
      ];
    case "applyMask":
      return [positiveId(kind.input, "program input"), positiveId(kind.mask, "program mask")];
    case "applyTransition":
      return [
        positiveId(kind.from, "program transition from"),
        positiveId(kind.to, "program transition to"),
      ];
    case "backdrop":
    case "applyClip":
    case "applyFilter":
    case "applyOpacity":
    case "applyTransform":
      return [positiveId(kind.input, "program input")];
    case "applyShader":
      return [
        positiveId(kind.input, "program input"),
        ...(kind.backdrops === undefined ? [] : array(kind.backdrops, "shader backdrops")).map(
          (backdrop) => positiveId(backdrop, "program shader backdrop"),
        ),
      ];
    default:
      return fail("program_schedule", `unknown program pass kind '${String(kind.kind)}'`);
  }
}
function required<K, V>(map: ReadonlyMap<K, V>, key: K, label: string): V {
  const value = map.get(key);
  if (value === undefined) fail("missing_value", `${label} '${String(key)}' is absent`);
  return value;
}
function requiredIndex<T>(values: T[], id: unknown, label: string): T {
  const index = positiveIdOrZero(id, `${label} id`);
  const value = values[index];
  if (value === undefined) fail("missing_value", `${label} ${index} is absent`);
  return value;
}
function fontKey(hash: string, index: number): string {
  return `${hash}:${index}`;
}
/** @internal Shared by executor admission and its source-level contract tests. */
export function admitProgramTextures(
  requirementsValue: unknown,
  bindingsValue: unknown,
  objects: ReadonlyMap<number, CanvasKitExternalObject>,
): Map<string, CanvasKitExternalObject> {
  const requiredTextures = array(requirementsValue, "program texture requirements");
  const textureBindings = array(bindingsValue, "program textures");
  if (textureBindings.length !== requiredTextures.length) {
    fail("texture_contract", "program texture bindings do not match requirements");
  }
  const textures = new Map<string, CanvasKitExternalObject>();
  for (let index = 0; index < textureBindings.length; index += 1) {
    const binding = record(textureBindings[index], "texture binding");
    const requirement = record(requiredTextures[index], "texture requirement");
    if (String(binding.key) !== String(requirement.key)) {
      fail("texture_contract", "program texture binding order/key is not canonical");
    }
    const object = required(objects, positiveId(binding.slot, "texture slot"), "texture slot");
    const expectedKind = requirement.colorDomain === "data" ? "dataTexture" : "visual";
    if (object.kind !== expectedKind || !object.image) {
      fail("texture_contract", `texture '${String(binding.key)}' has the wrong interpretation`);
    }
    const identity = textureIdentity(requirement);
    if (textures.has(identity)) {
      fail("texture_contract", `duplicate program texture '${String(binding.key)}'`);
    }
    textures.set(identity, object);
  }
  return textures;
}

/** @internal Resolves the same full identity used by image and shader execution. */
export function resolveProgramTexture(
  textures: ReadonlyMap<string, CanvasKitExternalObject>,
  textureValue: unknown,
  label = "program texture",
): CanvasKitExternalObject {
  return required(textures, textureIdentity(record(textureValue, `${label} requirement`)), label);
}

function textureIdentity(texture: Wire): string {
  const kind = String(texture.kind);
  const sample = texture.sampleTimeMicros;
  if (
    sample !== null &&
    sample !== undefined &&
    (typeof sample !== "number" || !Number.isSafeInteger(sample) || sample < 0)
  ) {
    fail(
      "texture_contract",
      "texture sampleTimeMicros must be a non-negative safe integer or null",
    );
  }
  if (kind === "video") {
    if (sample === null || sample === undefined) {
      fail("texture_contract", "video textures must carry sampleTimeMicros");
    }
  } else if (kind === "image" || kind === "generated" || kind === "frame") {
    if (sample !== null && sample !== undefined) {
      fail("texture_contract", "only video textures may carry sampleTimeMicros");
    }
  } else {
    fail("texture_contract", `texture kind '${kind}' is outside the closed set`);
  }
  return `${String(texture.key)}\u0000${String(texture.colorDomain)}\u0000${sample === null || sample === undefined ? "static" : String(sample)}`;
}
function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (typeof left !== typeof right) return false;
  if (Array.isArray(left) || Array.isArray(right)) {
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => deepEqual(value, right[index]))
    );
  }
  if (typeof left !== "object" || left === null || right === null) return false;
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord);
  const rightKeys = Object.keys(rightRecord);
  return (
    leftKeys.length === rightKeys.length &&
    leftKeys.every(
      (key) => Object.hasOwn(rightRecord, key) && deepEqual(leftRecord[key], rightRecord[key]),
    )
  );
}
function bigintJson(_key: string, value: unknown): unknown {
  return typeof value === "bigint" ? value.toString() : value;
}

function mul3(left: number[], right: number[]): number[] {
  const output = Array<number>(9).fill(0);
  for (let row = 0; row < 3; row += 1)
    for (let column = 0; column < 3; column += 1)
      for (let index = 0; index < 3; index += 1)
        output[row * 3 + column]! += left[row * 3 + index]! * right[index * 3 + column]!;
  return output;
}

function inverse3(value: number[]): number[] | null {
  const [a, b, c, d, e, f, g, h, i] = value as [
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
    number,
  ];
  const cofactors = [
    e * i - f * h,
    c * h - b * i,
    b * f - c * e,
    f * g - d * i,
    a * i - c * g,
    c * d - a * f,
    d * h - e * g,
    b * g - a * h,
    a * e - b * d,
  ];
  const determinant = a * cofactors[0]! + b * cofactors[3]! + c * cofactors[6]!;
  return Number.isFinite(determinant) && Math.abs(determinant) > 1e-15
    ? cofactors.map((item) => item / determinant)
    : null;
}

function project3(matrix: number[], point: [number, number]): [number, number] | null {
  const w = matrix[6]! * point[0] + matrix[7]! * point[1] + matrix[8]!;
  if (!Number.isFinite(w) || Math.abs(w) <= 1e-12) return null;
  const result: [number, number] = [
    (matrix[0]! * point[0] + matrix[1]! * point[1] + matrix[2]!) / w,
    (matrix[3]! * point[0] + matrix[4]! * point[1] + matrix[5]!) / w,
  ];
  return result.every(Number.isFinite) ? result : null;
}

function maxBigInt(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}

function fail(code: string, message: string): never {
  throw new CanvasKitCompositorError(code, message);
}
